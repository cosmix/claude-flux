//! Ordered lifecycle around a stage merge, and the only door to post-merge
//! cleanup.
//!
//! Loom's costliest recurring defect is the *phantom merge*: recording a stage
//! as merged, or destroying its branch, without proving in git that its commits
//! landed. `auto_merge::attempt_auto_merge` used to run cleanup inside its own
//! success arms, deleting the worktree and branch *before* its caller could
//! verify ancestry — and the caller derives a missing `completed_commit` from
//! exactly that branch. This module makes the ordering structural:
//!
//! ```text
//! overlay reconcile -> merge attempt -> verify merged ancestry
//!   -> base reconcile & publish -> mark state and release dependents
//!   -> cleanup worktree/overlay
//! ```
//!
//! The merge attempt, the ancestry verification and the state marking stay with
//! the callers, which hold the orchestrator handle and the stage-file lock.
//! Everything around them lives here, and [`MergeLifecycle::cleanup`] is the
//! single path by which a merge caller may reach `cleanup_after_merge`. It is
//! also where a failed or refused cleanup is written onto the stage record
//! for `loom status`.

use std::path::Path;

use crate::context::graph_store::GraphStore;
use crate::context::refresh::{mark_semantic_stale, reconcile_source_graph, SourceGraphScope};
use crate::context::store::ContextStore;
use crate::fs::work_dir::WorkDir;
use crate::git::cleanup::{cleanup_after_merge, needs_cleanup, CleanupConfig, CleanupResult};

mod containment;

/// What the cleanup step did.
///
/// Deliberately not a `Result`: a cleanup failure must never turn a successful
/// merge into an error. Cleanup errors used to propagate through `?`, which made
/// the caller mark a perfectly good merge as `MergeBlocked`.
#[derive(Debug)]
pub enum CleanupOutcome {
    /// No worktree and no branch left to remove.
    NothingToDo,
    /// Deferred: the caller is running inside the worktree it would delete.
    Deferred,
    /// Refused: the stage branch still holds commits that are not in the
    /// target branch, and ancestry could not be verified.
    Refused { reason: String },
    /// Cleanup ran.
    Done(CleanupResult),
    /// Cleanup was attempted and failed. NOT fatal.
    Failed(String),
}

/// The steps that bracket a stage merge, bound to one stage.
#[derive(Debug, Clone, Copy)]
pub struct MergeLifecycle<'a> {
    stage_id: &'a str,
    repo_root: &'a Path,
    work_dir: &'a Path,
}

impl<'a> MergeLifecycle<'a> {
    pub fn new(stage_id: &'a str, repo_root: &'a Path, work_dir: &'a Path) -> Self {
        Self {
            stage_id,
            repo_root,
            work_dir,
        }
    }

    /// STEP 1 — refresh the stage's source-graph overlay BEFORE the merge.
    ///
    /// Never fails the merge: every error path degrades the semantic layer
    /// instead of returning.
    pub fn reconcile_overlay(&self) {
        let Some(plan) = self.plan_id() else { return };
        let worktree = self.repo_root.join(".worktrees").join(self.stage_id);
        if !worktree.exists() {
            tracing::debug!(stage = %self.stage_id, "No worktree; skipping overlay reconcile");
            return;
        }

        let stage = self.stage_id.to_string();
        let scope = SourceGraphScope::Overlay { plan, stage };
        self.reconcile(&worktree, scope, "overlay");
    }

    /// STEP 4 — rebuild and publish the base source-graph layer for the merged
    /// revision.
    ///
    /// Never fails the merge: every error path degrades the semantic layer
    /// instead of returning.
    pub fn reconcile_base(&self, target_branch: &str) {
        match self.base_revision(target_branch) {
            Ok(revision) => {
                self.reconcile(self.repo_root, SourceGraphScope::Base { revision }, "base")
            }
            Err(error) => self.degrade(&format!(
                "could not resolve the merged revision of '{target_branch}': {error}"
            )),
        }
    }

    /// STEP 6 — LAST. Discard the stage overlay, then remove worktree + branch.
    ///
    /// Refuses unless the stage's work is provably contained in
    /// `target_branch`: deleting a branch that still holds unmerged commits
    /// destroys work.
    pub fn cleanup(&self, target_branch: &str, config: &CleanupConfig) -> CleanupOutcome {
        let cwd = std::env::current_dir().ok();
        self.cleanup_with_cwd(cwd.as_deref(), target_branch, config)
    }

