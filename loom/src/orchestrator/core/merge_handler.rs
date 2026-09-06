//! Merge session handling and auto-merge logic

use anyhow::{Context, Result};
use chrono::Utc;

use crate::git::branch::branch_name_for_stage;
use crate::git::cleanup::CleanupConfig;
use crate::git::merge::{check_merge_state, MergeState};
use crate::git::merge::{
    get_conflicting_files_from_status, verify_merge_succeeded, MergeProbeOutcome,
};
use crate::models::failure::{FailureInfo, FailureType};
use crate::models::session::{Session, SessionType};
use crate::models::stage::StageStatus;
use crate::orchestrator::auto_merge::{attempt_auto_merge, is_auto_merge_enabled, AutoMergeResult};
use crate::orchestrator::merge_lifecycle::{self, CleanupOutcome, MergeLifecycle};
use crate::orchestrator::signals::{
    find_live_merge_session_for_stage, generate_merge_signal, remove_signal,
};
use crate::process::is_process_alive;
use crate::verify::transitions::load_stage;

use super::persistence::Persistence;
use super::{clear_status_line, Orchestrator};

/// Maximum number of merge-resolver sessions the daemon will spawn for a single
/// stage before giving up and routing it to `NeedsHumanReview`. Mirrors the
/// crash-retry cap (`DEFAULT_MAX_RETRIES`).
///
/// Without this cap a resolver that fails fast and deterministically would be
/// respawned on every ~5s poll cycle (the kept signal file is NOT a guard —
/// `find_live_merge_session_for_stage` deletes it once the PID is dead), each
/// spawn on `opus`/`xhigh` → unbounded token + window burn (O-3).
const MAX_MERGE_RESOLVER_ATTEMPTS: u32 = 3;

impl Orchestrator {
    pub(super) fn handle_merge_session_completed(
        &mut self,
        session_id: &str,
        stage_id: &str,
    ) -> Result<()> {
        clear_status_line();
        eprintln!("Merge session '{session_id}' completed for stage '{stage_id}'");

        // Check if the merge was successful and update stage accordingly
        let mut stage = self.load_stage(stage_id)?;

        // Determine the merge point to check against
        let merge_point = crate::git::branch::resolve_target_branch(
            &self.config.base_branch,
            &self.config.repo_root,
        );

        // If stage is already marked as merged, do NOT trust the flag blindly
        // for non-knowledge stages. Derive commit if missing, then verify
        // ancestry; only treat as merged if the verification passes. Trust
        // merged=true only for knowledge stages (no branch by design).
        if stage.merged {
            let actually_merged = if stage.stage_type == crate::models::stage::StageType::Knowledge
            {
                true
            } else {
                let commit = match stage.completed_commit.clone() {
                    Some(c) => Some(c),
                    None => crate::git::get_branch_head(
                        &branch_name_for_stage(stage_id),
                        &self.config.repo_root,
                    )
                    .ok(),
                };
                commit
                    .map(|c| {
                        verify_merge_succeeded(&c, &merge_point, &self.config.repo_root)
                            .unwrap_or(false)
                    })
                    .unwrap_or(false)
            };

            if actually_merged {
                // Merge resolved - clean up signal and active session
                if let Err(e) = remove_signal(session_id, &self.config.work_dir) {
                    eprintln!("Warning: Failed to remove merge signal: {e}");
                }
                self.active_sessions.remove(stage_id);
                self.clear_merge_resolver_attempts(stage_id);
                clear_status_line();
                eprintln!("Stage '{stage_id}' merge completed successfully");
                return Ok(());
            }

            tracing::error!(
                stage_id = %stage_id,
                "Merge session ended with merged=true but ancestry verification failed; \
                 falling through to verify_and_finalize_merge to revert."
            );
            match self.update_stage(stage_id, |current| {
                current.merged = false;
                Ok(())
            }) {
                Ok(updated) => stage = updated,
                Err(error) => tracing::warn!(
                    stage_id = %stage_id,
                    %error,
                    "Failed to revert merged=true after failed verification"
                ),
            }
            // Fall through to the verify_and_finalize logic below.
        }

        // Check if the merge was actually successful by examining git state.
        // check_merge_state uses completed_commit + git ancestry (primary) and
        // falls back to metadata flags. With the reordered check, git ancestry
        // takes priority over the merge_conflict flag.
        let merge_state = check_merge_state(&stage, &merge_point, &self.config.repo_root);

        // If check_merge_state couldn't determine success (Conflict/Unknown — e.g.,
        // completed_commit was never set), fall back to checking the branch HEAD directly.
        let merge_state = match merge_state {
            Ok(MergeState::Conflict) | Ok(MergeState::Unknown) => {
                let branch_name = branch_name_for_stage(stage_id);
                match crate::git::get_branch_head(&branch_name, &self.config.repo_root) {
                    Ok(head) => {
                        match crate::git::branch::is_ancestor_of(
                            &head,
                            &merge_point,
                            &self.config.repo_root,
                        ) {
                            Ok(true) => Ok(MergeState::Merged),
                            _ => merge_state,
                        }
                    }
                    Err(_) => {
                        // Branch doesn't exist — may have been cleaned up after merge
                        if !crate::git::branch_exists(&branch_name, &self.config.repo_root)
                            .unwrap_or(true)
                        {
                            Ok(MergeState::BranchMissing)
                        } else {
                            merge_state
                        }
                    }
                }
            }
            other => other,
        };

        match merge_state {
            Ok(MergeState::Merged) => {
                // `finalize_merge_resolution` re-verifies git ancestry before
                // writing merged=true (phantom-merge invariant). If verification
                // unexpectedly fails here, it routes to the surface-to-user arm
                // instead of lying about merge status.
                if !self.finalize_merge_resolution(
                    &mut stage,
                    session_id,
                    stage_id,
                    &merge_point,
                    "merge verified and marked as complete",
                ) {
                    self.surface_unresolved_merge(stage_id);
                }
            }
            Ok(MergeState::BranchMissing) => {
                // BranchMissing means a commit was recorded but is NOT an ancestor
                // of the merge point AND the branch is gone — i.e. stranded work,
                // not a completed merge. Writing merged=true here is the documented
                // phantom-merge bug. `finalize_merge_resolution` ancestry-checks and
                // will return false; route to surface-to-user without lying.
                if !self.finalize_merge_resolution(
                    &mut stage,
                    session_id,
                    stage_id,
                    &merge_point,
                    "branch cleaned up after verified merge",
                ) {
                    tracing::error!(
                        stage_id = %stage_id,
                        "Merge session ended with branch missing but no ancestry proof \
                         that the work landed in the target. NOT marking merged=true \
                         (phantom-merge prevention). Run `loom stage merge {}` manually.",
                        stage_id
                    );
                    self.surface_unresolved_merge(stage_id);
                }
            }
            Ok(MergeState::Pending) | Ok(MergeState::Conflict) | Ok(MergeState::Unknown) => {
                // PID dead but merge not resolved - remove active session but KEEP signal
                // file so the user-facing instructions below are surfaced. NOTE: the
                // signal does NOT prevent respawn — `find_live_merge_session_for_stage`
                // deletes it once the PID is dead. Respawn is bounded by the per-stage
                // resolver attempt cap in `spawn_merge_resolution_sessions` (see O-3).
                self.surface_unresolved_merge(stage_id);
            }
            Err(e) => {
                // PID dead but merge state unknown - remove active session but KEEP signal
                // file so the user-facing instructions below are surfaced. As above, the
                // signal is not a respawn guard; the attempt cap bounds respawning.
                eprintln!("Warning: Failed to verify merge state: {e}");
                self.surface_unresolved_merge(stage_id);
            }
        }

        Ok(())
    }

