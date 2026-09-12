pub mod knowledge;
pub mod locking;
pub mod memory;
pub mod permissions;
pub mod plan_lifecycle;
pub mod plan_review;
pub mod safe_fs;
pub mod safe_read;
pub mod safe_write;
pub mod session_files;
pub mod stage_files;
pub mod stage_loading;
pub mod stage_request;
pub mod tmux_tmpdir;
pub mod verifications;
pub mod work_dir;
pub mod work_integrity;
pub mod worktree_files;

use anyhow::Result;
use std::path::{Path, PathBuf};

// Re-export commonly used config functions
pub use work_dir::{load_config, load_config_required, Config};

// Re-export stage loading functions
pub use stage_loading::{extract_stage_definition, load_stages_from_work_dir};

// Re-export session file utilities
pub use session_files::{
    find_session_for_stage, find_sessions_for_stage, save_session, session_to_markdown,
};

// Re-export verification utilities
pub use verifications::{
    delete_verification, list_verifications, load_verification, store_verification, GapRecord,
    VerificationRecord,
};

// Re-export plan lifecycle utilities
pub use plan_lifecycle::{
    all_stages_merged, mark_plan_done_if_all_merged, mark_plan_in_progress, DONE_PREFIX,
    IN_PROGRESS_PREFIX,
};

/// Parse base_branch from config.toml in a work directory.
///
/// This is a convenience function for extracting the base_branch field
/// from the plan configuration.
///
/// # Arguments
/// * `work_dir` - Path to the .loom/work directory
///
/// # Returns
/// * `Ok(Some(String))` - base_branch found in config
/// * `Ok(None)` - No config file or no base_branch field
/// * `Err(_)` - Failed to read or parse config
pub fn parse_base_branch_from_config(work_dir: &Path) -> Result<Option<String>> {
    match load_config(work_dir)? {
        Some(config) => Ok(config.base_branch()),
        None => Ok(None),
    }
}

/// Resolve the configured merge target without hiding malformed configuration.
///
/// A missing config or omitted `base_branch` uses the repository's default
/// branch. Read and parse failures are propagated so callers cannot silently
/// merge into a different branch.
pub fn resolve_target_branch_from_config(work_dir: &Path, repo_root: &Path) -> Result<String> {
    let configured = parse_base_branch_from_config(work_dir)?;
    Ok(crate::git::branch::resolve_target_branch(
        &configured,
        repo_root,
    ))
}

/// Get the merge point (base_branch) from config.toml in a work directory.
///
/// This is a convenience function that returns the base_branch field,
/// falling back to "main" if not configured.
///
/// # Arguments
/// * `work_dir` - Path to the .loom/work directory
///
/// # Returns
/// * `Ok(String)` - base_branch found in config, or "main" as default
/// * `Err(_)` - Failed to read or parse config
pub fn get_merge_point(work_dir: &Path) -> Result<String> {
    match load_config(work_dir)? {
        Some(config) => Ok(config.base_branch().unwrap_or_else(|| "main".to_string())),
        None => Ok("main".to_string()),
    }
}

/// Get the plan source path from config.toml in a work directory.
///
/// # Arguments
/// * `work_dir` - Path to the .loom/work directory
///
/// # Returns
/// * `Ok(Some(PathBuf))` - source_path found in config
/// * `Ok(None)` - No config file or no source_path field
/// * `Err(_)` - Failed to read or parse config
pub fn get_source_path(work_dir: &Path) -> Result<Option<PathBuf>> {
    match load_config(work_dir)? {
        Some(config) => Ok(config.source_path()),
        None => Ok(None),
    }
}

/// Resolve the plan source path to an absolute path.
///
/// In worktrees, `.loom/work` is a symlink to `../../../.loom/work`. A relative
/// `source_path` (e.g., `doc/plans/PLAN-foo.md`) must be resolved from the
/// **main** project root, not the worktree root. This function follows that
/// symlink (via [`work_dir::WorkDir::main_project_root`], the one place the
/// layout's hop count lives) to find the real project root.
///
/// Absolute paths are returned as-is for backward compatibility.
pub fn resolve_source_path(work_dir: &Path) -> Result<Option<PathBuf>> {
    let config = match load_config(work_dir)? {
        Some(c) => c,
        None => return Ok(None),
    };

    let source_path = match config.source_path() {
        Some(p) => p,
        None => return Ok(None),
    };

    if source_path.is_absolute() {
        return Ok(Some(source_path));
    }

    // Resolve relative paths from the main project root, following the
    // worktree symlink and applying the resolved layout's hop count.
    let project_root = work_dir::WorkDir::new(work_dir)
        .ok()
        .and_then(|wd| wd.main_project_root())
        .and_then(|p| p.canonicalize().ok())
        .unwrap_or_else(|| {
            work_dir
                .canonicalize()
                .ok()
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| work_dir.to_path_buf())
        });

    Ok(Some(project_root.join(&source_path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_branch_resolution_rejects_malformed_config() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("config.toml"), "base_branch = [").unwrap();

        let error = resolve_target_branch_from_config(temp.path(), temp.path()).unwrap_err();

        assert!(error.to_string().contains("parse config.toml"));
    }
}