    /// [`MergeLifecycle::cleanup`] with the working directory injected, so the
    /// deferral branch is reachable from a test without `set_current_dir` — a
    /// process-global side effect that breaks tests running in parallel.
    ///
    /// `None` means the cwd could not be determined, which defers: cleanup must
    /// not remove a directory it cannot prove it is standing outside of.
    fn cleanup_with_cwd(
        &self,
        cwd: Option<&Path>,
        target_branch: &str,
        config: &CleanupConfig,
    ) -> CleanupOutcome {
        if !needs_cleanup(self.stage_id, self.repo_root) {
            // Nothing left on disk, so any old warning is stale.
            self.record_cleanup_warning(None);
            return CleanupOutcome::NothingToDo;
        }

        let defer = match cwd {
            Some(cwd) => should_defer_cleanup(cwd, self.repo_root, self.stage_id),
            None => true,
        };
        if defer {
            tracing::debug!(stage = %self.stage_id, "Deferring cleanup: inside the worktree");
            return CleanupOutcome::Deferred;
        }

        if let Some(reason) = containment::containment_refusal(self, target_branch) {
            tracing::error!(stage = %self.stage_id, %reason, "Refusing post-merge cleanup");
            self.record_cleanup_warning(Some(format!("refused: {reason}")));
            return CleanupOutcome::Refused { reason };
        }

        self.finish_cleanup(cleanup_after_merge(self.stage_id, self.repo_root, config))
    }

    /// Translate the result of `cleanup_after_merge` into a [`CleanupOutcome`]
    /// and record it on the stage. Split out of `cleanup_with_cwd` to keep
    /// that function within the file's line-length limit.
    fn finish_cleanup(&self, result: anyhow::Result<CleanupResult>) -> CleanupOutcome {
        match result {
            Ok(result) => {
                // Only now: a merged overlay describes a revision nobody should
                // read, but a FAILED cleanup leaves the worktree and branch in
                // place, and the overlay still describes them.
                self.discard_overlay();
                self.record_cleanup_warning(None);
                CleanupOutcome::Done(result)
            }
            Err(failure) => {
                // A cleanup failure must not turn a successful merge into an
                // error: propagating it with `?` used to make the caller mark
                // the stage MergeBlocked even though the merge had succeeded.
                let error = format!("{failure:#}");
                tracing::warn!(stage = %self.stage_id, %error, "Post-merge cleanup failed");
                self.record_cleanup_warning(Some(format!("failed: {error}")));
                CleanupOutcome::Failed(error)
            }
        }
    }

    /// Record (or clear) the cleanup warning on the stage record. Best effort:
    /// a stage with no record (orphan cleanup) has nothing to annotate, and a
    /// failed write must not change the cleanup outcome.
    fn record_cleanup_warning(&self, warning: Option<String>) {
        let stage = match crate::verify::transitions::load_stage(self.stage_id, self.work_dir) {
            Ok(stage) => stage,
            Err(_) => return,
        };
        if stage.cleanup_warning == warning {
            return;
        }
        if let Err(error) =
            crate::verify::transitions::update_stage(self.stage_id, self.work_dir, |s| {
                s.cleanup_warning = warning.clone();
                Ok(())
            })
        {
            tracing::debug!(stage = %self.stage_id, %error, "Could not record cleanup warning");
        }
    }

    /// Drop the stage's source-graph overlay. Best effort by design.
    fn discard_overlay(&self) {
        let Some(plan) = self.plan_id() else { return };
        match self.stores() {
            Ok((_, graph_store)) => {
                if let Err(error) = graph_store.discard_overlay(&plan, self.stage_id) {
                    tracing::warn!(stage = %self.stage_id, %error, "Overlay discard failed");
                }
            }
            Err(error) => {
                tracing::warn!(stage = %self.stage_id, %error, "No graph store to discard from")
            }
        }
    }