    /// Remove active-session tracking for an unresolved merge and print
    /// user-facing recovery instructions.
    ///
    /// Used by all non-finalizing arms of `handle_merge_session_completed`.
    /// The stage is left in its current (MergeConflict/MergeBlocked) status so
    /// it remains visible to the user and to `spawn_merge_resolution_sessions`.
    fn surface_unresolved_merge(&mut self, stage_id: &str) {
        self.active_sessions.remove(stage_id);
        clear_status_line();
        eprintln!("Merge may not be complete. To finish:");
        eprintln!("  1. Verify the merge was successful: git status");
        eprintln!("  2. If merge is complete, run: loom worktree remove {stage_id}");
        eprintln!("  3. If issues remain, run: loom stage merge {stage_id}");
    }

    /// Finalize a resolved merge: write `merged=true`, transition to Completed,
    /// update the graph, and clean up the signal + active session.
    ///
    /// PHANTOM-MERGE INVARIANT: this is a daemon-side automated path, so it MUST
    /// NOT write `merged=true` without git ancestry proof. Before finalizing it
    /// derives `completed_commit` from the stage branch HEAD when missing and
    /// requires `verify_merge_succeeded(commit, merge_point)` to return
    /// `Ok(true)`. If that proof is unavailable the stage is left unchanged and
    /// the function returns `false` so the caller can surface the situation to
    /// the user instead of lying about merge status. Mirrors
    /// `verify_and_finalize_merge`.
    ///
    /// Returns `true` only when the merge was ancestry-verified and finalized.
    fn finalize_merge_resolution(
        &mut self,
        stage: &mut crate::models::stage::Stage,
        session_id: &str,
        stage_id: &str,
        merge_point: &str,
        log_message: &str,
    ) -> bool {
        // Derive completed_commit from the branch HEAD if missing so we have
        // something to ancestry-check. If neither is available, refuse.
        if stage.completed_commit.is_none() {
            let branch_name = branch_name_for_stage(stage_id);
            match crate::git::get_branch_head(&branch_name, &self.config.repo_root) {
                Ok(head) => stage.completed_commit = Some(head),
                Err(_) => {
                    tracing::error!(
                        stage_id = %stage_id,
                        "Cannot finalize merge: no completed_commit and branch HEAD \
                         unavailable; refusing to write merged=true (phantom-merge prevention)"
                    );
                    return false;
                }
            }
        }

        // Safe to unwrap: ensured Some above.
        let completed_commit = stage.completed_commit.clone().unwrap();
        match verify_merge_succeeded(&completed_commit, merge_point, &self.config.repo_root) {
            Ok(true) => {}
            other => {
                tracing::error!(
                    stage_id = %stage_id,
                    commit = %completed_commit,
                    target = %merge_point,
                    verified = ?other,
                    "Refusing to finalize merge: ancestry verification did not pass \
                     (phantom-merge prevention)"
                );
                return false;
            }
        }

        let updated = self.update_stage(stage_id, |current| {
            if !matches!(
                current.status,
                StageStatus::MergeConflict | StageStatus::MergeBlocked | StageStatus::Completed
            ) {
                anyhow::bail!(
                    "stage status changed to {} during merge verification",
                    current.status
                );
            }
            match current.completed_commit.as_deref() {
                Some(fresh) if fresh != completed_commit => {
                    anyhow::bail!("completed_commit changed during merge verification")
                }
                None => current.completed_commit = Some(completed_commit.clone()),
                Some(_) => {}
            }
            current.merged = true;
            current.merge_conflict = false;
            if matches!(
                current.status,
                StageStatus::MergeConflict | StageStatus::MergeBlocked
            ) {
                if let Err(error) = current.try_transition(StageStatus::Completed) {
                    current.force_status_with_reason(
                        StageStatus::Completed,
                        &format!("merge resolved but transition was illegal: {error}"),
                    );
                }
            }
            Ok(())
        });
        match updated {
            Ok(updated) => *stage = updated,
            Err(error) => {
                tracing::warn!(stage_id = %stage_id, %error, "Failed to persist merge resolution");
                return false;
            }
        }

        self.graph.set_node_merged(stage_id, true);
        if let Err(e) = self.graph.mark_completed(stage_id) {
            eprintln!("Warning: Failed to mark stage as completed in graph: {e}");
        }

        if let Err(e) = remove_signal(session_id, &self.config.work_dir) {
            eprintln!("Warning: Failed to remove merge signal: {e}");
        }
        self.active_sessions.remove(stage_id);
        self.clear_merge_resolver_attempts(stage_id);

        clear_status_line();
        eprintln!("Stage '{stage_id}' {log_message}");
        true
    }

