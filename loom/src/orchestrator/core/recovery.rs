//! Error recovery and state synchronization

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::models::session::Session;
use crate::models::stage::{Stage, StageStatus};
use crate::orchestrator::retry::{calculate_backoff, is_backoff_elapsed, should_auto_retry};
use crate::orchestrator::session_registry::orphan_evidence;
use crate::parser::frontmatter::parse_from_markdown;
use crate::verify::transitions::update_stage_at_path;

use super::orphan_adoption::{register_live_current_session, session_is_current_for_stage};
use super::persistence::Persistence;
use super::{clear_status_line, Orchestrator};

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct StageScanCounter {
    pub(super) directory_reads: usize,
    pub(super) entries_visited: usize,
}

/// Enumerate stage paths exactly once. Callers load the already-known path
/// directly instead of resolving every ID through another directory scan.
pub(super) fn scan_stage_paths(
    stages_dir: &Path,
    counter: &mut StageScanCounter,
) -> Result<Vec<PathBuf>> {
    counter.directory_reads += 1;
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(stages_dir)? {
        counter.entries_visited += 1;
        let path = entry?.path();
        if path.extension().and_then(|s| s.to_str()) == Some("md") {
            paths.push(path);
        }
    }
    Ok(paths)
}

pub(super) fn load_stage_at_path(path: &Path) -> Result<Stage> {
    let content = crate::fs::locking::locked_read(path)?;
    crate::verify::transitions::parse_stage_from_markdown(&content)
        .with_context(|| format!("Failed to parse stage from: {}", path.display()))
}

fn persist_recovery_completed_commit(
    stage_id: &str,
    stage_path: &Path,
    work_dir: &Path,
    commit: String,
) -> Result<Stage> {
    update_stage_at_path(stage_id, stage_path, work_dir, |stage| {
        if stage.completed_commit.is_none() {
            stage.completed_commit = Some(commit);
        }
        Ok(())
    })
}

fn recover_orphaned_stage(
    stage: &mut Stage,
    route_to_handoff: bool,
    commits_ahead: usize,
    target_branch: &str,
) {
    if route_to_handoff {
        if let Err(error) = stage.try_mark_needs_handoff() {
            stage.force_status_with_reason(
                StageStatus::NeedsHandoff,
                &format!("orphan recovery (route=handoff): {error}"),
            );
        }
    } else {
        if stage.status == StageStatus::Executing {
            let _ = stage.try_mark_blocked();
        }
        if let Err(error) = stage.try_mark_queued() {
            stage.force_status_with_reason(
                StageStatus::Queued,
                &format!("orphan recovery (route=requeue): {error}"),
            );
        }
    }
    stage.session = None;
    stage.close_reason = Some(if route_to_handoff {
        format!(
            "Session orphaned; branch has {commits_ahead} commit(s) ahead of {target_branch} \
             — needs handoff (use `loom check {}` to diagnose or `loom stage retry \
             --kill-session {}` to retry)",
            stage.id, stage.id
        )
    } else {
        "Session crashed/orphaned".to_string()
    });
    stage.updated_at = chrono::Utc::now();
}

/// Trait for recovery operations
pub(super) trait Recovery: Persistence {
    /// Reconcile any active main-repo merge with stage state on disk and
    /// update the in-memory graph if a stage was mutated.
    ///
    /// MUST run BEFORE `sync_graph_with_stage_files` AND BEFORE
    /// `recover_orphaned_sessions`:
    /// - Recovery deletes orphaned merge sessions; attribution depends on
    ///   their metadata.
    /// - Sync reads stage files into the graph; if reconcile flips
    ///   `Completed + merged=true` -> `MergeConflict + merged=false` AFTER
    ///   sync, the graph still has the stale view and would queue dependents
    ///   based on a phantom merge.
    fn reconcile_and_update_graph(&mut self) -> Result<()>;

    /// Sync the execution graph with existing stage file statuses.
    /// This syncs FROM files TO graph.
    fn sync_graph_with_stage_files(&mut self) -> Result<()>;

    /// Sync queued status from graph back to stage files.
    /// This ensures files reflect when dependencies are satisfied.
    /// This syncs FROM graph TO files.
    fn sync_queued_status_to_files(&mut self) -> Result<()>;

    /// Re-adopt live agents that have no session record at all, rebuilding
    /// the record and relinking the stage to it. The mirror image of
    /// [`Self::recover_orphaned_sessions`], which iterates session FILES and
    /// so cannot see an agent that never got one; runs FIRST from inside
    /// that method for the same reason. Never fails the recovery pass —
    /// per-agent failures are logged and skipped.
    fn adopt_orphaned_agents(&mut self) -> usize;

    /// Recover orphaned sessions (process died but session/stage files exist).
    fn recover_orphaned_sessions(&mut self) -> Result<usize>;

    /// Check if all stages are in a terminal state (for watch mode exit)
    fn all_stages_terminal(&self) -> bool;

    /// Print a status update showing current stage counts
    fn print_status_update(&self);
}

/// Check whether a Blocked stage is *retryable at all* — i.e. it has a
/// retryable failure type and has not exhausted its retry budget — WITHOUT
/// considering backoff timing.
///
/// This is the shared predicate behind both the requeue decision and the
/// watch-mode exit check. `all_stages_terminal()` uses it so the daemon does
/// NOT shut down while a crashed stage is still *pending backoff* (the backoff
/// has not elapsed yet, so `check_retry_eligibility` is currently false, but
/// the retry WILL fire on a later tick). See O-1.
///
/// # Arguments
/// * `stage` - The stage to check
///
/// # Returns
/// `true` if the stage will be auto-retried now or after its backoff elapses.
fn is_retry_pending(stage: &Stage) -> bool {
    let Some(ref info) = stage.failure_info else {
        return false;
    };
    let max = stage.max_retries.unwrap_or(3);
    should_auto_retry(&info.failure_type, stage.retry_count, max)
}

/// Check if a blocked stage is eligible for automatic retry *right now*.
///
/// A stage is eligible for retry if:
/// - It is retryable at all (`is_retry_pending`: retryable failure_type and
///   retry_count < max_retries (default 3))
/// - Sufficient time has elapsed since the last failure (exponential backoff)
///
/// # Arguments
/// * `stage` - The stage to check
///
/// # Returns
/// `true` if the stage should be automatically retried now, `false` otherwise
fn check_retry_eligibility(stage: &Stage) -> bool {
    if !is_retry_pending(stage) {
        return false;
    }

    // Calculate backoff: base 30s, max 300s (5 minutes)
    let backoff = calculate_backoff(stage.retry_count, 30, 300);
    is_backoff_elapsed(stage.last_failure_at, backoff)
}

