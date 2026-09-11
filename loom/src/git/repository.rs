//! Repository bootstrap helpers for Loom.
//!
//! Loom relies on git worktrees for stage isolation, which requires a
//! non-bare repository with at least one commit. These helpers ensure the
//! current project is worktree-capable without staging or committing user files.

use anyhow::{bail, Context, Result};
use std::path::Path;

use crate::git::init_blockers;
use crate::git::runner::{run_git, run_git_bool, run_git_checked};
use crate::git::worktree::check_git_available;

const INITIAL_COMMIT_MESSAGE: &str = "Initialize repository for loom";
const BOOTSTRAP_README: &str = "README.md";

/// Describes whether repository bootstrap changed the current project.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RepoBootstrapResult {
    pub initialized_repo: bool,
    pub created_initial_commit: bool,
    pub removed_stale_git_locks: bool,
    pub backed_up_git_config: bool,
}

impl RepoBootstrapResult {
    pub fn changed(self) -> bool {
        self.initialized_repo
            || self.created_initial_commit
            || self.removed_stale_git_locks
            || self.backed_up_git_config
    }
}

/// Ensure the current directory is a git repository with at least one commit.
///
/// This is the minimum git state Loom needs before it can create worktrees.
/// If the directory is not yet a git repo, this runs `git init`, first
/// clearing any leftover `.git/` contents (stale locks or an unreadable
/// config, e.g. from a sandbox placeholder or an interrupted git process)
/// that would make `git init` refuse to run. If the repo has no commits, it
/// creates a bootstrap commit containing only `README.md`.
pub fn ensure_repo_ready_for_worktrees(repo_root: &Path) -> Result<RepoBootstrapResult> {
    ensure_repo_ready_with_config(repo_root, &[])
}

pub(super) fn ensure_repo_ready_with_config(
    repo_root: &Path,
    git_config_args: &[&str],
) -> Result<RepoBootstrapResult> {
    check_git_available()?;

    let mut result = RepoBootstrapResult::default();

    if !is_git_repository(repo_root, git_config_args)? {
        let git_dir = repo_root.join(".git");
        if git_dir.is_dir() {
            result.removed_stale_git_locks = init_blockers::remove_stale_locks(&git_dir)?;
            result.backed_up_git_config = init_blockers::back_up_unreadable_config(repo_root)?;
        }
        initialize_repo(repo_root, git_config_args)?;
        result.initialized_repo = true;
    }

    if !has_head_commit(repo_root, git_config_args) {
        create_bootstrap_commit(repo_root, git_config_args)?;
        result.created_initial_commit = true;
    }

    Ok(result)
}