    /// Verify merge succeeded and update stage state accordingly.
    ///
    /// This helper encapsulates the common pattern of verifying a merge via git ancestry
    /// check and updating stage/graph state based on the result.
    ///
    /// A failed verification always moves the stage to `MergeBlocked` with the
    /// reason in `failure_info`, whatever its prior status.
    ///
    /// Returns `true` if the merge was verified successful via git ancestry.
    /// Returns `false` otherwise. Caller must NOT assume `merged=true` on false.
    fn verify_and_finalize_merge(
        &mut self,
        stage: &mut crate::models::stage::Stage,
        stage_id: &str,
        target_branch: &str,
    ) -> bool {
        let branch_name = branch_name_for_stage(stage_id);
        let completed_commit = match stage
            .completed_commit
            .clone()
            .or_else(|| crate::git::get_branch_head(&branch_name, &self.config.repo_root).ok())
        {
            Some(commit) => commit,
            None => {
                tracing::error!(
                    stage_id = %stage_id,
                    branch = %branch_name,
                    "Cannot verify merge without completed_commit or branch HEAD"
                );
                self.persist_merge_blocked(
                    stage,
                    stage_id,
                    &format!(
                        "cannot verify merge: no completed_commit recorded and branch \
                         {branch_name} is missing"
                    ),
                );
                return false;
            }
        };

        match verify_merge_succeeded(&completed_commit, target_branch, &self.config.repo_root) {
            Ok(true) => self.persist_verified_merge(stage, stage_id, &completed_commit),
            Ok(false) => {
                tracing::error!(
                    stage_id = %stage_id,
                    commit = %completed_commit,
                    target = %target_branch,
                    "Merge verification failed: commit not in target branch"
                );
                self.persist_merge_blocked(
                    stage,
                    stage_id,
                    &format!(
                        "merge verification failed: commit {completed_commit} is not an \
                         ancestor of {target_branch}"
                    ),
                );
                false
            }
            Err(error) => {
                tracing::error!(
                    stage_id = %stage_id,
                    %error,
                    "Merge verification errored"
                );
                self.persist_merge_blocked(
                    stage,
                    stage_id,
                    &format!("merge verification errored: {error:#}"),
                );
                false
            }
        }
    }

    fn persist_verified_merge(
        &mut self,
        stage: &mut crate::models::stage::Stage,
        stage_id: &str,
        completed_commit: &str,
    ) -> bool {
        let updated = self.update_stage(stage_id, |current| {
            match current.completed_commit.as_deref() {
                Some(fresh) if fresh != completed_commit => {
                    anyhow::bail!("completed_commit changed during merge verification")
                }
                None => current.completed_commit = Some(completed_commit.to_string()),
                Some(_) => {}
            }
            current.merged = true;
            Ok(())
        });
        match updated {
            Ok(updated) => {
                *stage = updated;
                true
            }
            Err(error) => {
                tracing::warn!(stage_id = %stage_id, %error, "Failed to persist verified merge");
                false
            }
        }
    }

    /// Persist a failed auto-merge as `MergeBlocked`, recording `error` in
    /// `failure_info`. Applies to `Completed` stages too, so `loom status`
    /// shows the failure and `loom stage merge` can retry it.
    fn persist_merge_blocked(
        &mut self,
        stage: &mut crate::models::stage::Stage,
        stage_id: &str,
        error: &str,
    ) {
        let first_line = error.lines().next().unwrap_or(error);
        let updated = self.update_stage(stage_id, |current| {
            current.failure_info = Some(FailureInfo {
                failure_type: FailureType::InfrastructureError,
                detected_at: Utc::now(),
                evidence: error.lines().map(str::to_owned).collect(),
            });
            if current.try_mark_merge_blocked().is_err() {
                current.force_status_with_reason(
                    StageStatus::MergeBlocked,
                    &format!("auto-merge failed: {first_line}"),
                );
            }
            Ok(())
        });
        match updated {
            Ok(updated) => *stage = updated,
            Err(error) => {
                tracing::warn!(stage_id = %stage_id, %error, "Failed to persist MergeBlocked");
                return;
            }
        }
        if stage.status == StageStatus::MergeBlocked {
            let _ = self.graph.mark_status(stage_id, StageStatus::MergeBlocked);
            clear_status_line();
            eprintln!("Stage '{stage_id}' auto-merge failed: {first_line}");
            eprintln!("  Then run from the stage worktree: loom stage merge {stage_id}");
        }
    }

