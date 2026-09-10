//! Batch cleanup operations

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

use super::base::cleanup_base_branch;
use super::branch::cleanup_branch;
use super::config::{CleanupConfig, CleanupResult};
use super::worktree::cleanup_worktree;
use crate::git::branch::branch_name_for_stage;
use crate::models::worktree::Worktree;

/// Perform full cleanup after a successful merge
///
/// This function removes all resources associated with a stage after its
/// branch has been successfully merged. It's safe to call even if some
/// resources have already been cleaned up.
///
/// # Arguments
/// * `stage_id` - The stage ID to clean up
/// * `repo_root` - Path to the repository root
/// * `config` - Cleanup configuration options
///
/// # Returns
/// A `CleanupResult` describing what was cleaned up
pub fn cleanup_after_merge(
    stage_id: &str,
    repo_root: &Path,
    config: &CleanupConfig,
) -> Result<CleanupResult> {
    let branch_name = branch_name_for_stage(stage_id);
    drain_spool_before_removal(stage_id, repo_root);
    let worktree_removed = cleanup_worktree(stage_id, repo_root, config.force_worktree_removal)
        .with_context(|| format!("Failed to remove worktree for stage '{stage_id}'"))?;
    let branch_deleted = cleanup_branch(stage_id, repo_root, config.force_branch_deletion)
        .with_context(|| format!("Failed to delete branch '{branch_name}'"))?;
    let base_branch_deleted = cleanup_base_branch(stage_id, repo_root)
        .with_context(|| format!("Failed to delete base branch for stage '{stage_id}'"))?;
    if config.prune_worktrees {
        prune_worktrees(repo_root).context("Failed to prune worktree metadata")?;
    }

    let result = CleanupResult {
        worktree_removed,
        branch_deleted,
        base_branch_deleted,
        warnings: Vec::new(),
    };
    if config.verbose {
        report_cleanup(stage_id, &branch_name, &result);
    }
    Ok(result)
}

/// Drain a stage's memory and telemetry spools one last time before its
/// worktree is removed. The spool files live inside the worktree, so anything
/// still pending when the worktree is deleted is lost for good.
///
/// Best-effort and NEVER fails the cleanup: a stage whose spool could not
/// be drained must still have its worktree and branches removed, otherwise a
/// spool problem would wedge the merge pipeline, which is far worse than
/// losing best-effort records. On success with entries drained, log at `info`; on
/// failure, log at `warn` and continue - the daemon's own per-tick drain
/// (`orchestrator::core::spool_drain`) will have already caught most
/// entries, so this is a last-chance sweep, not the primary path.
fn drain_spool_before_removal(stage_id: &str, repo_root: &Path) {
    let worktree_root = Worktree::worktree_path(repo_root, stage_id);
    if !worktree_root.exists() {
        return;
    }
    let work_dir = match crate::fs::work_dir::WorkDir::new(repo_root) {
        Ok(wd) => wd.root().to_path_buf(),
        Err(e) => {
            tracing::warn!(
                stage_id = %stage_id,
                error = %e,
                "Failed to resolve state directory before draining worktree spools; any pending \
                 entries will be lost with the worktree"
            );
            return;
        }
    };
    drain_memory_spool(stage_id, &work_dir, &worktree_root);
    drain_telemetry_spool(stage_id, &work_dir, &worktree_root);
}

fn drain_memory_spool(stage_id: &str, work_dir: &Path, worktree_root: &Path) {
    match crate::fs::memory::drain_into_journal(work_dir, stage_id, worktree_root) {
        Ok(outcome) if outcome.drained > 0 || outcome.skipped_malformed > 0 => {
            tracing::info!(
                stage_id = %stage_id,
                drained = outcome.drained,
                skipped_malformed = outcome.skipped_malformed,
                "Drained memory spool before worktree removal"
            );
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(
                stage_id = %stage_id,
                worktree_root = %worktree_root.display(),
                error = %e,
                "Failed to drain memory spool before worktree removal; any pending \
                 entries will be lost with the worktree"
            );
        }
    }
}

fn drain_telemetry_spool(stage_id: &str, work_dir: &Path, worktree_root: &Path) {
    match crate::telemetry::spool::drain_into_events(work_dir, worktree_root) {
        Ok(outcome) if outcome.drained > 0 || outcome.skipped_malformed > 0 => {
            tracing::info!(
                stage_id = %stage_id,
                drained = outcome.drained,
                skipped_malformed = outcome.skipped_malformed,
                "Drained telemetry spool before worktree removal"
            );
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(
                stage_id = %stage_id,
                worktree_root = %worktree_root.display(),
                error = %e,
                "Failed to drain telemetry spool before worktree removal; any pending entries will be lost with the worktree"
            );
        }
    }
}

fn report_cleanup(stage_id: &str, branch_name: &str, result: &CleanupResult) {
    if result.worktree_removed {
        println!("  Removed worktree: .worktrees/{stage_id}");
    }
    if result.branch_deleted {
        println!("  Deleted branch: {branch_name}");
    }
    if result.base_branch_deleted {
        println!("  Deleted base branch: loom/_base/{stage_id}");
    }
}

/// Prune stale worktree references
///
/// Runs `git worktree prune` to clean up any stale worktree metadata.
pub fn prune_worktrees(repo_root: &Path) -> Result<()> {
    let output = Command::new("git")
        .args(["worktree", "prune"])
        .current_dir(repo_root)
        .output()
        .with_context(|| "Failed to run git worktree prune")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git worktree prune failed: {stderr}");
    }

    Ok(())
}

/// Clean up multiple stages at once
///
/// # Arguments
/// * `stage_ids` - List of stage IDs to clean up
/// * `repo_root` - Path to the repository root
/// * `config` - Cleanup configuration options
///
/// # Returns
/// Map of stage_id to CleanupResult
pub fn cleanup_multiple_stages(
    stage_ids: &[&str],
    repo_root: &Path,
    config: &CleanupConfig,
) -> Vec<(String, CleanupResult)> {
    let mut results = Vec::with_capacity(stage_ids.len());

    for stage_id in stage_ids {
        let result =
            cleanup_after_merge(stage_id, repo_root, config).unwrap_or_else(|e| CleanupResult {
                worktree_removed: false,
                branch_deleted: false,
                base_branch_deleted: false,
                warnings: vec![e.to_string()],
            });
        results.push(((*stage_id).to_string(), result));
    }

    // Final prune after all cleanups
    if config.prune_worktrees {
        let _ = prune_worktrees(repo_root);
    }

    results
}

/// Check if a stage has resources that need cleanup
///
/// Returns true if the stage has a worktree, branch, or base branch that
/// exists. The base branch check matters: a prior cleanup that removed the
/// worktree and branch but failed on `loom/_base/<id>` must still be
/// reported as needing cleanup, or the still-true `cleanup_warning` on the
/// stage is silently stranded.
pub fn needs_cleanup(stage_id: &str, repo_root: &Path) -> bool {
    let worktree_path = repo_root.join(".worktrees").join(stage_id);
    let branch_name = branch_name_for_stage(stage_id);

    // Check worktree exists
    if worktree_path.exists() {
        return true;
    }

    // Check branch exists
    let output = Command::new("git")
        .args([
            "rev-parse",
            "--verify",
            &format!("refs/heads/{branch_name}"),
        ])
        .current_dir(repo_root)
        .output();

    if matches!(output, Ok(o) if o.status.success()) {
        return true;
    }

    // Check base branch exists; an unanswerable query counts as "absent".
    super::base::base_branch_exists(stage_id, repo_root).unwrap_or(false)
}