/// Decide, from a freshly loaded stage FILE, whether it should keep the
/// watch-mode daemon alive. Pure and exhaustive over `StageStatus` so a new
/// variant fails to compile here instead of silently defaulting either way.
///
/// - `Completed`/`Skipped`/`MergeConflict`/`CompletedWithFailures`/
///   `MergeBlocked`/`NeedsHumanReview` are always terminal.
/// - `Blocked` is terminal unless a crash auto-retry is still pending
///   (retryable failure type + retry budget remaining), regardless of
///   whether the backoff has elapsed — the daemon must not shut down before
///   that retry fires (O-1).
/// - `Queued`/`WaitingForDeps` are terminal only when the stage is held;
///   otherwise the daemon still needs to spawn or wait on them.
/// - `NeedsAdjudication`/`Executing`/`WaitingForInput`/`NeedsHandoff` are
///   never terminal: each still needs the daemon alive to spawn, watch, or
///   close out a session (an adjudicator judge, in `NeedsAdjudication`'s
///   case).
fn stage_file_is_terminal(stage: &Stage) -> bool {
    match stage.status {
        StageStatus::Completed
        | StageStatus::Skipped
        | StageStatus::MergeConflict
        | StageStatus::CompletedWithFailures
        | StageStatus::MergeBlocked
        | StageStatus::NeedsHumanReview => true,
        StageStatus::Blocked => !is_retry_pending(stage),
        StageStatus::Queued | StageStatus::WaitingForDeps => stage.held,
        StageStatus::NeedsAdjudication
        | StageStatus::Executing
        | StageStatus::WaitingForInput
        | StageStatus::NeedsHandoff => false,
    }
}

impl Orchestrator {
    /// Re-verify a `Completed + merged=true` non-knowledge stage at sync time.
    ///
    /// Derives `completed_commit` from `loom/<id>` HEAD when missing, then
    /// checks ancestry against the (pre-resolved) target branch. If the
    /// ancestry check fails OR the branch is also missing, reverts
    /// `merged=false` so dependents don't treat the stage as satisfied.
    ///
    /// `target` is passed in so the caller resolves it once per sync pass
    /// rather than spawning `git symbolic-ref` per stage (P-3).
    ///
    /// # Returns
    /// `true` if the stage is verified `merged=true` (ancestry holds) — the
    /// caller memoizes this so the verification is not repeated every tick.
    /// `false` if the merge was reverted or could not be verified.
    pub(super) fn verify_merged_true_or_revert(
        &mut self,
        stage: &mut Stage,
        stage_path: &Path,
        target: &str,
    ) -> bool {
        let original_commit = stage.completed_commit.clone();
        let branch_name = crate::git::branch::branch_name_for_stage(&stage.id);
        let commit = original_commit
            .clone()
            .or_else(|| crate::git::get_branch_head(&branch_name, &self.config.repo_root).ok());
        let verified = commit.as_ref().is_some_and(|commit| {
            crate::git::merge::verify_merge_succeeded(commit, target, &self.config.repo_root)
                .unwrap_or(false)
        });

        if !verified {
            tracing::error!(
                stage_id = %stage.id,
                commit = ?commit,
                target = %target,
                "Phantom merge detected at sync; reverting merged=false when the \
                 canonical commit still matches this verification probe"
            );
        }

        let stage_id = stage.id.clone();
        let mut applicable = false;
        let result =
            update_stage_at_path(&stage_id, stage_path, &self.config.work_dir, |current| {
                let commit_matches = match (&original_commit, &current.completed_commit) {
                    (Some(probed), Some(fresh)) => probed == fresh,
                    (None, None) => {
                        current.completed_commit.clone_from(&commit);
                        true
                    }
                    (None, Some(fresh)) => commit.as_ref() == Some(fresh),
                    (Some(_), None) => false,
                };
                applicable = commit_matches;
                if commit_matches && !verified {
                    current.merged = false;
                }
                Ok(())
            });
        match result {
            Ok(updated) => {
                *stage = updated;
                applicable && verified && stage.merged
            }
            Err(error) => {
                tracing::warn!(stage_id = %stage_id, %error, "Failed to persist merge verification");
                false
            }
        }
    }

    /// Mark `stage_id` as executing in the graph, skipping the no-op warn
    /// when the node is already `Executing` (the common case on every tick
    /// after the first). `mark_executing` only accepts a Queued -> Executing
    /// transition, so this must not be called unconditionally.
    fn sync_executing_node(&mut self, stage_id: &str) {
        let already_executing = self
            .graph
            .get_node(stage_id)
            .is_some_and(|n| n.status == StageStatus::Executing);
        if !already_executing {
            if let Err(e) = self.graph.mark_executing(stage_id) {
                tracing::warn!("Failed to sync graph status for stage {}: {}", stage_id, e);
            }
        }
    }
}

impl Recovery for Orchestrator {
    fn reconcile_and_update_graph(&mut self) -> Result<()> {
        use crate::orchestrator::merge_attribution::{
            reconcile_main_repo_active_merge, ReconciliationOutcome,
        };
        match reconcile_main_repo_active_merge(&self.config.repo_root, &self.config.work_dir)? {
            ReconciliationOutcome::NoActiveMerge
            | ReconciliationOutcome::UnattributedLogged
            | ReconciliationOutcome::AttributedNoOp { .. } => {}
            ReconciliationOutcome::StageMutated { stage_id, .. } => {
                // Disk was corrected; update the graph immediately so any
                // caller in this iteration sees the corrected state. The
                // next sync_graph_with_stage_files call will pick this up
                // again, which is harmless (idempotent).
                // P-3: a phantom-merge revert changes git ancestry reality, so
                // invalidate the verified-merged memo for this stage; the next
                // sync re-verifies it instead of trusting the cached result.
                self.verified_merged.remove(&stage_id);
                self.graph.set_node_merged(&stage_id, false);
                if let Err(e) = self
                    .graph
                    .mark_status(&stage_id, StageStatus::MergeConflict)
                {
                    tracing::warn!(
                        stage_id = %stage_id,
                        error = %e,
                        "Failed to update graph after phantom-merge revert; \
                         next sync will reconcile."
                    );
                }
            }
        }
        Ok(())
    }

