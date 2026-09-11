//! Git operations for loom worktree management
//!
//! This module provides:
//! - Worktree creation/removal for parallel stage execution
//! - Branch management for stage isolation
//! - Merge operations for integrating completed work
//! - Cleanup utilities for successful merges
//! - Git hook installation for .loom/work protection

pub mod branch;
pub mod cleanup;
pub mod hooks;
mod init_blockers;
pub mod merge;
pub mod repository;
pub mod runner;
pub mod worktree;

// Re-export commonly used types and functions
pub use worktree::{
    check_git_available, check_worktree_support, clean_worktrees, create_worktree,
    ensure_work_symlink, get_or_create_worktree, get_worktree_path, list_worktrees,
    remove_worktree, resolve_base_branch, worktree_exists, BaseBranchError, ResolvedBase,
    WorktreeInfo,
};

pub use merge::{
    abort_merge, build_merge_report, check_merge_state, checkout_branch,
    conflict_resolution_instructions, get_conflicting_files, merge_stage, verify_merge_succeeded,
    MergeResult, MergeState, MergeStatusReport,
};

pub use repository::{ensure_repo_ready_for_worktrees, RepoBootstrapResult};

pub use branch::{
    branch_exists, branch_name_for_stage, cleanup_merged_branches, create_branch, current_branch,
    default_branch, delete_branch, get_branch_head, get_uncommitted_changes_summary,
    has_uncommitted_changes, is_branch_merged, list_branches, list_loom_branches,
    list_working_tree_changes, stage_id_from_branch, BranchInfo,
};

pub use cleanup::{
    base_branch_exists, cleanup_after_merge, cleanup_all_base_branches, cleanup_base_branch,
    cleanup_branch, cleanup_multiple_stages, cleanup_worktree, needs_cleanup, prune_worktrees,
    CleanupConfig, CleanupResult,
};

pub use hooks::{install_pre_commit_hook, is_pre_commit_hook_installed};

pub use runner::{run_git, run_git_bool, run_git_checked};

/// Initialize git module - check prerequisites
pub fn init() -> anyhow::Result<()> {
    check_git_available()?;
    check_worktree_support()?;
    Ok(())
}
