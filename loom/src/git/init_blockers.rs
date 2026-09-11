//! Cleanup for leftovers that make `git init` refuse to run in a directory
//! git does not yet consider a repository.
//!
//! A `.git/` directory can exist without git recognizing it as a repository -
//! for example after a sandbox placeholder or an interrupted git process left
//! behind stale lock files or an unreadable `config`. `git init` then fails
//! instead of repairing itself. These helpers clear exactly those leftovers;
//! callers invoke them only after `git rev-parse --is-inside-work-tree` has
//! already failed, never as a general-purpose cleanup.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::git::runner::run_git;

/// Stale lock file names that block `git init` if left over from an
/// interrupted git process or a sandbox placeholder.
const STALE_LOCK_NAMES: &[&str] = &["config.lock", "HEAD.lock"];

/// Remove stale lock files from `git_dir` that would make `git init` refuse
/// to run. Returns `true` if anything was removed.
///
/// Only removes a name from [`STALE_LOCK_NAMES`], and only when the entry is
/// not a directory. Symlinks are removed as themselves, never followed.
pub(super) fn remove_stale_locks(git_dir: &Path) -> Result<bool> {
    let mut removed = false;
    for name in STALE_LOCK_NAMES {
        let path = git_dir.join(name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if metadata.is_dir() {
            continue;
        }
        fs::remove_file(&path)
            .with_context(|| format!("Failed to remove stale lock file: {}", path.display()))?;
        removed = true;
    }
    Ok(removed)
}

/// Back up `repo_root/.git/config` if it exists but git cannot parse it,
/// moving it aside so `git init` can create a fresh one.
///
/// Returns `false` if there is no config file, it is not a regular file, or
/// it is already readable (including empty). Returns `true` after moving an
/// unreadable config to `.git/config.loom-backup-<unix-seconds>`; its content
/// is preserved, never discarded.
pub(super) fn back_up_unreadable_config(repo_root: &Path) -> Result<bool> {
    let config_path = repo_root.join(".git/config");
    if !config_path.is_file() {
        return Ok(false);
    }

    let output = run_git(&["config", "--file", ".git/config", "--list"], repo_root)?;
    if output.status.success() {
        return Ok(false);
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let backup_path = repo_root.join(format!(".git/config.loom-backup-{timestamp}"));
    fs::rename(&config_path, &backup_path).with_context(|| {
        format!(
            "Failed to back up unreadable git config: {} -> {}",
            config_path.display(),
            backup_path.display()
        )
    })?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::repository::ensure_repo_ready_with_config;
    use crate::git::runner::run_git_checked;
    use std::process::Command;
    use tempfile::TempDir;

    const TEST_IDENTITY: &[&str] = &[
        "-c",
        "user.name=Test User",
        "-c",
        "user.email=test@example.com",
    ];

    fn init_repo_without_commits(path: &Path) {
        Command::new("git")
            .args(["init"])
            .current_dir(path)
            .output()
            .unwrap();
    }

    #[test]
    fn removes_stale_lock_files() {
        let temp = TempDir::new().unwrap();
        let git_dir = temp.path().join(".git");
        fs::create_dir_all(&git_dir).unwrap();
        fs::write(git_dir.join("config.lock"), "").unwrap();
        fs::write(git_dir.join("HEAD.lock"), "").unwrap();

        let removed = remove_stale_locks(&git_dir).unwrap();

        assert!(removed);
        assert!(!git_dir.join("config.lock").exists());
        assert!(!git_dir.join("HEAD.lock").exists());
    }

    #[test]
    fn reports_no_locks_removed_when_none_exist() {
        let temp = TempDir::new().unwrap();
        let git_dir = temp.path().join(".git");
        fs::create_dir_all(&git_dir).unwrap();

        let removed = remove_stale_locks(&git_dir).unwrap();

        assert!(!removed);
    }

    #[test]
    fn leaves_a_directory_named_like_a_lock_alone() {
        let temp = TempDir::new().unwrap();
        let git_dir = temp.path().join(".git");
        fs::create_dir_all(git_dir.join("config.lock")).unwrap();

        let removed = remove_stale_locks(&git_dir).unwrap();

        assert!(!removed);
        assert!(git_dir.join("config.lock").is_dir());
    }

    #[test]
    fn leaves_a_readable_config_untouched() {
        let temp = TempDir::new().unwrap();
        let repo_root = temp.path();
        fs::create_dir_all(repo_root.join(".git")).unwrap();
        fs::write(repo_root.join(".git/config"), "").unwrap();

        let backed_up = back_up_unreadable_config(repo_root).unwrap();

        assert!(!backed_up);
        assert!(repo_root.join(".git/config").exists());
    }

    #[test]
    fn backs_up_an_unparseable_config() {
        let temp = TempDir::new().unwrap();
        let repo_root = temp.path();
        fs::create_dir_all(repo_root.join(".git")).unwrap();
        fs::write(repo_root.join(".git/config"), "[[[garbage\n").unwrap();

        let backed_up = back_up_unreadable_config(repo_root).unwrap();

        assert!(backed_up);
        assert!(!repo_root.join(".git/config").exists());

        let backups: Vec<_> = fs::read_dir(repo_root.join(".git"))
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("config.loom-backup-")
            })
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            fs::read_to_string(backups[0].path()).unwrap(),
            "[[[garbage\n"
        );
    }

    #[test]
    fn reports_no_backup_when_config_is_missing() {
        let temp = TempDir::new().unwrap();
        let repo_root = temp.path();
        fs::create_dir_all(repo_root.join(".git")).unwrap();

        let backed_up = back_up_unreadable_config(repo_root).unwrap();

        assert!(!backed_up);
    }

    #[test]
    fn bootstraps_over_sandbox_leftovers_in_git_dir() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();
        let git_dir = repo_root.join(".git");
        fs::create_dir_all(git_dir.join("hooks")).unwrap();
        fs::write(git_dir.join("hooks/pre-commit"), "#!/bin/sh\necho loom\n").unwrap();
        fs::write(git_dir.join("config"), "").unwrap();
        fs::write(git_dir.join("config.lock"), "").unwrap();
        fs::write(git_dir.join("config.worktree"), "").unwrap();
        fs::write(git_dir.join("HEAD.lock"), "").unwrap();

        let result = ensure_repo_ready_with_config(repo_root, TEST_IDENTITY).unwrap();

        assert!(result.initialized_repo);
        assert!(result.created_initial_commit);
        assert!(result.removed_stale_git_locks);
        assert!(!result.backed_up_git_config);

        assert!(
            !run_git_checked(&["rev-parse", "--verify", "HEAD"], repo_root)
                .unwrap()
                .is_empty()
        );
        assert!(!git_dir.join("config.lock").exists());
        assert!(!git_dir.join("HEAD.lock").exists());
        assert_eq!(
            fs::read_to_string(git_dir.join("hooks/pre-commit")).unwrap(),
            "#!/bin/sh\necho loom\n"
        );
    }

    #[test]
    fn bootstraps_over_unreadable_git_config() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();
        let git_dir = repo_root.join(".git");
        fs::create_dir_all(&git_dir).unwrap();
        fs::write(git_dir.join("config"), "[[[garbage\n").unwrap();

        let result = ensure_repo_ready_with_config(repo_root, TEST_IDENTITY).unwrap();

        assert!(result.backed_up_git_config);
        assert!(
            !run_git_checked(&["rev-parse", "--verify", "HEAD"], repo_root)
                .unwrap()
                .is_empty()
        );

        let backups: Vec<_> = fs::read_dir(&git_dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("config.loom-backup-")
            })
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            fs::read_to_string(backups[0].path()).unwrap(),
            "[[[garbage\n"
        );
    }

    #[test]
    fn recovers_repo_with_history_and_unreadable_config() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();
        init_repo_without_commits(repo_root);

        fs::write(repo_root.join("README.md"), "# temp\n").unwrap();
        run_git_checked(&["add", "README.md"], repo_root).unwrap();
        Command::new("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                "Initial commit",
            ])
            .current_dir(repo_root)
            .output()
            .unwrap();

        fs::write(repo_root.join(".git/config"), "[[[garbage\n").unwrap();

        let result = ensure_repo_ready_with_config(repo_root, TEST_IDENTITY).unwrap();

        assert!(result.initialized_repo);
        assert!(!result.created_initial_commit);
        assert!(result.backed_up_git_config);

        let log = run_git_checked(&["log", "--oneline"], repo_root).unwrap();
        assert_eq!(log.lines().count(), 1);
        assert!(run_git_checked(&["show", "HEAD:README.md"], repo_root)
            .unwrap()
            .contains("temp"));
    }
}