    /// Attempt auto-merge for a completed stage.
    ///
    /// Returns `true` if the merge succeeded or was not needed (stage can be marked Completed).
    /// Returns `false` if the merge failed with conflicts (stage should be marked MergeConflict).
    pub(super) fn try_auto_merge(&mut self, stage_id: &str) -> bool {
        // Load the stage to check auto_merge setting
        let mut stage = match load_stage(stage_id, &self.config.work_dir) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Warning: Failed to load stage for auto-merge check: {e}");
                // If we can't load the stage, allow completion to proceed
                return true;
            }
        };

        // If stage is already merged (e.g., by `loom stage complete`), skip auto-merge.
        // Without this guard, the daemon would redundantly attempt to merge an already-merged
        // stage. If cleanup partially removed the branch but not the worktree directory,
        // the redundant merge would fail and force-overwrite the Completed status to
        // MergeConflict/MergeBlocked — even though Completed is a terminal state.
        // This would spawn a spurious resolver session while the dependent stage was
        // already started by sync_graph_with_stage_files.
        if stage.merged {
            self.cleanup_already_merged(stage_id);
            return true;
        }

        // Load plan-level auto_merge setting from config.
        //
        // O-20: distinguish "no plan-level setting exists" (legitimate None →
        // fall back to the daemon/stage default) from "the plan file exists but
        // could not be read or parsed". In the latter case a plan-level
        // `auto_merge: false` may be present-but-unseen; silently defaulting to
        // enabled would merge a stage the user asked NOT to auto-merge. Log a
        // warning so the fallback is visible rather than silent.
        let plan_auto_merge = self.read_plan_level_auto_merge(stage_id);

        if !is_auto_merge_enabled(&stage, self.config.auto_merge, plan_auto_merge) {
            // Auto-merge disabled - skip the merge attempt and leave the stage as
            // Completed + !merged. The user will run `loom stage merge <id>`
            // manually when ready. DO NOT write merged=true here — that would
            // silently satisfy downstream dependency checks without the work
            // actually being merged.
            tracing::info!(
                stage_id = %stage_id,
                "auto-merge disabled; leaving stage as Completed + !merged \
                 (run `loom stage merge {}` to merge manually)",
                stage_id
            );
            return true;
        }

        // Get target branch (from config or default branch of the repo)
        let target_branch = crate::git::branch::resolve_target_branch(
            &self.config.base_branch,
            &self.config.repo_root,
        );

        // Phantom-merge guard: refuse to auto-merge a stage whose branch
        // EXISTS but has zero commits beyond the merge target. Without this
        // check, an empty branch (HEAD == target HEAD) silently "merges" as a
        // no-op: `completed_commit` is filled from branch HEAD (which equals
        // target HEAD), `attempt_auto_merge` returns AlreadyUpToDate /
        // FastForward, and `is_ancestor_of(target_HEAD, target)` trivially
        // passes — resulting in `merged: true` for work that was never
        // committed. Leave the stage at Completed + !merged so dependents do
        // not unblock. Skip the guard when the branch is missing — that path
        // already lands in the `NoBranch` / `Err` arms below with their own
        // recovery handling.
        let stage_branch = branch_name_for_stage(stage_id);
        let stage_branch_exists =
            crate::git::branch::branch_exists(&stage_branch, &self.config.repo_root)
                .unwrap_or(false);
        if stage_branch_exists {
            match crate::git::branch::commits_ahead_of(
                &stage_branch,
                &target_branch,
                &self.config.repo_root,
            ) {
                Ok(0) => {
                    tracing::error!(
                        stage_id = %stage_id,
                        branch = %stage_branch,
                        target = %target_branch,
                        "Stage branch has zero commits beyond target; routing to human review"
                    );
                    let reason = format!(
                        "branch {stage_branch} has zero commits beyond {target_branch}: the \
                         agent never committed work for this stage. Re-queue it with \
                         `loom stage human-review {stage_id} --approve`, or redo it manually."
                    );
                    self.route_to_human_review(stage_id, reason);
                    return false;
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(
                        stage_id = %stage_id,
                        branch = %stage_branch,
                        error = %e,
                        "commits_ahead_of probe failed; proceeding with merge attempt"
                    );
                }
            }
        }

        // Capture completed_commit before merge attempt so the orchestrator can
        // later verify merge resolution via git ancestry even if conflicts occur.
        if stage.completed_commit.is_none() {
            let branch_name = branch_name_for_stage(stage_id);
            if let Ok(head) = crate::git::get_branch_head(&branch_name, &self.config.repo_root) {
                match self.update_stage(stage_id, |current| {
                    if current.completed_commit.is_none() {
                        current.completed_commit = Some(head);
                    }
                    Ok(())
                }) {
                    Ok(updated) => stage = updated,
                    Err(error) => {
                        eprintln!("Warning: Failed to save completed_commit: {error}")
                    }
                }
            }
        }

        clear_status_line();
        eprintln!("Auto-merging stage '{stage_id}'...");

        MergeLifecycle::new(stage_id, &self.config.repo_root, &self.config.work_dir)
            .reconcile_overlay();

        match attempt_auto_merge(
            &stage,
            &self.config.repo_root,
            &self.config.work_dir,
            &target_branch,
            &self.backend,
        ) {
            Ok(AutoMergeResult::Success {
                files_changed,
                insertions,
                deletions,
            }) => self.finalize_auto_merge(
                &mut stage,
                stage_id,
                &target_branch,
                &format!("merged: {files_changed} files, +{insertions} -{deletions}"),
            ),
            Ok(AutoMergeResult::FastForward) => self.finalize_auto_merge(
                &mut stage,
                stage_id,
                &target_branch,
                "merged (fast-forward)",
            ),
            Ok(AutoMergeResult::AlreadyUpToDate) => {
                self.finalize_auto_merge(&mut stage, stage_id, &target_branch, "already up to date")
            }
            Ok(AutoMergeResult::ConflictResolutionSpawned {
                session,
                conflicting_files,
            }) => {
                // CRITICAL: Transition stage to MergeConflict status to prevent dependent stages
                // from starting before conflicts are resolved
                let updated = self.update_stage(stage_id, |current| {
                    if let Err(error) = current.try_mark_merge_conflict() {
                        current.force_status_with_reason(
                            StageStatus::MergeConflict,
                            &format!(
                                "auto-merge spawned a conflict resolver but transition was \
                                 illegal: {error}"
                            ),
                        );
                    }
                    Ok(())
                });
                let persisted = match updated {
                    Ok(updated) => Some(updated),
                    Err(error) => {
                        eprintln!("Warning: Failed to save stage merge conflict status: {error}");
                        None
                    }
                };

                // Also update the graph to reflect MergeConflict status
                if persisted
                    .as_ref()
                    .is_some_and(|stage| stage.status == StageStatus::MergeConflict)
                {
                    if let Err(e) = self.graph.mark_status(stage_id, StageStatus::MergeConflict) {
                        eprintln!("Warning: Failed to mark stage as merge conflict in graph: {e}");
                    }
                }

                // Track the merge session so the monitor can detect its lifecycle
                let session = *session;
                let session_id = session.id.clone();
                self.active_sessions
                    .insert(stage_id.to_string(), session.clone());
                if let Err(e) = self.save_session(&session) {
                    eprintln!("Warning: Failed to save merge session: {e}");
                    // Remove from active_sessions to avoid tracking a session
                    // that the monitor can't reload from disk after restart
                    self.active_sessions.remove(stage_id);
                }

                clear_status_line();
                eprintln!(
                    "Stage '{stage_id}' has {} conflict(s). Spawned resolution session: {session_id}",
                    conflicting_files.len()
                );

                // Return false to indicate merge did not succeed - stage should NOT be marked Completed
                false
            }
            Ok(AutoMergeResult::NoWorktree) => {
                // Nothing to merge - stage may have been created without worktree
                self.verify_and_finalize_merge(&mut stage, stage_id, &target_branch)
            }
            Err(e) => {
                clear_status_line();
                tracing::error!(
                    stage_id = %stage_id,
                    error = %e,
                    "Auto-merge failed"
                );
                // MergeBlocked with the error recorded, so status shows it and `loom stage merge` can retry.
                self.persist_merge_blocked(&mut stage, stage_id, &format!("{e:#}"));
                // Return false - merge failed, stage should not be marked Completed
                false
            }
        }
    }

    /// Run the deferred worktree/branch cleanup for a stage already marked
    /// merged (e.g. by `loom stage complete`).
    ///
    /// `loom stage complete` intentionally skips `cleanup_after_merge` when it
    /// runs from inside the worktree it would delete — removing the live
    /// agent session's cwd breaks every remaining Claude Code hook spawn for
    /// that session (Stop, SessionEnd, the trailing PostToolUse all fail with
    /// `posix_spawn '/bin/sh'` ENOENT once their cwd is gone). By the time
    /// this runs, cleanup is safe: the session has already exited (or this is
    /// a startup retry / prior-run stage), so nothing depends on the worktree
    /// still existing. The outcome is reported (not just logged at `warn`) so
    /// a failed or refused cleanup is visible on the daemon's console instead
    /// of only in tracing output nobody watches.
    fn cleanup_already_merged(&self, stage_id: &str) {
        let target_branch = crate::git::branch::resolve_target_branch(
            &self.config.base_branch,
            &self.config.repo_root,
        );
        let outcome = MergeLifecycle::new(stage_id, &self.config.repo_root, &self.config.work_dir)
            .cleanup(&target_branch, &CleanupConfig::quiet());
        report_deferred_cleanup(stage_id, &outcome);
    }

    /// Shared tail for `try_auto_merge`'s three merge-succeeded outcomes
    /// (Success, FastForward, AlreadyUpToDate): re-verify ancestry via
    /// `verify_and_finalize_merge`, and only on success run the primitive's
    /// post-merge base-reconcile + cleanup and print the summary line.
    fn finalize_auto_merge(
        &mut self,
        stage: &mut crate::models::stage::Stage,
        stage_id: &str,
        target_branch: &str,
        summary: &str,
    ) -> bool {
        let success = self.verify_and_finalize_merge(stage, stage_id, target_branch);
        if success {
            merge_lifecycle::finish_verified_merge(
                stage_id,
                &self.config.repo_root,
                &self.config.work_dir,
                target_branch,
                &CleanupConfig::quiet(),
            );
            clear_status_line();
            eprintln!("Stage '{stage_id}' {summary}");
        }
        success
    }

    /// Read the plan-level `auto_merge` flag from the active plan file.
    ///
    /// Returns:
    /// - `Some(flag)` when the plan declares an explicit `auto_merge` value.
    /// - `None` when there is genuinely no plan-level setting (no config, no
    ///   `source_path`, or the plan omits `auto_merge`) — callers fall back to
    ///   the daemon/stage default.
    ///
    /// O-20: when the plan path is known (a `source_path` exists) but the file
    /// cannot be read or its metadata cannot be parsed, this logs a warning and
    /// returns `None`. A plan-level `auto_merge: false` could be hiding in an
    /// unreadable plan; defaulting to enabled without surfacing that would
    /// silently override the user's intent.
    fn read_plan_level_auto_merge(&self, stage_id: &str) -> Option<bool> {
        let config = match crate::fs::load_config(&self.config.work_dir) {
            Ok(Some(c)) => c,
            Ok(None) => return None,
            Err(e) => {
                tracing::warn!(
                    stage_id = %stage_id,
                    error = %e,
                    "Failed to load config for plan-level auto_merge; \
                     falling back to default auto-merge setting"
                );
                return None;
            }
        };

        // No source_path → no plan file to consult; legitimate None.
        let source_path = config.source_path()?;
        let plan_path = self.config.repo_root.join(&source_path);

        let plan_content = match std::fs::read_to_string(&plan_path) {
            Ok(content) => content,
            Err(e) => {
                tracing::warn!(
                    stage_id = %stage_id,
                    plan_path = %plan_path.display(),
                    error = %e,
                    "Plan file is referenced but could not be read; a plan-level \
                     auto_merge:false may be ignored — falling back to default auto-merge setting"
                );
                return None;
            }
        };

        let yaml_content = match crate::plan::parser::extract_yaml_metadata(&plan_content) {
            Ok(y) => y,
            Err(e) => {
                tracing::warn!(
                    stage_id = %stage_id,
                    plan_path = %plan_path.display(),
                    error = %e,
                    "Failed to extract YAML metadata from plan; a plan-level \
                     auto_merge:false may be ignored — falling back to default auto-merge setting"
                );
                return None;
            }
        };

        match crate::plan::parser::parse_and_validate(&yaml_content) {
            Ok(metadata) => metadata.loom.auto_merge,
            Err(e) => {
                tracing::warn!(
                    stage_id = %stage_id,
                    plan_path = %plan_path.display(),
                    error = %e,
                    "Failed to parse plan metadata; a plan-level auto_merge:false may \
                     be ignored — falling back to default auto-merge setting"
                );
                None
            }
        }
    }

    /// Spawn merge resolution sessions for stages in MergeConflict or MergeBlocked status.
    ///
    /// Called during the main loop to detect stages that need merge resolution
    /// and spawn Claude Code sessions to resolve them.
    pub fn spawn_merge_resolution_sessions(&mut self) -> Result<usize> {
        let stages_dir = self.config.work_dir.join("stages");
        if !stages_dir.exists() {
            return Ok(0);
        }
        let mut spawned = 0;
        // Read all stage files
        for entry in std::fs::read_dir(&stages_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }

            // Extract stage ID from filename using the canonical parser
            let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            let stage_id = match crate::fs::stage_files::extract_stage_id(filename) {
                Some(id) => id,
                None => continue,
            };

            // Load stage and check status
            let stage = match self.load_stage(&stage_id) {
                Ok(s) => s,
                Err(_) => continue,
            };

            // Only handle MergeConflict and MergeBlocked statuses
            if !matches!(
                stage.status,
                StageStatus::MergeConflict | StageStatus::MergeBlocked
            ) {
                continue;
            }

            // Skip if there's already an active merge session for this stage.
            //
            // When `loom stage complete` detects a merge conflict, the stage transitions
            // to MergeConflict and `spawn_merge_resolver()` returns DaemonManaged (no
            // session spawned). However, the original execution session (SessionType::Stage)
            // may still be alive in active_sessions — it hasn't exited yet. That stage
            // session will never resolve the merge conflict, so we must not let it block
            // merge resolver spawning.
            //
            // Logic:
            // - If the active session is a Stage session (not Merge), it's stale in the
            //   context of merge resolution. Remove it and its signal, then fall through.
            // - If it IS a Merge session, only skip if the process is still alive.
            if self.active_sessions.contains_key(&stage_id) {
                let session = self.active_sessions.get(&stage_id).unwrap();
                if session.session_type != SessionType::Stage {
                    // It's a merge (or base-conflict) session — only skip if still alive
                    let is_alive = session.pid.map(is_process_alive).unwrap_or(false);
                    if is_alive {
                        continue;
                    }
                }
                // Either a stale Stage session or a dead Merge session — clean up
                let stale_session = self.active_sessions.remove(&stage_id).unwrap();
                let stale_session_id = stale_session.id.clone();
                // Kill the original session to prevent zombie processes.
                // When loom stage complete detected a merge conflict, the Stage session
                // may still be running -- actively terminate it.
                let kill_result = self.backend.kill_session(&stale_session);
                if let Err(e) = &kill_result {
                    tracing::debug!(
                        session_id = %stale_session_id,
                        error = %e,
                        "Failed to kill stale session (may already be dead)"
                    );
                }
                // Remove the old signal file so it doesn't block respawning
                if let Err(e) = remove_signal(&stale_session_id, &self.config.work_dir) {
                    eprintln!("Warning: Failed to remove stale signal for session '{stale_session_id}': {e}");
                }
            }

            // Use the shared helper that checks signal + PID liveness and
            // cleans up stale signals atomically.
            match find_live_merge_session_for_stage(&stage_id, &self.config.work_dir) {
                Ok(Some(_)) => continue, // Live resolver already running — skip
                Ok(None) => { /* fall through to spawn */ }
                Err(e) => {
                    tracing::warn!(
                        stage_id = %stage_id,
                        error = %e,
                        "Failed to check for existing merge signal; falling through to spawn"
                    );
                }
            }

            // O-3: bound merge-resolver respawning. Reaching this point means no
            // live resolver exists for a stage still in MergeConflict/MergeBlocked
            // — i.e. the previous resolver (if any) died without resolving. The
            // kept signal file is NOT a respawn guard (it was just deleted by
            // find_live_merge_session_for_stage when its PID was found dead), so
            // without a cap this loop respawns a fresh resolver every poll cycle.
            // Count only successfully spawned resolvers. Probe and spawn
            // failures are transient operational errors, not failed resolver
            // sessions, so they must leave the retry budget intact.
            let attempts = self.merge_resolver_attempts(&stage_id);
            if attempts >= MAX_MERGE_RESOLVER_ATTEMPTS {
                self.escalate_merge_resolver_exhausted(&stage_id, attempts);
                continue;
            }

            // Spawn a merge resolution session
            if let Err(e) = self.spawn_merge_resolution_session(&stage) {
                clear_status_line();
                eprintln!(
                    "Warning: Failed to spawn merge resolution session for '{stage_id}': {e}"
                );
            } else {
                spawned += 1;
            }
        }

        Ok(spawned)
    }

    /// Directory holding per-stage merge-resolver attempt counters.
    ///
    /// Stored on disk (rather than in memory) so the cap survives daemon
    /// restarts — a resolver that crash-loops across restarts must not reset
    /// its budget each time `loom run` starts.
    fn merge_resolver_attempts_dir(&self) -> std::path::PathBuf {
        self.config.work_dir.join("merge-resolver-attempts")
    }

    fn merge_resolver_attempts(&self, stage_id: &str) -> u32 {
        std::fs::read_to_string(
            self.merge_resolver_attempts_dir()
                .join(format!("{stage_id}.count")),
        )
        .ok()
        .and_then(|count| count.trim().parse::<u32>().ok())
        .unwrap_or(0)
    }

    /// Record a resolver only after its session successfully spawned.
    fn next_merge_resolver_attempt(&self, stage_id: &str) -> u32 {
        let dir = self.merge_resolver_attempts_dir();
        let path = dir.join(format!("{stage_id}.count"));
        let current = self.merge_resolver_attempts(stage_id);
        let next = current.saturating_add(1);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!(
                stage_id = %stage_id,
                error = %e,
                "Failed to create merge-resolver-attempts dir; proceeding without persisting count"
            );
            return next;
        }
        if let Err(e) = std::fs::write(&path, next.to_string()) {
            tracing::warn!(
                stage_id = %stage_id,
                error = %e,
                "Failed to persist merge-resolver attempt count; proceeding"
            );
        }
        next
    }

    /// Clear the persisted merge-resolver attempt counter for `stage_id`.
    ///
    /// Called once a merge is finalized so a later, unrelated conflict on the
    /// same stage id starts with a fresh budget.
    fn clear_merge_resolver_attempts(&self, stage_id: &str) {
        let path = self
            .merge_resolver_attempts_dir()
            .join(format!("{stage_id}.count"));
        if path.exists() {
            if let Err(e) = std::fs::remove_file(&path) {
                tracing::warn!(
                    stage_id = %stage_id,
                    error = %e,
                    "Failed to clear merge-resolver attempt counter"
                );
            }
        }
    }

    /// Persist `NeedsHumanReview` with `reason` as the review text and mirror it
    /// into the graph. Returns whether the persist succeeded.
    fn route_to_human_review(&mut self, stage_id: &str, reason: String) -> bool {
        let updated = self.update_stage(stage_id, |stage| {
            stage.force_status_with_reason(StageStatus::NeedsHumanReview, &reason);
            stage.review_reason = Some(reason.clone());
            Ok(())
        });
        if let Err(error) = updated {
            tracing::warn!(
                stage_id = %stage_id,
                %error,
                "Failed to save stage after routing to human review"
            );
            return false;
        }
        if let Err(e) = self
            .graph
            .mark_status(stage_id, StageStatus::NeedsHumanReview)
        {
            tracing::warn!(
                stage_id = %stage_id,
                error = %e,
                "Failed to mark stage NeedsHumanReview in graph after routing to human review"
            );
        }
        true
    }

    /// Route a stage whose merge-resolver budget is exhausted to
    /// `NeedsHumanReview` and persist it.
    ///
    /// `MergeConflict`/`MergeBlocked -> NeedsHumanReview` is not a legal edge, so
    /// this uses the sanctioned forced-assignment path. After escalation the
    /// stage is no longer in MergeConflict/MergeBlocked, so the spawn loop stops
    /// considering it and respawning ceases.
    fn escalate_merge_resolver_exhausted(&mut self, stage_id: &str, failed_attempts: u32) {
        let reason = format!(
            "merge resolution failed after {failed_attempts} resolver attempt(s); \
             escalating to human review. Resolve manually with `loom stage merge {stage_id}`."
        );
        tracing::error!(
            stage_id = %stage_id,
            failed_attempts = %failed_attempts,
            "Merge-resolver attempt cap reached; routing stage to NeedsHumanReview"
        );
        if !self.route_to_human_review(stage_id, reason) {
            return;
        }

        // Remove any lingering active session and clear the counter so a future
        // manual re-merge starts fresh.
        self.active_sessions.remove(stage_id);
        self.clear_merge_resolver_attempts(stage_id);

        clear_status_line();
        eprintln!(
            "Stage '{stage_id}' needs human review: merge resolution failed after \
             {failed_attempts} attempt(s). Run `loom stage merge {stage_id}` manually."
        );
    }

    /// Spawn a merge resolution session for a stage with merge issues.
    fn spawn_merge_resolution_session(
        &mut self,
        stage: &crate::models::stage::Stage,
    ) -> Result<()> {
        let source_branch = branch_name_for_stage(&stage.id);

        let target_branch = crate::git::branch::resolve_target_branch(
            &self.config.base_branch,
            &self.config.repo_root,
        );

        // Get conflicting files. If MERGE_HEAD is already set,
        // get_conflicting_files_from_status refuses (helper-level guard) — in
        // that case we fall back to reading the active merge's unmerged paths
        // directly. Only run the probe merge when there is NO active merge.
        let conflicting_files = if crate::git::merge::merge_head_exists(&self.config.repo_root)? {
            Vec::new()
        } else {
            match get_conflicting_files_from_status(
                &source_branch,
                &target_branch,
                &self.config.repo_root,
                &self.config.work_dir,
            )? {
                MergeProbeOutcome::Clean => Vec::new(),
                MergeProbeOutcome::Conflicts(files) => files,
            }
        };

        let session = Session::new_merge(source_branch.clone(), target_branch.clone());

        // Detect any active merge in the main repo so the signal can branch
        // between "start a fresh merge" and "continue the existing one".
        // If MERGE_HEAD is set, get_conflicting_files_from_status will refuse
        // (helper-level guard); fall back to reading the active merge's
        // unmerged paths directly.
        let in_progress = crate::git::merge::detect_in_progress_merge_at(&self.config.repo_root)?;
        let conflicting_files = if conflicting_files.is_empty() {
            match in_progress.as_ref().map(|m| &m.state) {
                Some(crate::git::merge::ActiveMergeState::HasUnmergedPaths(paths)) => paths.clone(),
                _ => conflicting_files,
            }
        } else {
            conflicting_files
        };

        let signal_path = generate_merge_signal(
            &session,
            stage,
            &source_branch,
            &target_branch,
            &conflicting_files,
            in_progress.as_ref(),
            &self.config.work_dir,
        )
        .context("Failed to generate merge signal")?;

        let spawned_session = self
            .backend
            .spawn_merge_session(stage, session, &signal_path, &self.config.repo_root)
            .context("Failed to spawn merge resolution session")?;

        self.next_merge_resolver_attempt(&stage.id);

        clear_status_line();
        eprintln!(
            "Spawned merge resolution session for stage '{}': {}",
            stage.id, spawned_session.id
        );

        if !conflicting_files.is_empty() {
            eprintln!("  Conflicting files:");
            for file in &conflicting_files {
                eprintln!("    - {file}");
            }
        }

        self.active_sessions
            .insert(stage.id.clone(), spawned_session.clone());

        self.save_session(&spawned_session)?;

        Ok(())
    }
}

