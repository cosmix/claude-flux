//! Plan file lifecycle management - renaming plan files based on execution state.
//!
//! This module handles:
//! - Adding `IN_PROGRESS-` prefix when execution starts
//! - Replacing `IN_PROGRESS-` with `DONE-` when all stages are merged
//! - Checking plan source paths in config.toml
//! - Checking if all stages are merged

use anyhow::{Context, Result};
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::value;

use crate::fs::work_dir::{self, WorkDir};
use crate::parser::frontmatter::extract_frontmatter_field;

// Filename prefix constants
pub const IN_PROGRESS_PREFIX: &str = "IN_PROGRESS-";
pub const DONE_PREFIX: &str = "DONE-";

// ===== Filename Operations =====

/// Add a prefix to the plan filename, preserving the directory
pub fn add_prefix_to_filename(path: &Path, prefix: &str) -> PathBuf {
    let parent = path.parent().unwrap_or(Path::new("."));
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("plan.md");

    parent.join(format!("{prefix}{filename}"))
}

/// Remove a prefix from the plan filename if present
pub fn remove_prefix_from_filename(path: &Path, prefix: &str) -> PathBuf {
    let parent = path.parent().unwrap_or(Path::new("."));
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("plan.md");

    if let Some(stripped) = filename.strip_prefix(prefix) {
        parent.join(stripped)
    } else {
        path.to_path_buf()
    }
}

/// Check if the filename has a specific prefix
pub fn has_prefix(path: &Path, prefix: &str) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| name.starts_with(prefix))
}

// ===== Config Operations =====

/// Get the plan source path from config.toml
///
/// This is a convenience wrapper around `crate::fs::get_source_path` that accepts
/// a `WorkDir` reference instead of a raw path.
pub fn get_plan_source_path(work_dir: &WorkDir) -> Result<Option<PathBuf>> {
    crate::fs::get_source_path(work_dir.root())
}

/// Update the plan source path in config.toml.
///
/// Goes through the centralized `fs::work_dir` API so comments and unknown
/// keys (other tables, sandbox snapshots, etc.) are preserved.
pub fn update_plan_source_path(work_dir: &WorkDir, new_path: &Path) -> Result<()> {
    let mut doc = work_dir::read_config(work_dir.root())?;

    let plan = doc.entry("plan").or_insert(toml_edit::table());
    if let Some(table) = plan.as_table_mut() {
        table["source_path"] = value(new_path.display().to_string());
    }

    work_dir::write_config(work_dir.root(), &doc)
        .context("Failed to write .loom/work/config.toml")?;
    Ok(())
}

// ===== Merge Status =====

/// Check if all stages are merged by reading stage files.
pub fn all_stages_merged(work_dir: &WorkDir) -> Result<bool> {
    let stages_dir = work_dir.root().join("stages");

    if !stages_dir.exists() {
        return Ok(false);
    }

    let entries = fs::read_dir(&stages_dir).context("Failed to read stages directory")?;

    let mut found_any_stage = false;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        found_any_stage = true;

        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read stage file: {}", path.display()))?;

        // Parse YAML frontmatter to check merged status
        match extract_frontmatter_field(&content, "merged") {
            Ok(Some(value)) if value == "true" => {
                // Stage is merged, continue checking others
            }
            _ => {
                // Not merged or error parsing
                return Ok(false);
            }
        }
    }

    // Must have at least one stage to be considered "all merged"
    Ok(found_any_stage)
}

// ===== Post-Completion Commit =====