    fn sync_graph_with_stage_files(&mut self) -> Result<()> {
        let stages_dir = self.config.work_dir.join("stages");
        if !stages_dir.exists() {
            return Ok(());
        }

        // Collect stage IDs that may need a one-shot auto-merge retry (Fix 11).
        // We iterate twice: first to sync state, then to retry stuck stages.
        // The two-phase approach avoids borrow-checker issues with calling
        // `self.try_auto_merge` while holding a loaded stage.
        let mut stuck_completed_stage_ids: Vec<String> = Vec::new();

        // Resolve the target branch ONCE per pass instead of spawning
        // `git symbolic-ref` per Completed stage every tick (P-3).
        let target_branch = crate::git::branch::resolve_target_branch(
            &self.config.base_branch,
            &self.config.repo_root,
        );

        // Read all stage files and sync their status to the graph. Each file is
        // loaded from the path this single enumeration already produced.
        let mut scan = StageScanCounter::default();
        let stage_paths = scan_stage_paths(&stages_dir, &mut scan)?;
        tracing::debug!(
            directory_reads = scan.directory_reads,
            entries_visited = scan.entries_visited,
            "Indexed stage files for recovery sync"
        );
        for path in stage_paths {
            // Extract stage ID from filename (handles prefixed format like
            // 01-stage-id.md) via the canonical helper. A hand-rolled parser
            // here previously ate the leading digits of digit-leading IDs like
            // `2fa-login`, so the stage never synced (A-2 / O-10).
            let filename = match path.file_name().and_then(|s| s.to_str()) {
                Some(name) => name,
                None => continue,
            };
            let stage_id = match crate::fs::stage_files::extract_stage_id(filename) {
                Some(id) if !id.is_empty() => id,
                _ => continue,
            };

            // Load the stage and sync status
            // NOTE: We use stage.id (from YAML frontmatter) for graph operations,
            // not stage_id (from filename), because the graph is built using frontmatter IDs.
            let mut stage = match load_stage_at_path(&path) {
                Ok(stage) => stage,
                Err(e) => {
                    // A-4: do not silently skip a corrupt/unparseable stage file
                    // forever. Log at error with the file path so the operator
                    // sees a diagnostic instead of a frozen stage.
                    tracing::error!(
                        stage_id = %stage_id,
                        path = %path.display(),
                        error = %e,
                        "Failed to load stage file during sync; skipping (corrupt stage file?)"
                    );
                    continue;
                }
            };
            {
                tracing::debug!(
                    stage_id = %stage.id,
                    status = ?stage.status,
                    merged = stage.merged,
                    "[sync_graph_with_stage_files] Loaded stage"
                );
                // Always sync outputs to the graph so they're available for dependent stages
                if !stage.outputs.is_empty() {
                    self.graph
                        .set_node_outputs(&stage.id, stage.outputs.clone());
                }

                match stage.status {
                    StageStatus::Completed => {
                        // Verify merged=true non-knowledge stages: derive
                        // commit from branch HEAD if missing, then check
                        // ancestry. Old force-unsafe routes set merged=true
                        // without ever populating completed_commit, so a
                        // sync guard that only runs "when a commit exists"
                        // misses exactly that bug class.
                        // P-3: skip the git ancestry subprocess for a stage
                        // already verified merged this daemon session — the
                        // fact cannot change absent a history rewrite, and
                        // reconcile mutations invalidate the memo.
                        if stage.merged
                            && stage.stage_type != crate::models::stage::StageType::Knowledge
                            && !self.verified_merged.contains(&stage.id)
                        {
                            if self.verify_merged_true_or_revert(&mut stage, &path, &target_branch)
                            {
                                self.verified_merged.insert(stage.id.clone());
                            } else {
                                // Reverted / unverifiable — ensure it is not
                                // memoized so a later tick re-checks it.
                                self.verified_merged.remove(&stage.id);
                            }
                        }

                        // If stage is Completed but not merged, try to verify the
                        // merge via git ancestry. NEVER assume merged without proof —
                        // doing so produces phantom merges and lost work (see
                        // doc/plans/PLAN-fix-phantom-merge.md).
                        if !stage.merged {
                            // If completed_commit is missing, try to derive it from
                            // the stage's branch head before attempting verification.
                            if stage.completed_commit.is_none() {
                                let branch_name =
                                    crate::git::branch::branch_name_for_stage(&stage.id);
                                match crate::git::get_branch_head(
                                    &branch_name,
                                    &self.config.repo_root,
                                ) {
                                    Ok(head) => {
                                        tracing::info!(
                                            stage_id = %stage.id,
                                            commit = %head,
                                            "Derived completed_commit from branch head for recovery"
                                        );
                                        let stage_id = stage.id.clone();
                                        match persist_recovery_completed_commit(
                                            &stage_id,
                                            &path,
                                            &self.config.work_dir,
                                            head,
                                        ) {
                                            Ok(updated) => stage = updated,
                                            Err(error) => tracing::warn!(
                                                stage_id = %stage_id,
                                                %error,
                                                "Failed to save derived completed_commit"
                                            ),
                                        }
                                    }
                                    Err(_) => {
                                        // Branch is missing; cannot verify. Leave as
                                        // Completed + !merged. Do NOT save. This stage
                                        // is a candidate for the one-shot retry below
                                        // (Fix 11), in case the user ran `loom stage
                                        // complete --no-verify` before restart.
                                        tracing::error!(
                                            stage_id = %stage.id,
                                            branch = %branch_name,
                                            "Completed stage has no completed_commit and branch \
                                             is missing; cannot verify merge. Leaving as \
                                             Completed + !merged."
                                        );
                                        stuck_completed_stage_ids.push(stage.id.clone());
                                    }
                                }
                            }

                            // If we have a completed_commit (either pre-existing or
                            // just derived), run the ancestry check against the
                            // pass-hoisted target branch (P-3).
                            if let Some(completed_commit) = stage.completed_commit.clone() {
                                match crate::git::merge::verify_merge_succeeded(
                                    &completed_commit,
                                    &target_branch,
                                    &self.config.repo_root,
                                ) {
                                    Ok(true) => {
                                        tracing::info!(
                                            stage_id = %stage.id,
                                            "Auto-verified merge for completed stage, \
                                             marking as merged"
                                        );
                                        let stage_id = stage.id.clone();
                                        let mut applied = false;
                                        match update_stage_at_path(
                                            &stage_id,
                                            &path,
                                            &self.config.work_dir,
                                            |current| {
                                                if current.status == StageStatus::Completed
                                                    && current.completed_commit.as_deref()
                                                        == Some(completed_commit.as_str())
                                                {
                                                    current.merged = true;
                                                    applied = true;
                                                }
                                                Ok(())
                                            },
                                        ) {
                                            Ok(updated) => stage = updated,
                                            Err(error) => tracing::warn!(
                                                stage_id = %stage_id,
                                                %error,
                                                "Failed to save auto-verified merge state"
                                            ),
                                        }
                                        if applied && stage.merged {
                                            self.verified_merged.insert(stage_id);
                                        }
                                    }
                                    Ok(false) => {
                                        // Commit is not in target branch. Do NOT
                                        // write merged=true. Mark as a retry candidate
                                        // so the daemon makes a one-shot attempt.
                                        tracing::debug!(
                                            stage_id = %stage.id,
                                            commit = %completed_commit,
                                            target = %target_branch,
                                            "Completed stage commit is not yet in the target \
                                             branch; queuing the one-shot auto-merge attempt"
                                        );
                                        stuck_completed_stage_ids.push(stage.id.clone());
                                    }
                                    Err(e) => {
                                        // Verification failed (e.g., transient git
                                        // error). Do NOT write merged=true. Also a
                                        // retry candidate.
                                        tracing::error!(
                                            stage_id = %stage.id,
                                            error = %e,
                                            "Merge verification errored for completed stage; \
                                             leaving as Completed + !merged"
                                        );
                                        stuck_completed_stage_ids.push(stage.id.clone());
                                    }
                                }
                            }
                        }

                        if stage.status != StageStatus::Completed {
                            self.graph.set_node_merged(&stage.id, stage.merged);
                            if let Err(error) =
                                self.graph.mark_status(&stage.id, stage.status.clone())
                            {
                                tracing::warn!(
                                    stage_id = %stage.id,
                                    %error,
                                    "Failed to sync concurrently updated stage status"
                                );
                            }
                            continue;
                        }

                        // IMPORTANT: Set merged status FIRST, before mark_completed().
                        // mark_completed() triggers update_ready_status() which needs the
                        // correct merged value to determine if dependent stages are ready.
                        tracing::debug!(
                            stage_id = %stage.id,
                            merged = stage.merged,
                            "[sync_graph_with_stage_files] Completed stage"
                        );
                        self.graph.set_node_merged(&stage.id, stage.merged);
                        // Now mark as completed - this triggers update_ready_status() which
                        // will see the correct merged value set above
                        if let Err(e) = self.graph.mark_completed(&stage.id) {
                            tracing::warn!(
                                "Failed to sync graph status for stage {}: {}",
                                stage.id,
                                e
                            );
                        }
                    }
                    StageStatus::Queued => {
                        // Sync Ready status from stage files to graph
                        // This handles stages marked Ready by `loom verify` -> trigger_dependents()
                        if let Err(e) = self.graph.mark_queued(&stage.id) {
                            tracing::warn!(
                                "Failed to sync graph status for stage {}: {}",
                                stage.id,
                                e
                            );

                            // The file says Queued but the graph disagrees —
                            // dependencies are not satisfied here yet. Park the
                            // node at WaitingForDeps so `update_ready_status`
                            // can promote it once they are.
                            //
                            // Without this the node keeps whatever status it
                            // held (NeedsHandoff after a handoff re-queue,
                            // Blocked after a crash), and `update_ready_status`
                            // only ever promotes from WaitingForDeps — so the
                            // graph and the file disagree forever and the stage
                            // never runs, which is the exact shape of the bug
                            // this sync is meant to repair.
                            let needs_reset = self
                                .graph
                                .get_node(&stage.id)
                                .is_some_and(|n| n.status != StageStatus::WaitingForDeps);
                            if needs_reset {
                                if let Err(e) = self
                                    .graph
                                    .force_status(&stage.id, StageStatus::WaitingForDeps)
                                {
                                    tracing::warn!(
                                        stage_id = %stage.id,
                                        error = %e,
                                        "Failed to park unschedulable stage at WaitingForDeps"
                                    );
                                }
                            }
                        }
                    }
                    StageStatus::Executing => {
                        self.sync_executing_node(&stage.id);
                    }
                    StageStatus::Blocked => {
                        // Check if the blocked stage is eligible for automatic retry
                        if check_retry_eligibility(&stage) {
                            let stage_id = stage.id.clone();
                            let mut queued = false;
                            match update_stage_at_path(
                                &stage_id,
                                &path,
                                &self.config.work_dir,
                                |current| {
                                    if current.status == StageStatus::Blocked
                                        && check_retry_eligibility(current)
                                    {
                                        current.try_mark_queued()?;
                                        queued = true;
                                    }
                                    Ok(())
                                },
                            ) {
                                Ok(updated) => stage = updated,
                                Err(error) => {
                                    tracing::warn!(
                                        stage_id = %stage_id,
                                        %error,
                                        "Failed to save stage during retry"
                                    );
                                }
                            }
                            if queued {
                                clear_status_line();
                                tracing::warn!(
                                    stage_id = %stage_id,
                                    attempt = stage.retry_count + 1,
                                    "Auto-retrying stage"
                                );
                                if let Err(e) = self.graph.mark_queued(&stage_id) {
                                    tracing::warn!(
                                        "Failed to sync graph status for stage {}: {}",
                                        stage_id,
                                        e
                                    );
                                }
                            } else if stage.status != StageStatus::Blocked {
                                if let Err(error) =
                                    self.graph.mark_status(&stage_id, stage.status.clone())
                                {
                                    tracing::warn!(
                                        stage_id = %stage_id,
                                        %error,
                                        "Failed to sync concurrently updated retry status"
                                    );
                                }
                            }
                        } else {
                            // Not eligible for retry, just mark as blocked in graph
                            if let Err(e) = self.graph.mark_status(&stage.id, StageStatus::Blocked)
                            {
                                tracing::warn!(
                                    "Failed to sync graph status for stage {}: {}",
                                    stage.id,
                                    e
                                );
                            }
                        }
                    }
                    StageStatus::WaitingForInput => {
                        if let Err(e) = self
                            .graph
                            .mark_status(&stage.id, StageStatus::WaitingForInput)
                        {
                            tracing::warn!(
                                "Failed to sync graph status for stage {}: {}",
                                stage.id,
                                e
                            );
                        }
                    }
                    StageStatus::NeedsHandoff => {
                        if let Err(e) = self.graph.mark_status(&stage.id, StageStatus::NeedsHandoff)
                        {
                            tracing::warn!(
                                "Failed to sync graph status for stage {}: {}",
                                stage.id,
                                e
                            );
                        }
                    }
                    StageStatus::MergeConflict => {
                        if let Err(e) = self
                            .graph
                            .mark_status(&stage.id, StageStatus::MergeConflict)
                        {
                            tracing::warn!(
                                "Failed to sync graph status for stage {}: {}",
                                stage.id,
                                e
                            );
                        }
                    }
                    StageStatus::CompletedWithFailures => {
                        if let Err(e) = self
                            .graph
                            .mark_status(&stage.id, StageStatus::CompletedWithFailures)
                        {
                            tracing::warn!(
                                "Failed to sync graph status for stage {}: {}",
                                stage.id,
                                e
                            );
                        }
                    }
                    StageStatus::MergeBlocked => {
                        if let Err(e) = self.graph.mark_status(&stage.id, StageStatus::MergeBlocked)
                        {
                            tracing::warn!(
                                "Failed to sync graph status for stage {}: {}",
                                stage.id,
                                e
                            );
                        }
                    }
                    StageStatus::Skipped => {
                        if let Err(e) = self.graph.mark_status(&stage.id, StageStatus::Skipped) {
                            tracing::warn!(
                                "Failed to sync graph status for stage {}: {}",
                                stage.id,
                                e
                            );
                        }
                    }
                    StageStatus::WaitingForDeps => {
                        if let Err(e) = self
                            .graph
                            .mark_status(&stage.id, StageStatus::WaitingForDeps)
                        {
                            tracing::warn!(
                                "Failed to sync graph status for stage {}: {}",
                                stage.id,
                                e
                            );
                        }
                    }
                    StageStatus::NeedsHumanReview => {
                        if let Err(e) = self
                            .graph
                            .mark_status(&stage.id, StageStatus::NeedsHumanReview)
                        {
                            tracing::warn!(
                                "Failed to sync graph status for stage {}: {}",
                                stage.id,
                                e
                            );
                        }
                    }
                    StageStatus::NeedsAdjudication => {
                        if let Err(e) = self
                            .graph
                            .mark_status(&stage.id, StageStatus::NeedsAdjudication)
                        {
                            tracing::warn!(
                                "Failed to sync graph status for stage {}: {}",
                                stage.id,
                                e
                            );
                        }
                    }
                }
            }
        }

        // Fix 11: one-shot auto-merge for Completed + !merged stages.
        //
        // This loop is the normal merge path, not only a recovery path: the
        // sync runs before the monitor poll on every tick, so a stage the
        // daemon has just written as Completed (no merge yet, no
        // completed_commit) gets its first `try_auto_merge` here. The later
        // StageCompleted event finds it merged and only runs cleanup.
        //
        // It also covers the recovery cases the one-shot was written for: a
        // `loom stage complete --no-verify` completion followed by a daemon
        // restart, and a derived commit that ancestry reports Ok(false).
        //
        // A failed attempt leaves the stage MergeBlocked with the error in
        // failure_info; only auto-merge-disabled stages remain Completed +
        // !merged. `merge_retry_attempted` is in-memory only: the entry stays
        // in the set so a stage is not retried every 5-second poll, and
        // user-driven `loom stage merge` is independent of it.
        for stuck_id in stuck_completed_stage_ids {
            if self.merge_retry_attempted.contains(&stuck_id) {
                continue;
            }
            self.merge_retry_attempted.insert(stuck_id.clone());
            tracing::info!(
                stage_id = %stuck_id,
                "one-shot merge retry for stuck Completed + !merged stage"
            );
            // Ignore return value — even if the retry fails, we've logged it
            // and won't retry again this session.
            let _ = self.try_auto_merge(&stuck_id);
        }

        // After syncing all stage statuses, refresh ready status to ensure
        // dependent stages get marked as Queued when their dependencies complete.
        // This handles cases where stages are processed out of topological order.
        self.graph.refresh_ready_status();

        Ok(())
    }