/// Make a failed or refused deferred cleanup visible on the daemon's console.
///
/// `MergeLifecycle::cleanup` already logs at `warn`, but the daemon's tracing
/// goes to stderr nobody watches; the stage is Completed and merged either
/// way, so the only thing lost by silence is the worktree the user later
/// finds still on disk.
fn report_deferred_cleanup(stage_id: &str, outcome: &CleanupOutcome) {
    match outcome {
        CleanupOutcome::Failed(e) => {
            clear_status_line();
            eprintln!("Warning: deferred cleanup for stage '{stage_id}' failed: {e}");
            eprintln!("  Clean up manually with: loom worktree remove {stage_id}");
        }
        CleanupOutcome::Refused { reason } => {
            clear_status_line();
            eprintln!("Warning: deferred cleanup for stage '{stage_id}' refused: {reason}");
            eprintln!("  Clean up manually with: loom worktree remove {stage_id}");
        }
        CleanupOutcome::Done(_) | CleanupOutcome::NothingToDo | CleanupOutcome::Deferred => {}
    }
}

#[cfg(test)]
#[path = "merge_handler_attempt_tests.rs"]
mod merge_handler_attempt_tests;

#[cfg(test)]
mod tests {
    #[test]
    fn test_merge_blocked_to_completed_transition_is_valid() {
        use crate::models::stage::StageStatus;

        let status = StageStatus::MergeBlocked;
        assert!(
            status.can_transition_to(&StageStatus::Completed),
            "MergeBlocked -> Completed should be a valid transition"
        );

        let result = status.try_transition(StageStatus::Completed);
        assert!(
            result.is_ok(),
            "MergeBlocked -> Completed transition should succeed"
        );
    }

