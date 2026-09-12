//! Generate a review in the current checkout.
use crate::commands::common::resolve_work_dir;
use crate::git::worktree::find_worktree_root_from_cwd;
use anyhow::{Context, Result};
use colored::Colorize;
use std::{
    env,
    path::{Path, PathBuf},
};

fn resolve_output_root(cwd: &Path, project_root: &Path) -> PathBuf {
    find_worktree_root_from_cwd(cwd).unwrap_or_else(|| project_root.to_path_buf())
}

pub fn execute(ai_summary: bool) -> Result<()> {
    let workspace = resolve_work_dir()?;
    let root = workspace
        .main_project_root()
        .context("Could not determine project root")?;
    let cwd = env::current_dir().context("Failed to get current directory")?;
    let output_root = resolve_output_root(&cwd, &root);
    let output =
        crate::fs::plan_review::generate(workspace.root(), &root, &output_root, ai_summary)?;
    println!(
        "{} Review document written to {}",
        "✓".green().bold(),
        output
            .strip_prefix(&output_root)
            .unwrap_or(&output)
            .display()
            .to_string()
            .cyan()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_output_root_inside_worktree() {
        let cwd = Path::new("/repo/.worktrees/my-stage/loom/src");
        let project_root = PathBuf::from("/repo");
        assert_eq!(
            resolve_output_root(cwd, &project_root),
            PathBuf::from("/repo/.worktrees/my-stage")
        );
    }

    #[test]
    fn test_resolve_output_root_in_main_repo() {
        let cwd = Path::new("/repo/loom/src");
        let project_root = PathBuf::from("/repo");
        assert_eq!(resolve_output_root(cwd, &project_root), project_root);
    }
}