    fn sync_queued_status_to_files(&mut self) -> Result<()> {
        // Get all nodes that are Queued in the graph
        let queued_stage_ids: Vec<String> = self
            .graph
            .all_nodes()
            .iter()
            .filter(|node| node.status == StageStatus::Queued)
            .map(|node| node.id.clone())
            .collect();

        let stages_dir = self.config.work_dir.join("stages");
        if !stages_dir.exists() {
            return Ok(());
        }
        let mut scan = StageScanCounter::default();
        let stage_paths: HashMap<String, PathBuf> = scan_stage_paths(&stages_dir, &mut scan)?
            .into_iter()
            .filter_map(|path| {
                let filename = path.file_name()?.to_str()?;
                crate::fs::stage_files::extract_stage_id(filename).map(|id| (id, path))
            })
            .collect();

        // For each queued stage, update the file if it's still WaitingForDeps
        for stage_id in queued_stage_ids {
            let Some(stage_path) = stage_paths.get(&stage_id) else {
                tracing::error!(
                    stage_id = %stage_id,
                    "Failed to locate stage during queued-status sync"
                );
                continue;
            };
            let updated =
                update_stage_at_path(&stage_id, stage_path, &self.config.work_dir, |stage| {
                    if stage.status == StageStatus::WaitingForDeps {
                        stage.try_mark_queued()?;
                    }
                    Ok(())
                });
            match updated {
                Ok(stage) if stage.status != StageStatus::Queued => {
                    if let Err(error) = self.graph.mark_status(&stage_id, stage.status.clone()) {
                        tracing::warn!(
                            stage_id = %stage_id,
                            %error,
                            "Failed to sync concurrently updated queued-stage status"
                        );
                    }
                }
                Ok(_) => {}
                Err(error) => tracing::error!(
                    stage_id = %stage_id,
                    %error,
                    "Failed to update stage during queued-status sync; skipping (corrupt stage file?)"
                ),
            }
        }

        Ok(())
    }