    /// Run one reconcile, degrading the semantic layer if it fails.
    fn reconcile(&self, project_root: &Path, scope: SourceGraphScope, layer: &str) {
        let (store, graph_store) = match self.stores() {
            Ok(stores) => stores,
            Err(error) => {
                self.degrade(&format!("no context store for the {layer} layer: {error}"));
                return;
            }
        };

        match reconcile_source_graph(&store, &graph_store, project_root, scope) {
            Ok(outcome) => tracing::debug!(
                stage = %self.stage_id,
                layer,
                files_enumerated = outcome.counters.files_enumerated,
                files_hashed = outcome.counters.files_hashed,
                files_parsed = outcome.counters.files_parsed,
                files_reused = outcome.counters.files_reused,
                files_deleted = outcome.counters.files_deleted,
                files_untracked = outcome.counters.files_untracked,
                bytes_serialized = outcome.counters.bytes_serialized,
                enumerate_ms = outcome.counters.enumerate_ms,
                hash_ms = outcome.counters.hash_ms,
                parse_ms = outcome.counters.parse_ms,
                persist_ms = outcome.counters.persist_ms,
                nodes = %outcome.nodes,
                edges = %outcome.edges,
                stale = outcome.freshness.stale,
                "Reconciled the source graph"
            ),
            Err(error) => self.degrade(&format!("{layer} source-graph reconcile failed: {error}")),
        }
    }

    /// Record that the semantic layer can no longer be trusted, and warn.
    ///
    /// A source-graph rebuild failing is not a reason to reject correct code, so
    /// every path here returns normally with the merge intact.
    fn degrade(&self, reason: &str) {
        tracing::warn!(stage = %self.stage_id, reason, "Marking the semantic layer stale");
        let marked = self
            .context_store()
            .and_then(|store| mark_semantic_stale(&store, reason));
        if let Err(error) = marked {
            tracing::warn!(stage = %self.stage_id, %error, "Could not mark semantic stale");
        }
    }

    /// The revision a published base layer is keyed by: the git SHA of the
    /// target branch HEAD, i.e. the merged revision the layer describes.
    ///
    /// Isolated so swapping in a content fingerprint is a one-line change.
    fn base_revision(&self, target_branch: &str) -> anyhow::Result<String> {
        crate::git::get_branch_head(target_branch, self.repo_root)
    }

    /// Open the context cache. Resolved under the canonical MAIN project root,
    /// so every worktree shares one cache instead of growing a stale private
    /// copy.
    fn context_store(&self) -> anyhow::Result<ContextStore> {
        ContextStore::open(&WorkDir::new(self.repo_root)?)
    }

    fn stores(&self) -> anyhow::Result<(ContextStore, GraphStore)> {
        let store = self.context_store()?;
        let graph_store = GraphStore::new(store.root(), self.work_dir);
        Ok((store, graph_store))
    }

    /// The active plan id. Absent config or absent plan id is not an error: it
    /// only means this stage has no source-graph overlay to keep.
    fn plan_id(&self) -> Option<String> {
        match crate::fs::load_config(self.work_dir) {
            Ok(config) => config.and_then(|config| config.plan_id().map(str::to_string)),
            Err(error) => {
                tracing::debug!(stage = %self.stage_id, %error, "Unreadable loom config");
                None
            }
        }
    }
}

/// The post-merge tail: base reconcile, then cleanup. Call ONLY after the merge
/// has been verified and the stage marked merged.
pub fn finish_verified_merge(
    stage_id: &str,
    repo_root: &Path,
    work_dir: &Path,
    target_branch: &str,
    config: &CleanupConfig,
) -> CleanupOutcome {
    let lifecycle = MergeLifecycle::new(stage_id, repo_root, work_dir);
    lifecycle.reconcile_base(target_branch);
    lifecycle.cleanup(target_branch, config)
}

/// Whether worktree cleanup for `stage_id` must be deferred rather than run now.
///
/// Removing `repo_root/.worktrees/<stage_id>` while `cwd` is inside it deletes
/// the current process's (and its parent Claude session's) live working
/// directory, which breaks any hooks the session fires afterward — they spawn a
/// shell with a cwd that no longer exists. When that's the case, the caller must
/// skip immediate cleanup and leave it for the orchestrator (which cleans up
/// after killing the session).
pub fn should_defer_cleanup(cwd: &Path, repo_root: &Path, stage_id: &str) -> bool {
    let expected = repo_root.join(".worktrees").join(stage_id);
    let expected = match expected.canonicalize() {
        Ok(p) => p,
        // Worktree doesn't exist on disk - cleanup would be a no-op anyway.
        Err(_) => return false,
    };
    match cwd.canonicalize() {
        Ok(cwd) => cwd.starts_with(&expected),
        // Can't verify cwd is safe - assume the worst and defer.
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_cleanup_warning;
