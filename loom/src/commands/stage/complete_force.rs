//! The `--force-unsafe` completion path.

use anyhow::{Context, Result};
use std::path::Path;

use crate::git::worktree::find_repo_root_from_cwd;
use crate::models::stage::{Stage, StageStatus};
use crate::verify::transitions::{trigger_dependents, update_stage};

use crate::commands::stage::acceptance_runner::resolve_stage_execution_paths;

use super::sync_worktree_permissions;

fn warn_force_unsafe() {
    eprintln!();
    eprintln!("⚠️  WARNING: Using --force-unsafe bypasses state machine validation!");
    eprintln!("⚠️  This can corrupt dependency tracking and cause unexpected behavior.");
    eprintln!("⚠️  Use only for manual recovery scenarios.");
    eprintln!();
}

fn apply_forced_merge(stage: &mut Stage, assume_merged: bool) {
    // Only set merged=true if explicitly requested via --assume-merged
    stage.merge_assumed = assume_merged;
    if assume_merged {
        stage.merged = true;
        println!("  → Stage marked as merged (manual merge assumed)");
    } else {
        stage.merged = false;
        eprintln!();
        eprintln!("⚠️  WARNING: Stage NOT marked as merged (--assume-merged not provided).");
        eprintln!("⚠️  Dependent stages will NOT be automatically triggered.");
        eprintln!("⚠️  If you manually merged the branch, re-run with --assume-merged to trigger dependents.");
        eprintln!();
    }
}

fn persist_forced_completion(stage: &Stage, stage_id: &str, work_dir: &Path) -> Result<()> {
    // Re-apply only the force-completion-owned fields (forced status, merged,
    // merge_assumed, completed_commit) onto the FRESH on-disk stage so an
    // administrative force-complete does not revert concurrent daemon/dispute
    // writes to unrelated fields (A-5). force_status_with_reason bypasses
    // transition validation by design — this is the documented administrative
    // override.
    let forced_merged = stage.merged;
    let forced_merge_assumed = stage.merge_assumed;
    let forced_commit = stage.completed_commit.clone();
    update_stage(stage_id, work_dir, |s| {
        s.force_status_with_reason(
            StageStatus::Completed,
            "--force-unsafe: administrative force-completion from any state",
        );
        s.merged = forced_merged;
        s.merge_assumed = forced_merge_assumed;
        s.completed_commit = forced_commit.clone();
        Ok(())
    })?;
    Ok(())
}

fn trigger_forced_dependents(stage_id: &str, work_dir: &Path) -> Result<()> {
    let cwd = std::env::current_dir().context("Failed to get current directory")?;
    let repo_root = find_repo_root_from_cwd(&cwd).unwrap_or_else(|| cwd.clone());
    let target_branch = crate::fs::resolve_target_branch_from_config(work_dir, &repo_root)?;
    let triggered = trigger_dependents(stage_id, work_dir, &repo_root, &target_branch)
        .context("Failed to trigger dependent stages")?;

    if !triggered.is_empty() {
        println!("Triggered {} dependent stage(s):", triggered.len());
        for dep_id in &triggered {
            println!("  → {dep_id}");
        }
    }
    Ok(())
}

/// Handle force-unsafe completion mode.
///
/// Bypasses state machine validation and marks stage as completed directly.
/// This is a manual recovery command for administrative use only.
///
/// # Invariant
///
/// **Callers MUST invoke `route_complete_for_conflicts` first.** This function
/// performs no ancestry check on its own — the verified-route guarantees the
/// router already established that the commit is in the target branch's
/// history (when `assume_merged=true`) or that no active merge would be
/// orphaned (when `assume_merged=false`).
pub(super) fn handle_force_unsafe_completion(
    mut stage: Stage,
    stage_id: &str,
    assume_merged: bool,
    work_dir: &Path,
) -> Result<()> {
    warn_force_unsafe();

    // Best-effort permission sync before force-completing
    // Uses resolve_stage_execution_paths to get worktree paths, same as normal completion
    if let Ok(execution_paths) = resolve_stage_execution_paths(&stage) {
        sync_worktree_permissions(
            &execution_paths.worktree_root,
            &execution_paths.acceptance_dir,
        );
    }

    println!(
        "Force-completing stage '{}' (was: {:?})",
        stage_id, stage.status
    );

    // Forced status assignment: --force-unsafe is an explicit administrative
    // override that may be invoked from any source status. Use
    // force_status_with_reason so the bypass is logged and visible.
    stage.force_status_with_reason(
        StageStatus::Completed,
        "--force-unsafe: administrative force-completion from any state",
    );

    apply_forced_merge(&mut stage, assume_merged);
    persist_forced_completion(&stage, stage_id, work_dir)?;
    println!("Stage '{stage_id}' force-completed!");

    // Only trigger dependent stages if merged=true (i.e., --assume-merged was used)
    if stage.merged {
        trigger_forced_dependents(stage_id, work_dir)?;
    }

    Ok(())
}