    #[test]
    fn test_merge_conflict_to_completed_transition_is_valid() {
        use crate::models::stage::StageStatus;

        let status = StageStatus::MergeConflict;
        assert!(
            status.can_transition_to(&StageStatus::Completed),
            "MergeConflict -> Completed should be a valid transition"
        );
    }

    #[test]
    fn test_merge_states_to_human_review_require_forced_assignment() {
        // O-3 escalation routes an exhausted-budget merge stage to
        // NeedsHumanReview. That edge is intentionally NOT in the transition
        // table (MergeConflict/MergeBlocked only legally go to Completed/Blocked
        // /Queued/Executing), so `escalate_merge_resolver_exhausted` MUST use
        // `force_status_with_reason`. If a future change makes this edge legal,
        // update the escalation path to prefer a `try_*` transition.
        use crate::models::stage::StageStatus;

        assert!(
            !StageStatus::MergeConflict.can_transition_to(&StageStatus::NeedsHumanReview),
            "MergeConflict -> NeedsHumanReview is expected to be illegal (escalation forces it)"
        );
        assert!(
            !StageStatus::MergeBlocked.can_transition_to(&StageStatus::NeedsHumanReview),
            "MergeBlocked -> NeedsHumanReview is expected to be illegal (escalation forces it)"
        );
    }