    fn adopt_orphaned_agents(&mut self) -> usize {
        let work_dir = self.config.work_dir.clone();
        let stages_dir = work_dir.join("stages");
        let mut adopted = 0;

        for evidence in orphan_evidence(&work_dir) {
            if self.try_adopt_orphan(&work_dir, &stages_dir, &evidence) {
                adopted += 1;
            }
        }

        adopted
    }

    fn recover_orphaned_sessions(&mut self) -> Result<usize> {
        // FIRST: give every live-but-unrecorded agent a record, so the
        // file-driven scan below sees it instead of concluding its stage is
        // idle. Idempotent — the record written here makes the next scan skip
        // that pid file entirely.
        self.adopt_orphans_and_log();

        let sessions_dir = self.config.work_dir.join("sessions");
        if !sessions_dir.exists() {
            return Ok(0);
        }

        let stages_by_id = self.index_stages_for_recovery()?;
        let mut recovered = 0;

        for entry in std::fs::read_dir(&sessions_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }

            // Load session from file. A read or parse failure must not abort
            // the whole recovery pass / daemon (O-4) — log and skip this file.
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to read session file during orphan recovery; skipping"
                    );
                    continue;
                }
            };
            let session: Session = match parse_from_markdown(&content, "Session") {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to parse session file during orphan recovery; skipping"
                    );
                    continue;
                }
            };

            let Some(stage_id) = session.stage_id.as_deref() else {
                continue;
            };
            let Some((indexed_stage, _)) = stages_by_id.get(stage_id) else {
                // A terminal/historical record that no current stage owns.
                continue;
            };
            if !session_is_current_for_stage(indexed_stage, &session) {
                // Historical sessions remain archival evidence. Never probe,
                // delete, or associate them with the stage's current attempt.
                continue;
            }

            // Check if the session is still running. Treat a probe *error* as
            // "unknown" and skip recovery this pass (O-9) — matching the
            // monitor's fail-safe behavior. Failing UNSAFE here (unwrap_or
            // false) would delete a live session's files and requeue its
            // stage, spawning a duplicate session into the same worktree.
            let is_running = match self.liveness.is_alive(&session) {
                Ok(alive) => alive,
                Err(e) => {
                    tracing::warn!(
                        session_id = %session.id,
                        error = %e,
                        "Liveness probe errored during orphan recovery; treating as unknown and skipping this pass"
                    );
                    continue;
                }
            };

            if is_running {
                register_live_current_session(&mut self.active_sessions, indexed_stage, &session);
                continue;
            }

            if !is_running {
                let stage_id = session.stage_id.as_deref().expect("checked above");
                let Some((_, stage_path)) = stages_by_id.get(stage_id) else {
                    continue;
                };

                // Git probing is deliberately outside the stage lock. The
                // operation revalidates the session association under the lock
                // before publishing only its recovery-owned fields.
                let branch_name = crate::git::branch::branch_name_for_stage(stage_id);
                let target_branch = crate::git::branch::resolve_target_branch(
                    &self.config.base_branch,
                    &self.config.repo_root,
                );
                let commits_ahead = crate::git::branch::commits_ahead_of(
                    &branch_name,
                    &target_branch,
                    &self.config.repo_root,
                )
                .unwrap_or(0);
                let route_to_handoff = commits_ahead > 0;
                let mut mutation_applied = false;
                let updated =
                    update_stage_at_path(stage_id, stage_path, &self.config.work_dir, |stage| {
                        if !session_is_current_for_stage(stage, &session) {
                            return Ok(());
                        }
                        match stage.status {
                            StageStatus::Executing
                            | StageStatus::NeedsHandoff
                            | StageStatus::Blocked => {
                                recover_orphaned_stage(
                                    stage,
                                    route_to_handoff,
                                    commits_ahead,
                                    &target_branch,
                                );
                                mutation_applied = true;
                            }
                            StageStatus::MergeConflict | StageStatus::MergeBlocked => {
                                stage.session = None;
                                stage.close_reason =
                                    Some("Merge session crashed/orphaned".to_string());
                                stage.updated_at = chrono::Utc::now();
                                mutation_applied = true;
                            }
                            _ => {}
                        }
                        Ok(())
                    })?;

                if !mutation_applied {
                    continue;
                }
                clear_status_line();
                tracing::warn!(
                    stage_id = %stage_id,
                    status = ?updated.status,
                    commits_ahead,
                    "Recovered orphaned current session"
                );
                if let Err(error) = self.graph.mark_status(stage_id, updated.status.clone()) {
                    tracing::warn!(stage_id = %stage_id, %error, "Failed to sync recovered stage status");
                }
                recovered += 1;

                // Remove the orphaned session file
                let _ = std::fs::remove_file(&path);

                // Remove the orphaned signal file
                let signal_path = self
                    .config
                    .work_dir
                    .join("signals")
                    .join(format!("{}.md", session.id));
                let _ = std::fs::remove_file(&signal_path);
            }
        }

        Ok(recovered)
    }

    fn all_stages_terminal(&self) -> bool {
        // Don't exit while merge resolution sessions are running — the daemon
        // needs to stay alive to monitor them and handle their completion.
        if !self.active_sessions.is_empty() {
            return false;
        }

        let stages_dir = self.config.work_dir.join("stages");
        if !stages_dir.exists() {
            return true;
        }

        for node in self.graph.all_nodes() {
            // Completed/Skipped in the graph are terminal without needing a
            // file read: no verdict, retry, approval, or amendment can ever
            // un-terminal them. Every other graph status can lag the stage
            // file by up to one tick after exactly those events, so the
            // decision for everything else comes from the file, not the
            // (possibly stale) graph status.
            if matches!(node.status, StageStatus::Completed | StageStatus::Skipped) {
                continue;
            }
            match self.load_stage(&node.id) {
                Ok(stage) if stage_file_is_terminal(&stage) => continue,
                Ok(_) => return false,
                // An unreadable stage file cannot be judged terminal.
                Err(_) => return false,
            }
        }
        true
    }

    fn print_status_update(&self) {
        // The `\r[Polling...]` line only makes sense on a TTY, where the
        // carriage return redraws it in place. When the daemon redirects
        // stdout to a log file (lifecycle.rs), these lines accumulate as
        // noise interleaved with tracing output (A-15 / O-17). Suppress them
        // when stdout is not a terminal; real tracing output is unaffected.
        if !io::stdout().is_terminal() {
            return;
        }

        let nodes = self.graph.all_nodes();
        let mut running = 0;
        let mut pending = 0;
        let mut completed = 0;
        let mut blocked = 0;

        for node in nodes {
            match node.status {
                StageStatus::Executing => running += 1,
                StageStatus::WaitingForDeps | StageStatus::Queued => pending += 1,
                StageStatus::Completed => completed += 1,
                StageStatus::Blocked => blocked += 1,
                StageStatus::Skipped => completed += 1, // Count skipped as completed for status display
                StageStatus::WaitingForInput => running += 1, // Paused but still active
                StageStatus::NeedsHandoff => running += 1, // Needs continuation but still in progress
                StageStatus::MergeConflict => blocked += 1, // Blocked on conflict resolution
                StageStatus::CompletedWithFailures => blocked += 1, // Failed acceptance, needs retry
                StageStatus::MergeBlocked => blocked += 1,          // Blocked on merge error
                StageStatus::NeedsHumanReview => blocked += 1,      // Waiting for human review
                StageStatus::NeedsAdjudication => blocked += 1, // Waiting for adjudicator verdict
            }
        }

        let mut status_parts = vec![
            format!("{running} running"),
            format!("{pending} pending"),
            format!("{completed} completed"),
        ];

        if blocked > 0 {
            status_parts.push(format!("{blocked} blocked"));
        }

        print!(
            "\r[Polling... {}] (Ctrl+C to detach, 'loom status' for details)    ",
            status_parts.join(", ")
        );
        // Flush stdout to ensure the status line appears immediately
        let _ = io::stdout().flush();
    }
}

