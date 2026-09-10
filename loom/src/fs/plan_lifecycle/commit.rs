//! Post-completion commit: keep the default branch clean after a plan is
//! marked done.

use anyhow::{Context, Result};
use colored::Colorize;
use std::path::Path;

use crate::fs::work_dir::WorkDir;

/// Commit tracked changes to keep the default branch clean after plan completion.
///
/// After all stages are merged and the plan is renamed to DONE, this commits:
/// - The plan file rename (IN_PROGRESS → DONE)
/// - Any other tracked modifications (e.g., knowledge files updated by integration-verify)
pub(super) fn commit_post_completion_changes(
    work_dir: &WorkDir,
    old_plan_path: &Path,
    new_plan_path: &Path,
) -> Result<()> {
    use crate::git::runner::run_git_checked;

    let repo_root = work_dir
        .project_root()
        .context("Failed to determine project root")?;

    // Stage the plan file rename: add the new DONE file and stage deletion of old
    run_git_checked(&["add", &new_plan_path.display().to_string()], repo_root)?;
    run_git_checked(&["add", &old_plan_path.display().to_string()], repo_root)?;

    // Stage any other tracked modifications (modified + deleted tracked files).
    // Does NOT add untracked files — safe for automated use.
    // Typically catches knowledge files updated by integration-verify.
    run_git_checked(&["add", "-u"], repo_root)?;

    // Check if there's anything staged to commit
    let staged = run_git_checked(&["diff", "--cached", "--name-only"], repo_root)?;
    if staged.is_empty() {
        return Ok(());
    }

    // Commit with a descriptive message
    let plan_name = new_plan_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("plan");
    run_git_checked(
        &[
            "commit",
            "-m",
            &format!("chore(loom): mark plan complete — {plan_name}"),
        ],
        repo_root,
    )?;

    println!(
        "  {} Committed post-completion changes to default branch",
        "✓".green().bold(),
    );

    Ok(())
}