fn configured_args<'a>(git_config_args: &[&'a str], args: &[&'a str]) -> Vec<&'a str> {
    git_config_args.iter().chain(args.iter()).copied().collect()
}

fn is_git_repository(repo_root: &Path, git_config_args: &[&str]) -> Result<bool> {
    let args = configured_args(git_config_args, &["rev-parse", "--is-inside-work-tree"]);
    let output = run_git(&args, repo_root)?;
    Ok(output.status.success())
}

fn initialize_repo(repo_root: &Path, git_config_args: &[&str]) -> Result<()> {
    let args = configured_args(git_config_args, &["init"]);
    run_git_checked(&args, repo_root).with_context(|| {
        format!(
            "Failed to initialize git repository at {}",
            repo_root.display()
        )
    })?;
    Ok(())
}

fn has_head_commit(repo_root: &Path, git_config_args: &[&str]) -> bool {
    let args = configured_args(git_config_args, &["rev-parse", "--verify", "HEAD"]);
    run_git_bool(&args, repo_root)
}

fn create_bootstrap_commit(repo_root: &Path, git_config_args: &[&str]) -> Result<()> {
    ensure_git_identity(repo_root, git_config_args)?;
    ensure_bootstrap_readme(repo_root)?;
    let add_args = configured_args(git_config_args, &["add", "--", BOOTSTRAP_README]);
    run_git_checked(&add_args, repo_root)
        .context("Failed to stage README.md for Loom bootstrap commit")?;
    let commit_args = configured_args(
        git_config_args,
        &[
            "commit",
            "-m",
            INITIAL_COMMIT_MESSAGE,
            "--",
            BOOTSTRAP_README,
        ],
    );
    run_git_checked(&commit_args, repo_root).context("Failed to create Loom bootstrap commit")?;

    Ok(())
}

fn ensure_bootstrap_readme(repo_root: &Path) -> Result<()> {
    let readme_path = repo_root.join(BOOTSTRAP_README);
    if !readme_path.exists() {
        std::fs::write(&readme_path, "")
            .context("Failed to create README.md for Loom bootstrap")?;
    }
    Ok(())
}

fn ensure_git_identity(repo_root: &Path, git_config_args: &[&str]) -> Result<()> {
    read_git_config(repo_root, git_config_args, "user.name")?;
    read_git_config(repo_root, git_config_args, "user.email")?;
    Ok(())
}

fn read_git_config(repo_root: &Path, git_config_args: &[&str], key: &str) -> Result<String> {
    let args = configured_args(git_config_args, &["config", "--get", key]);
    let output = run_git(&args, repo_root)?;

    if output.status.success() {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !value.is_empty() {
            return Ok(value);
        }
    }

    bail!(
        "Git identity is required before Loom can create its bootstrap commit. \
Set it with:\n  git config --global user.name \"Your Name\"\n  git config --global user.email \"you@example.com\""
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::runner::run_git_checked;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    const TEST_IDENTITY: &[&str] = &[
        "-c",
        "user.name=Test User",
        "-c",
        "user.email=test@example.com",
    ];
    const EMPTY_IDENTITY: &[&str] = &["-c", "user.name=", "-c", "user.email="];

    fn init_repo_without_commits(path: &Path) {
        Command::new("git")
            .args(["init"])
            .current_dir(path)
            .output()
            .unwrap();
    }

    #[test]
    fn bootstraps_missing_repository() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();
        fs::write(repo_root.join("README.md"), "# temp\n").unwrap();

        let result = ensure_repo_ready_with_config(repo_root, TEST_IDENTITY).unwrap();

        assert_eq!(
            result,
            RepoBootstrapResult {
                initialized_repo: true,
                created_initial_commit: true,
                removed_stale_git_locks: false,
                backed_up_git_config: false,
            }
        );
        assert!(repo_root.join(".git").exists());
        assert!(
            !run_git_checked(&["rev-parse", "--verify", "HEAD"], repo_root)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            run_git_checked(&["show", "HEAD:README.md"], repo_root).unwrap(),
            "# temp"
        );

        let status = run_git_checked(&["status", "--porcelain"], repo_root).unwrap();
        assert!(status.is_empty());
    }

    #[test]
    fn bootstraps_repo_without_commits() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();
        init_repo_without_commits(repo_root);

        let result = ensure_repo_ready_with_config(repo_root, TEST_IDENTITY).unwrap();

        assert_eq!(
            result,
            RepoBootstrapResult {
                initialized_repo: false,
                created_initial_commit: true,
                removed_stale_git_locks: false,
                backed_up_git_config: false,
            }
        );
        assert!(
            !run_git_checked(&["rev-parse", "--verify", "HEAD"], repo_root)
                .unwrap()
                .is_empty()
        );
        assert!(repo_root.join("README.md").exists());
        assert_eq!(fs::read_to_string(repo_root.join("README.md")).unwrap(), "");
        assert_eq!(
            run_git_checked(&["show", "HEAD:README.md"], repo_root).unwrap(),
            ""
        );
    }

    #[test]
    fn preserves_staged_changes_in_unborn_repo() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();
        init_repo_without_commits(repo_root);

        fs::write(repo_root.join("tracked.txt"), "tracked\n").unwrap();
        run_git_checked(&["add", "tracked.txt"], repo_root).unwrap();

        let result = ensure_repo_ready_with_config(repo_root, TEST_IDENTITY).unwrap();

        assert!(result.created_initial_commit);
        assert!(repo_root.join("README.md").exists());

        let status = run_git_checked(&["status", "--porcelain"], repo_root).unwrap();
        assert_eq!(status, "A  tracked.txt");
    }

    #[test]
    fn fails_without_git_identity() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();
        init_repo_without_commits(repo_root);

        let result = ensure_repo_ready_with_config(repo_root, EMPTY_IDENTITY);

        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(error.contains("Git identity is required"));
        assert!(error.contains("git config --global user.name"));
        assert!(error.contains("git config --global user.email"));
    }

    #[test]
    fn noops_for_repo_with_existing_head() {
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

        let result = ensure_repo_ready_for_worktrees(repo_root).unwrap();

        assert_eq!(result, RepoBootstrapResult::default());
    }
}