#[cfg(test)]
mod tests {
    //! Focused unit tests for the building blocks of the recovery-path
    //! phantom-merge fix (PLAN-fix-phantom-merge.md Fix 1).
    //!
    //! Full integration coverage of `sync_graph_with_stage_files` requires a
    //! live `Orchestrator` with a real `Backend` and `Monitor`, which is too
    //! heavy for a unit test. Instead, we exercise the exact helper calls the
    //! recovery path makes in sequence:
    //!
    //! 1. `get_branch_head(&loom/<id>, repo_root)` to derive HEAD when
    //!    `completed_commit` is missing.
    //! 2. `is_ancestor_of(commit, target, repo_root)` to verify the derived
    //!    commit actually landed in the target branch.
    //!
    //! End-to-end recovery behavior (including the one-shot retry and stuck
    //! stage handling) is exercised by `loom/tests/phantom_merge.rs`.
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    use crate::git::branch::{
        branch_name_for_stage, commits_ahead_of, get_branch_head, is_ancestor_of,
    };

    #[test]
    fn stage_enumeration_and_direct_load_remain_linear_at_scale() {
        for count in [10usize, 100, 1000] {
            let temp = TempDir::new().unwrap();
            for index in 0..count {
                let stage = Stage {
                    id: format!("stage-{index}"),
                    ..Stage::default()
                };
                let content =
                    crate::verify::transitions::serialize_stage_to_markdown(&stage).unwrap();
                std::fs::write(
                    temp.path().join(format!("{index:04}-stage-{index}.md")),
                    content,
                )
                .unwrap();
            }

            let mut counter = StageScanCounter::default();
            let paths = scan_stage_paths(temp.path(), &mut counter).unwrap();
            let loaded: Vec<_> = paths
                .iter()
                .map(|path| load_stage_at_path(path).unwrap())
                .collect();

            assert_eq!(loaded.len(), count);
            assert_eq!(counter.directory_reads, 1);
            assert_eq!(counter.entries_visited, count);
        }
    }

