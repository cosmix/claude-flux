//! Git hook installation for loom
//!
//! Installs git hooks to prevent accidental commits of .loom/work/ and .worktrees/

use anyhow::{bail, Context, Result};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Marker for loom's pre-commit hook section (for idempotent installation)
const LOOM_HOOK_START_MARKER: &str = "# LOOM_PRE_COMMIT_HOOK_START";
const LOOM_HOOK_END_MARKER: &str = "# LOOM_PRE_COMMIT_HOOK_END";

/// The pre-commit hook script content (embedded from hooks/git-pre-commit-hook.sh)
const PRE_COMMIT_HOOK_CONTENT: &str = include_str!("../../../hooks/git-pre-commit-hook.sh");

/// Install the pre-commit hook to the repository's .git/hooks directory
///
/// This function:
/// 1. Refuses if `repo_root` has no `.git` directory - there is nothing to
///    install into
/// 2. Creates the hooks directory if it doesn't exist
/// 3. Appends the loom hook to any existing pre-commit hook (idempotent)
/// 4. Creates a new pre-commit hook if none exists
/// 5. Makes the hook executable
///
/// # Arguments
/// * `repo_root` - Path to the repository root
///
/// # Returns
/// * `Ok(true)` - Hook was installed or updated
/// * `Ok(false)` - Hook was already up to date
/// * `Err` - Installation failed
pub fn install_pre_commit_hook(repo_root: &Path) -> Result<bool> {
    if !repo_root.join(".git").is_dir() {
        bail!(
            "{} is not a git repository (no .git directory); run `loom init` to set one up",
            repo_root.display()
        );
    }

    let git_hooks_dir = repo_root.join(".git/hooks");
    let hook_path = git_hooks_dir.join("pre-commit");

    // Ensure hooks directory exists
    if !git_hooks_dir.exists() {
        fs::create_dir_all(&git_hooks_dir).with_context(|| {
            format!(
                "Failed to create hooks directory: {}",
                git_hooks_dir.display()
            )
        })?;
    }

    // Extract only the loom section from the full hook file
    let loom_section = extract_loom_section(PRE_COMMIT_HOOK_CONTENT);

    let Some(content) = updated_hook_content(&hook_path, &loom_section)? else {
        return Ok(false); // Already up to date
    };

    fs::write(&hook_path, content)
        .with_context(|| format!("Failed to write hook: {}", hook_path.display()))?;

    make_executable(&hook_path)?;

    Ok(true)
}

/// Compute the new content for the pre-commit hook, or `None` if the hook is
/// already up to date and no write is needed.
fn updated_hook_content(hook_path: &Path, loom_section: &str) -> Result<Option<String>> {
    if !hook_path.exists() {
        // Create new hook with shebang and loom section
        return Ok(Some(format!(
            "#!/usr/bin/env bash\n# Git pre-commit hook\n\n{}",
            loom_section
        )));
    }

    let existing_content = fs::read_to_string(hook_path)
        .with_context(|| format!("Failed to read existing hook: {}", hook_path.display()))?;

    if existing_content.contains(LOOM_HOOK_START_MARKER) {
        let existing_section = extract_existing_loom_section(&existing_content);
        if existing_section.trim() == loom_section.trim() {
            return Ok(None); // Already up to date
        }

        // Replace existing loom section
        Ok(Some(replace_loom_section(&existing_content, loom_section)))
    } else {
        // Append loom section to existing hook
        Ok(Some(format!(
            "{}\n\n{}",
            existing_content.trim_end(),
            loom_section
        )))
    }
}

/// Make the pre-commit hook executable
fn make_executable(hook_path: &Path) -> Result<()> {
    let mut perms = fs::metadata(hook_path)
        .with_context(|| format!("Failed to get metadata for hook: {}", hook_path.display()))?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(hook_path, perms)
        .with_context(|| format!("Failed to set permissions on hook: {}", hook_path.display()))?;
    Ok(())
}