/// Commit tracked changes to keep the default branch clean after plan completion.
///
/// After all stages are merged and the plan is renamed to DONE, this commits:
/// - The plan file rename (IN_PROGRESS → DONE)
/// - Any other tracked modifications (e.g., knowledge files updated by integration-verify)
fn commit_post_completion_changes(
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

// ===== Plan Lifecycle Functions =====

/// Mark the plan file as in-progress by adding `IN_PROGRESS-` prefix.
///
/// This is called when `loom run` starts execution.
/// If the plan already has `IN_PROGRESS-` prefix, this is a no-op.
/// If the plan has `DONE-` prefix, we skip (user re-running a completed plan).
///
/// Returns the new path if renamed, None if no rename was needed.
pub fn mark_plan_in_progress(work_dir: &WorkDir) -> Result<Option<PathBuf>> {
    let Some(current_path) = get_plan_source_path(work_dir)? else {
        return Ok(None);
    };
    let root = work_dir
        .project_root()
        .context("Cannot determine project root")?;

    // Already marked as in-progress
    if has_prefix(&current_path, IN_PROGRESS_PREFIX) {
        return Ok(None);
    }

    // Already done - user is re-running, leave as-is
    if has_prefix(&current_path, DONE_PREFIX) {
        println!(
            "  {} Plan already marked as DONE, skipping prefix update",
            "→".dimmed()
        );
        return Ok(None);
    }

    // Check file exists before renaming
    if !root.join(&current_path).exists() {
        return Ok(None);
    }

    let new_path = add_prefix_to_filename(&current_path, IN_PROGRESS_PREFIX);

    // Rename the file
    fs::rename(root.join(&current_path), root.join(&new_path)).with_context(|| {
        format!(
            "Failed to rename plan file from {} to {}",
            current_path.display(),
            new_path.display()
        )
    })?;

    // Update config.toml with new path
    update_plan_source_path(work_dir, &new_path)?;

    println!(
        "  {} Plan marked as in-progress: {}",
        "→".cyan().bold(),
        new_path.file_name().unwrap_or_default().to_string_lossy()
    );

    Ok(Some(new_path))
}

/// Mark the plan file as done by replacing `IN_PROGRESS-` with `DONE-` prefix.
///
/// This is called after the orchestrator completes successfully.
/// Only renames if all stages are merged. If not all merged, leaves as `IN_PROGRESS-`.
///
/// Returns the new path if renamed, None if no rename was needed.
pub fn mark_plan_done_if_all_merged(work_dir: &WorkDir) -> Result<Option<PathBuf>> {
    let Some(current_path) = get_plan_source_path(work_dir)? else {
        return Ok(None);
    };

    // Only process IN_PROGRESS files
    if !has_prefix(&current_path, IN_PROGRESS_PREFIX) {
        return Ok(None);
    }

    // Check if all stages are merged
    if !all_stages_merged(work_dir)? {
        println!(
            "  {} Not all stages merged, leaving plan as IN_PROGRESS",
            "→".yellow().bold()
        );
        return Ok(None);
    }

    // Check file exists before renaming
    if !current_path.exists() {
        return Ok(None);
    }

    // Remove IN_PROGRESS- and add DONE-
    let without_prefix = remove_prefix_from_filename(&current_path, IN_PROGRESS_PREFIX);
    let new_path = add_prefix_to_filename(&without_prefix, DONE_PREFIX);

    // Rename the file
    fs::rename(&current_path, &new_path).with_context(|| {
        format!(
            "Failed to rename plan file from {} to {}",
            current_path.display(),
            new_path.display()
        )
    })?;

    // Update config.toml with new path
    update_plan_source_path(work_dir, &new_path)?;

    println!(
        "  {} Plan marked as done: {}",
        "✓".green().bold(),
        new_path.file_name().unwrap_or_default().to_string_lossy()
    );

    // Commit tracked changes to leave the default branch clean
    if let Err(e) = commit_post_completion_changes(work_dir, &current_path, &new_path) {
        eprintln!(
            "  {} Warning: Failed to commit post-completion changes: {e}",
            "⚠".yellow().bold()
        );
    }

    Ok(Some(new_path))
}

#[cfg(test)]
#[path = "plan_lifecycle_tests.rs"]
mod tests;