    #[test]
    fn recovery_commit_and_merge_reconciliation_preserve_each_other() {
        use std::sync::{Arc, Barrier};

        let temp = TempDir::new().unwrap();
        let work_dir = temp.path().to_path_buf();
        let stage = Stage {
            id: "recovery-merge-race".to_string(),
            name: "Recovery merge race".to_string(),
            status: StageStatus::Completed,
            merged: true,
            ..Stage::default()
        };
        crate::verify::transitions::create_stage(&stage, &work_dir).unwrap();
        let stage_path = crate::fs::stage_files::find_stage_file(
            &work_dir.join("stages"),
            "recovery-merge-race",
        )
        .unwrap()
        .unwrap();
        let barrier = Arc::new(Barrier::new(3));

        let recovery_dir = work_dir.clone();
        let recovery_path = stage_path.clone();
        let recovery_barrier = Arc::clone(&barrier);
        let recovery = std::thread::spawn(move || {
            recovery_barrier.wait();
            persist_recovery_completed_commit(
                "recovery-merge-race",
                &recovery_path,
                &recovery_dir,
                "verified-commit".to_string(),
            )
            .unwrap();
        });

        let merge_dir = work_dir.clone();
        let merge_barrier = Arc::clone(&barrier);
        let merge = std::thread::spawn(move || {
            merge_barrier.wait();
            crate::orchestrator::merge_attribution::reconcile_attributed_stage_record(
                "recovery-merge-race",
                &merge_dir,
            )
            .unwrap();
        });

        barrier.wait();
        recovery.join().unwrap();
        merge.join().unwrap();

        let stage =
            crate::verify::transitions::load_stage("recovery-merge-race", &work_dir).unwrap();
        assert_eq!(stage.completed_commit.as_deref(), Some("verified-commit"));
        assert_eq!(stage.status, StageStatus::MergeConflict);
        assert!(stage.merge_conflict);
        assert!(!stage.merged);
    }

    /// A-2 / O-10 regression: `sync_graph_with_stage_files` derives the stage
    /// ID from the filename via `crate::fs::stage_files::extract_stage_id`. The
    /// previous hand-rolled parser stripped a leading digit then trimmed all
    /// leading digits+dashes, corrupting digit-leading IDs (`01-2fa-login.md`
    /// → `fa-login`). The stage then never synced and the plan deadlocked.
    /// This guards the exact extractor the sync loop now calls.
    #[test]
    fn extract_stage_id_preserves_digit_leading_ids() {
        use crate::fs::stage_files::extract_stage_id;
        assert_eq!(
            extract_stage_id("01-2fa-login.md"),
            Some("2fa-login".to_string()),
            "digit-leading stage IDs must survive filename parsing during sync"
        );
        assert_eq!(
            extract_stage_id("02-3d-render.md"),
            Some("3d-render".to_string())
        );
        // Non-digit-leading IDs and the no-prefix form must still work.
        assert_eq!(
            extract_stage_id("03-core-arch.md"),
            Some("core-arch".to_string())
        );
        assert_eq!(
            extract_stage_id("plain-id.md"),
            Some("plain-id".to_string())
        );
    }