    #[test]
    fn test_max_merge_resolver_attempts_matches_default_retries() {
        // The merge-resolver respawn cap should mirror the crash-retry cap so
        // both failure-bounding mechanisms agree on "3 attempts".
        assert_eq!(super::MAX_MERGE_RESOLVER_ATTEMPTS, 3);
    }

    #[test]
    fn test_plan_auto_merge_extraction_true() {
        let plan_content = r#"# Test Plan

<!-- loom METADATA -->

```yaml
loom:
  version: 1
  auto_merge: true
  stages:
    - id: test-stage
      name: "Test"
      stage_type: knowledge
      working_dir: "."
      dependencies: []
      acceptance: []
```

<!-- END loom METADATA -->
"#;
        let yaml_content = crate::plan::parser::extract_yaml_metadata(plan_content).unwrap();
        let metadata = crate::plan::parser::parse_and_validate(&yaml_content).unwrap();
        assert_eq!(metadata.loom.auto_merge, Some(true));
    }

    #[test]
    fn test_plan_auto_merge_extraction_false() {
        let plan_content = r#"# Test Plan

<!-- loom METADATA -->

```yaml
loom:
  version: 1
  auto_merge: false
  stages:
    - id: test-stage
      name: "Test"
      stage_type: knowledge
      working_dir: "."
      dependencies: []
      acceptance: []
```

<!-- END loom METADATA -->
"#;
        let yaml_content = crate::plan::parser::extract_yaml_metadata(plan_content).unwrap();
        let metadata = crate::plan::parser::parse_and_validate(&yaml_content).unwrap();
        assert_eq!(metadata.loom.auto_merge, Some(false));
    }

    #[test]
    fn test_plan_auto_merge_default_none() {
        let plan_content = r#"# Test Plan

<!-- loom METADATA -->

```yaml
loom:
  version: 1
  stages:
    - id: test-stage
      name: "Test"
      stage_type: knowledge
      working_dir: "."
      dependencies: []
      acceptance: []
```

<!-- END loom METADATA -->
"#;
        let yaml_content = crate::plan::parser::extract_yaml_metadata(plan_content).unwrap();
        let metadata = crate::plan::parser::parse_and_validate(&yaml_content).unwrap();
        assert_eq!(metadata.loom.auto_merge, None);
    }
}
