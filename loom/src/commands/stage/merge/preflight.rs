//! Preflight checks for `merge_retry`: resolves the stage ID, verifies the
//! stage and cwd are in a mergeable state, and refuses an in-progress merge —
//! all before the caller decides whether to spend a fix attempt on this run.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use crate::git::merge::merge_head_exists;
use crate::models::stage::{Stage, StageStatus};
use crate::verify::transitions::load_stage;

/// Everything a merge retry needs, resolved and validated up front.
pub(super) struct RetryPreflight {
    pub(super) stage_id: String,
    pub(super) stage: Stage,
    pub(super) repo_root: PathBuf,
    pub(super) worktree_root: PathBuf,
}

/// Resolve the stage ID, load and validate the stage, confirm we're running
/// from a worktree, and refuse if a merge is already in progress.
///
/// This runs BEFORE the fix_attempts increment so a refused retry does not
/// burn an attempt.
pub(super) fn retry_preflight(stage_id: Option<String>, work_dir: &Path) -> Result<RetryPreflight> {
    // Resolve stage ID: use provided or detect from current worktree branch
    let stage_id = super::resolve_stage_id(stage_id, "merge <stage-id>")?;

    let stage = load_stage(&stage_id, work_dir)?;
    require_merge_state(&stage, &stage_id)?;

    let (repo_root, worktree_root) = resolve_worktree_paths(&stage_id)?;

    Ok(RetryPreflight {
        stage_id,
        stage,
        repo_root,
        worktree_root,
    })
}

/// Verify the stage is in a merge-failed state (MergeConflict, MergeBlocked,
/// or Completed but not yet merged).
fn require_merge_state(stage: &Stage, stage_id: &str) -> Result<()> {
    let is_merge_state = matches!(
        stage.status,
        StageStatus::MergeConflict | StageStatus::MergeBlocked
    ) || (stage.status == StageStatus::Completed && !stage.merged);

    if !is_merge_state {
        bail!(
            "Stage '{}' is in '{}' status. Only MergeConflict, MergeBlocked, or Completed but \
             not yet merged stages can use merge.\n\
             \n\
             Current status: {}\n\
             \n\
             For other failure states, use:\n\
             - loom stage retry {stage_id}       (for Blocked or CompletedWithFailures)\n\
             - loom stage merge {stage_id} --resolved (after manually resolving conflicts)",
            stage_id,
            stage.status,
            stage.status,
        );
    }

    Ok(())
}

/// Confirm we're running from a worktree, resolve the repo root and worktree
/// root, and refuse if a merge is already in progress in either.
///
/// Run BEFORE incrementing fix_attempts so a refused retry does not burn an
/// attempt.
fn resolve_worktree_paths(stage_id: &str) -> Result<(PathBuf, PathBuf)> {
    // Verify we're in a worktree (cwd should contain .worktrees in its path)
    let cwd = std::env::current_dir().context("Failed to get current directory")?;
    let cwd_str = cwd.to_string_lossy();
    if !cwd_str.contains(".worktrees") {
        bail!(
            "merge must be run from within a worktree.\n\
             \n\
             Current directory: {}\n\
             \n\
             Navigate to the worktree first:\n\
             - cd .worktrees/{stage_id}",
            cwd.display(),
        );
    }

    // Find repo root (parent of .worktrees)
    let repo_root = super::find_repo_root(&cwd)?;

    // Resolve worktree root from cwd. `git rev-parse --show-toplevel` returns
    // the top of the working tree, which for a worktree is its root.
    let worktree_root = crate::git::run_git_checked(&["rev-parse", "--show-toplevel"], &cwd)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| cwd.clone());

    // Refuse if either main repo or the worktree has an active merge — running
    // a programmatic merge over an in-progress resolution would clobber the
    // user's work.
    let main_active = merge_head_exists(&repo_root)?;
    let worktree_active = merge_head_exists(&worktree_root)?;
    if main_active || worktree_active {
        bail!(
            "Cannot retry merge: a merge is already in progress (main: {main_active}, \
             worktree: {worktree_active}). Resolve or abort it first.",
        );
    }

    Ok((repo_root, worktree_root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_completed_stage_not_yet_merged() {
        let mut stage = Stage::new("stage-a".to_string(), None);
        stage.status = StageStatus::Completed;
        stage.merged = false;
        assert!(require_merge_state(&stage, "stage-a").is_ok());
    }

    #[test]
    fn refuses_completed_stage_already_merged() {
        let mut stage = Stage::new("stage-a".to_string(), None);
        stage.status = StageStatus::Completed;
        stage.merged = true;
        assert!(require_merge_state(&stage, "stage-a").is_err());
    }

    #[test]
    fn refuses_executing_stage() {
        let mut stage = Stage::new("stage-a".to_string(), None);
        stage.status = StageStatus::Executing;
        stage.merged = false;
        assert!(require_merge_state(&stage, "stage-a").is_err());
    }

    #[test]
    fn accepts_merge_blocked_stage() {
        let mut stage = Stage::new("stage-a".to_string(), None);
        stage.status = StageStatus::MergeBlocked;
        stage.merged = false;
        assert!(require_merge_state(&stage, "stage-a").is_ok());
    }
}