    fn init_repo() -> TempDir {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(root)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(root)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(root)
            .output()
            .unwrap();
        std::fs::write(root.join("seed.txt"), "seed").unwrap();
        Command::new("git")
            .args(["add", "seed.txt"])
            .current_dir(root)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "seed"])
            .current_dir(root)
            .output()
            .unwrap();
        Command::new("git")
            .args(["branch", "-M", "main"])
            .current_dir(root)
            .output()
            .unwrap();

        tmp
    }

    /// Fix 1: when `completed_commit` is missing but the loom branch exists,
    /// recovery derives HEAD and checks ancestry. If the commit is NOT in the
    /// target branch, `merged` must stay false.
    ///
    /// This test stands in for the recovery-path decision: it verifies the
    /// helpers produce the exact (branch_head, is_ancestor=false) pair that
    /// the production code relies on to REFUSE to set merged=true.
    #[test]
    fn derive_head_and_ancestry_reports_not_ancestor_when_unmerged() {
        let repo = init_repo();
        let root = repo.path();

        // Create loom/oauth-hardening branch with a commit that stays off main.
        Command::new("git")
            .args(["checkout", "-b", "loom/oauth-hardening"])
            .current_dir(root)
            .output()
            .unwrap();
        std::fs::write(root.join("oauth.rs"), "hardened").unwrap();
        Command::new("git")
            .args(["add", "oauth.rs"])
            .current_dir(root)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "hardening"])
            .current_dir(root)
            .output()
            .unwrap();
        Command::new("git")
            .args(["checkout", "main"])
            .current_dir(root)
            .output()
            .unwrap();

        let branch = branch_name_for_stage("oauth-hardening");
        let head = get_branch_head(&branch, root).expect("HEAD derivable");
        assert!(
            !head.is_empty(),
            "branch HEAD should be derivable when branch exists"
        );

        let is_anc = is_ancestor_of(&head, "main", root).expect("ancestry check");
        assert!(
            !is_anc,
            "derived HEAD must NOT be an ancestor of main when branch is unmerged — \
             this is the exact signal that recovery uses to refuse merged=true (Fix 1)"
        );
    }

    /// Fix 1: when `completed_commit` is missing AND the loom branch is
    /// missing, recovery has no way to derive HEAD. The helper must surface
    /// the failure so the caller leaves the stage at Completed + !merged.
    #[test]
    fn derive_head_fails_when_branch_missing() {
        let repo = init_repo();
        let root = repo.path();

        let branch = branch_name_for_stage("nonexistent-stage");
        let result = get_branch_head(&branch, root);
        assert!(
            result.is_err(),
            "branch HEAD derivation must fail when branch does not exist — \
             recovery relies on this to log an error and leave stage as Completed + !merged"
        );
    }

    /// Fix 1 happy path: if the loom branch exists AND has been merged into
    /// main, the ancestry check returns true. The recovery path is allowed
    /// to set `merged = true` only in this case.
    #[test]
    fn ancestry_true_after_branch_merged_into_main() {
        let repo = init_repo();
        let root = repo.path();

        // Create a branch with a commit.
        Command::new("git")
            .args(["checkout", "-b", "loom/landed-stage"])
            .current_dir(root)
            .output()
            .unwrap();
        std::fs::write(root.join("landed.rs"), "done").unwrap();
        Command::new("git")
            .args(["add", "landed.rs"])
            .current_dir(root)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "landed"])
            .current_dir(root)
            .output()
            .unwrap();

        // Merge it into main with --no-ff.
        Command::new("git")
            .args(["checkout", "main"])
            .current_dir(root)
            .output()
            .unwrap();
        Command::new("git")
            .args([
                "merge",
                "--no-ff",
                "-m",
                "merge landed",
                "loom/landed-stage",
            ])
            .current_dir(root)
            .output()
            .unwrap();

        let branch = branch_name_for_stage("landed-stage");
        let head = get_branch_head(&branch, root).expect("HEAD derivable");
        let is_anc = is_ancestor_of(&head, "main", root).expect("ancestry check");
        assert!(
            is_anc,
            "after branch is merged into main, ancestry must be true — \
             recovery is then allowed to set merged=true"
        );
    }

    /// Orphan-recovery decision input: a stage whose worktree branch has
    /// uncommitted-merged commits beyond `main` should produce a positive
    /// `commits_ahead_of` count, which `recover_orphaned_sessions` reads to
    /// route the stage to `NeedsHandoff` instead of blindly re-queuing.
    ///
    /// Regression guard: this codifies the exact helper composition the
    /// recovery path makes — `branch_name_for_stage` + `commits_ahead_of`
    /// against the resolved target — so a refactor that breaks either
    /// surface caught here, not in production where the symptom is a
    /// wasteful retry of an already-committed stage.
    #[test]
    fn orphan_with_commits_ahead_signals_handoff_input() {
        let repo = init_repo();
        let root = repo.path();

        // Stage A: branch has 2 commits past main → handoff signal.
        Command::new("git")
            .args(["checkout", "-b", "loom/stage-with-work"])
            .current_dir(root)
            .output()
            .unwrap();
        for (i, name) in ["a.rs", "b.rs"].iter().enumerate() {
            std::fs::write(root.join(name), format!("{i}")).unwrap();
            Command::new("git")
                .args(["add", name])
                .current_dir(root)
                .output()
                .unwrap();
            Command::new("git")
                .args(["commit", "-m", &format!("commit-{i}")])
                .current_dir(root)
                .output()
                .unwrap();
        }
        Command::new("git")
            .args(["checkout", "main"])
            .current_dir(root)
            .output()
            .unwrap();

        let with_work = branch_name_for_stage("stage-with-work");
        assert_eq!(
            commits_ahead_of(&with_work, "main", root).unwrap(),
            2,
            "orphan recovery must see commits_ahead > 0 to route to NeedsHandoff"
        );

        // Stage B: branch never had commits (created and abandoned) →
        // no handoff signal, recovery should re-queue.
        Command::new("git")
            .args(["checkout", "-b", "loom/stage-no-work"])
            .current_dir(root)
            .output()
            .unwrap();
        Command::new("git")
            .args(["checkout", "main"])
            .current_dir(root)
            .output()
            .unwrap();

        let no_work = branch_name_for_stage("stage-no-work");
        assert_eq!(
            commits_ahead_of(&no_work, "main", root).unwrap(),
            0,
            "branch without commits must produce no handoff signal so retry can proceed"
        );

        // Stage C: no branch at all → defensive 0, never panics.
        let missing = branch_name_for_stage("never-spawned");
        assert_eq!(
            commits_ahead_of(&missing, "main", root).unwrap(),
            0,
            "missing branch must be treated as zero commits ahead (defensive)"
        );
    }
}

#[cfg(test)]
#[path = "recovery_adoption_tests.rs"]
mod recovery_adoption_tests;

#[cfg(test)]
#[path = "recovery_sync_tests.rs"]
mod recovery_sync_tests;

#[cfg(test)]
#[path = "recovery_terminal_tests.rs"]
mod recovery_terminal_tests;