/// Extract the loom section from the full hook content
fn extract_loom_section(content: &str) -> String {
    let start = content.find(LOOM_HOOK_START_MARKER);
    let end = content.find(LOOM_HOOK_END_MARKER);

    match (start, end) {
        (Some(s), Some(e)) => content[s..e + LOOM_HOOK_END_MARKER.len()].to_string(),
        _ => {
            // Fallback: use the whole content (shouldn't happen with proper hook file)
            content.to_string()
        }
    }
}

/// Extract existing loom section from a hook file
fn extract_existing_loom_section(content: &str) -> String {
    extract_loom_section(content)
}

/// Replace the loom section in an existing hook file
fn replace_loom_section(existing: &str, new_section: &str) -> String {
    let start = existing.find(LOOM_HOOK_START_MARKER);
    let end = existing.find(LOOM_HOOK_END_MARKER);

    match (start, end) {
        (Some(s), Some(e)) => {
            let before = &existing[..s];
            let after = &existing[e + LOOM_HOOK_END_MARKER.len()..];
            format!("{}{}{}", before, new_section, after)
        }
        _ => {
            // Shouldn't happen, but append if markers not found
            format!("{}\n\n{}", existing.trim_end(), new_section)
        }
    }
}

/// Check if the pre-commit hook is installed
pub fn is_pre_commit_hook_installed(repo_root: &Path) -> bool {
    let hook_path = repo_root.join(".git/hooks/pre-commit");
    if !hook_path.exists() {
        return false;
    }

    match fs::read_to_string(&hook_path) {
        Ok(content) => content.contains(LOOM_HOOK_START_MARKER),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_extract_loom_section() {
        let content =
            "#!/bin/bash\n# LOOM_PRE_COMMIT_HOOK_START\necho test\n# LOOM_PRE_COMMIT_HOOK_END\n";
        let section = extract_loom_section(content);
        assert!(section.contains("LOOM_PRE_COMMIT_HOOK_START"));
        assert!(section.contains("echo test"));
        assert!(section.contains("LOOM_PRE_COMMIT_HOOK_END"));
    }

    #[test]
    fn install_pre_commit_hook_refuses_outside_repo() {
        let temp = TempDir::new().unwrap();

        let result = install_pre_commit_hook(temp.path());

        assert!(result.is_err());
        assert!(!temp.path().join(".git").exists());
    }

    #[test]
    fn test_install_pre_commit_hook_new() {
        let temp = TempDir::new().unwrap();
        let git_dir = temp.path().join(".git");
        fs::create_dir_all(&git_dir).unwrap();

        let result = install_pre_commit_hook(temp.path());
        assert!(result.is_ok());
        assert!(result.unwrap()); // Should return true for new install

        let hook_path = git_dir.join("hooks/pre-commit");
        assert!(hook_path.exists());

        let content = fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains(LOOM_HOOK_START_MARKER));
    }

    #[test]
    fn test_install_pre_commit_hook_idempotent() {
        let temp = TempDir::new().unwrap();
        let git_dir = temp.path().join(".git");
        fs::create_dir_all(&git_dir).unwrap();

        // First install
        let result1 = install_pre_commit_hook(temp.path());
        assert!(result1.is_ok());
        assert!(result1.unwrap());

        // Second install should return false (no changes)
        let result2 = install_pre_commit_hook(temp.path());
        assert!(result2.is_ok());
        assert!(!result2.unwrap());
    }

    #[test]
    fn test_is_pre_commit_hook_installed() {
        let temp = TempDir::new().unwrap();
        let git_dir = temp.path().join(".git");
        fs::create_dir_all(&git_dir).unwrap();

        // Not installed initially
        assert!(!is_pre_commit_hook_installed(temp.path()));

        // Install and check
        install_pre_commit_hook(temp.path()).unwrap();
        assert!(is_pre_commit_hook_installed(temp.path()));
    }
}
