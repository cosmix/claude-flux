//! Repair checks and fixes for the git pre-commit hook and the committed
//! `.claude/settings.json` permissions (hooks/env belong in
//! settings.local.json instead — see `settings_checks`).

use std::path::Path;

use anyhow::Result;

use super::{RepairIssue, Severity};
use crate::fs::permissions::LOOM_PERMISSIONS;
use crate::git::is_pre_commit_hook_installed;

/// Check the git hook, Claude permissions, and Codex-native hook installation.
pub(super) fn check(repo_root: &Path) -> Vec<RepairIssue> {
    let mut issues = Vec::new();

    // Outside a repository there is nothing to install into; `loom init`
    // bootstraps git first and installs the hook in its startup repair.
    if repo_root.join(".git").is_dir() && !is_pre_commit_hook_installed(repo_root) {
        issues.push(RepairIssue {
            severity: Severity::Info,
            description: "Git pre-commit hook not installed".to_string(),
            fix_description: "Install loom pre-commit hook".to_string(),
        });
    }

    if let Some(issue) = settings_permissions_issue(repo_root) {
        issues.push(issue);
    }
    if crate::fs::permissions::codex_hooks_need_install() {
        issues.push(RepairIssue {
            severity: Severity::Info,
            description: "Codex hook installation incomplete or outdated".to_string(),
            fix_description: "Install Loom's Codex-native hooks".to_string(),
        });
    }

    issues
}

/// Whether `.claude/settings.json` exists and carries every LOOM_PERMISSIONS entry.
fn settings_permissions_issue(repo_root: &Path) -> Option<RepairIssue> {
    let settings_path = repo_root.join(".claude/settings.json");
    let parsed = parse_settings_json(&settings_path);

    let missing_reason = match &parsed {
        Some(val) if has_all_loom_permissions(val) => None,
        Some(_) => Some("permissions missing"),
        None => Some("file missing"),
    }?;

    Some(RepairIssue {
        severity: Severity::Info,
        description: format!("Project .claude/settings.json incomplete ({missing_reason})"),
        fix_description: "Restore permissions to .claude/settings.json".to_string(),
    })
}

fn parse_settings_json(settings_path: &Path) -> Option<serde_json::Value> {
    if !settings_path.exists() {
        return None;
    }
    std::fs::read_to_string(settings_path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
}

fn has_all_loom_permissions(val: &serde_json::Value) -> bool {
    val.get("permissions")
        .and_then(|p| p.get("allow"))
        .and_then(|a| a.as_array())
        .map(|arr| {
            let allowed: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
            LOOM_PERMISSIONS.iter().all(|perm| allowed.contains(perm))
        })
        .unwrap_or(false)
}

/// Install Claude Code and Codex hooks, configure permissions, and rebuild the
/// skill keyword index. `verbose = true` (`loom repair --fix`) keeps today's
/// per-step output; `verbose = false` (`loom init`'s unattended repair pass)
/// does the identical work through the quiet variants and prints nothing.
pub(super) fn fix_hooks(repo_root: &Path, verbose: bool) -> Result<()> {
    use crate::fs::permissions::{ensure_loom_permissions, ensure_loom_permissions_quiet};
    if verbose {
        fix_hooks_with(
            repo_root,
            install_hook_assets,
            ensure_loom_permissions,
            rebuild_skill_index,
        )?;
    } else {
        fix_hooks_with(
            repo_root,
            install_hook_assets,
            ensure_loom_permissions_quiet,
            rebuild_skill_index_quiet,
        )?;
    }
    Ok(())
}

pub(super) fn install_hook_assets() -> Result<()> {
    crate::fs::permissions::install_loom_hooks()?;
    crate::fs::permissions::install_codex_hooks()?;
    Ok(())
}

pub(super) fn fix_hooks_with<I, P, R>(
    repo_root: &Path,
    install: I,
    permissions: P,
    rebuild: R,
) -> Result<()>
where
    I: FnOnce() -> Result<()>,
    P: FnOnce(&Path) -> Result<()>,
    R: FnOnce() -> Result<()>,
{
    install()?;
    permissions(repo_root)?;
    rebuild()
}

/// Rebuild the skill keyword index using the built-in skill_index command
fn rebuild_skill_index() -> Result<()> {
    crate::commands::skill_index::execute()
}

/// Quiet counterpart of [`rebuild_skill_index`] for the unattended repair path.
fn rebuild_skill_index_quiet() -> Result<()> {
    crate::commands::skill_index::execute_quiet()
}
