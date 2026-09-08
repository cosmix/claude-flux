//! Workspace-shape checks and fixes: the state-directory shape and its
//! `.gitignore` entries, plus the unattended-repair API `loom init` uses to
//! heal a workspace before it is judged by
//! [`crate::fs::work_integrity::validate_work_dir_state`].

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use super::{RepairIssue, Severity};
use crate::fs::work_dir::Layout;
use crate::fs::work_integrity::{
    check_work_dir_state, is_in_worktree, is_work_dir_git_ignored, is_worktrees_git_ignored,
    state_dir, WorkDirState,
};

#[cfg(test)]
mod tests;

/// Check 1: state directory shape. Check 2: `.gitignore` has the state
/// directory. Check 3: `.gitignore` has `.worktrees`.
///
/// The descriptions below name the path `check_work_dir_state` actually
/// inspected — nested `.loom/work`, or, for a legacy workspace, `.work` — so
/// `WorkspaceFix::classify`'s substring dispatch matches on fragments that
/// stay the same across both spellings rather than on the interpolated path
/// itself.
pub(super) fn check(repo_root: &Path) -> Vec<RepairIssue> {
    let mut issues = Vec::new();

    let (_, work_layout) = state_dir(repo_root);
    let work_display_path = match work_layout {
        Layout::Nested => ".loom/work",
        Layout::Legacy => ".work",
    };

    // A symlink is corruption only in the main repo; in a worktree it is correct.
    match check_work_dir_state(repo_root) {
        WorkDirState::Symlink { target } if !is_in_worktree(repo_root) => {
            issues.push(RepairIssue {
                severity: Severity::Critical,
                description: format!("{work_display_path} is a symlink (-> {target}) in main repo"),
                fix_description: format!("Remove the {work_display_path} symlink and reinitialize"),
            });
        }
        WorkDirState::Invalid => {
            issues.push(RepairIssue {
                severity: Severity::Critical,
                description: format!(
                    "{work_display_path} exists but is neither directory nor symlink"
                ),
                fix_description: format!("Remove {work_display_path} and reinitialize"),
            });
        }
        _ => {}
    }

    if !is_work_dir_git_ignored(repo_root) {
        issues.push(RepairIssue {
            severity: Severity::Warning,
            description: format!("{work_display_path} not found in .gitignore"),
            fix_description: format!(
                "Add {work_display_path}/ and {work_display_path} to .gitignore"
            ),
        });
    }

    if !is_worktrees_git_ignored(repo_root) {
        issues.push(RepairIssue {
            severity: Severity::Warning,
            description: ".worktrees not found in .gitignore".to_string(),
            fix_description: "Add .worktrees/ and .worktrees to .gitignore".to_string(),
        });
    }

    issues
}

/// Fix a corrupted work-directory symlink in the main repo
///
/// The path is derived from [`state_dir`], the same resolver
/// `check_work_dir_state` uses, so this always operates on the path the
/// detector reported. Pointing it at the other layout's directory instead
/// would remove a healthy workspace and leave the reported corruption in
/// place.
pub(super) fn fix_work_symlink(repo_root: &Path) -> Result<()> {
    let (work_path, _layout) = state_dir(repo_root);
    fs::remove_file(&work_path)
        .with_context(|| format!("Failed to remove symlink at {}", work_path.display()))?;
    Ok(())
}

/// Fix an invalid work directory (neither dir nor symlink)
///
/// The path is derived from [`state_dir`], the same resolver
/// `check_work_dir_state` uses, and MUST stay in step with the detector: the
/// `else` branch below is a recursive delete, so aiming it at the other
/// layout's directory would destroy that workspace's stages, sessions,
/// signals, handoffs and memory while the real corruption survived.
pub(super) fn fix_invalid_work(repo_root: &Path) -> Result<()> {
    let (work_path, _layout) = state_dir(repo_root);
    if work_path.is_file() {
        fs::remove_file(&work_path)?;
    } else {
        fs::remove_dir_all(&work_path)?;
    }
    // The settings files may carry a LOOM_WORK_DIR pin naming the directory
    // we just deleted. Left in place, it shadows WorkDir::new's upward
    // search in every later session of this repo (see
    // scrub_stale_work_dir_env), so heal it now rather than leaving the very
    // next session to resolve a dead path.
    crate::fs::permissions::scrub_main_repo_settings_identity(repo_root);
    Ok(())
}

/// Add the state-directory entries to .gitignore
///
/// Writes whichever pair matches the resolved layout (see [`state_dir`]): the
/// nested `.loom/work/` and `.loom/work` pair, or, for a legacy workspace,
/// the `.work/` and `.work` pair. This must stay in step with
/// `is_work_dir_git_ignored`, which checks the same layout-dependent pair —
/// otherwise a fix here would never satisfy the check that raised the issue,
/// and `loom repair --fix` would never converge.
pub(super) fn fix_gitignore_work(repo_root: &Path) -> Result<()> {
    let (_, layout) = state_dir(repo_root);
    let (slash, bare) = match layout {
        Layout::Nested => (".loom/work/", ".loom/work"),
        Layout::Legacy => (".work/", ".work"),
    };

    let gitignore_path = repo_root.join(".gitignore");
    let mut content = if gitignore_path.exists() {
        fs::read_to_string(&gitignore_path)?
    } else {
        String::new()
    };

    // Add entries if not present
    let has_work_dir = content.lines().any(|l| l.trim() == slash);
    let has_work = content.lines().any(|l| l.trim() == bare);

    if !has_work_dir || !has_work {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        if !content.is_empty() {
            content.push('\n');
        }
        content.push_str("# loom workspace state\n");
        if !has_work_dir {
            content.push_str(slash);
            content.push('\n');
        }
        if !has_work {
            content.push_str(bare);
            content.push('\n');
        }
        fs::write(&gitignore_path, content)?;
    }

    Ok(())
}

/// Add .worktrees entries to .gitignore
pub(super) fn fix_gitignore_worktrees(repo_root: &Path) -> Result<()> {
    let gitignore_path = repo_root.join(".gitignore");
    let mut content = if gitignore_path.exists() {
        fs::read_to_string(&gitignore_path)?
    } else {
        String::new()
    };

    let has_worktrees_dir = content.lines().any(|l| l.trim() == ".worktrees/");
    let has_worktrees = content.lines().any(|l| l.trim() == ".worktrees");

    if !has_worktrees_dir || !has_worktrees {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        if !content.is_empty() && !content.contains("# loom worktrees") {
            content.push('\n');
        }
        if !content.contains("# loom worktrees") {
            content.push_str("# loom worktrees\n");
        }
        if !has_worktrees_dir {
            content.push_str(".worktrees/\n");
        }
        if !has_worktrees {
            content.push_str(".worktrees\n");
        }
        fs::write(&gitignore_path, content)?;
    }

    Ok(())
}

/// One repair `repair_workspace` actually applied, carrying the words
/// `check_all_issues` used for the issue it healed.
#[derive(Debug, Clone)]
pub struct AppliedRepair {
    pub description: String,
}

/// Run the full check set and apply every workspace repair that is safe to
/// run unattended. Prints NOTHING: an empty vector means the workspace
/// needed no unattended repair, and the caller renders the report. The
/// `HooksAndSettings` fix is the only variant with output of its own; it is
/// routed through `WorkspaceFix::apply(.., verbose: false)`, which drives it
/// through the quiet permission/skill-index variants instead.
pub fn repair_workspace(repo_root: &Path) -> Result<Vec<AppliedRepair>> {
    repair_workspace_from(repo_root, super::check_all_issues(repo_root))
}

/// Testing seam for `repair_workspace`, over a caller-supplied issue set
/// instead of `check_all_issues`.
///
/// For each issue: skip it if it is not a workspace-family fix, or if it is
/// one but not safe to run unattended. Otherwise apply it — `Ok(true)`
/// records an [`AppliedRepair`], `Ok(false)` records nothing, and `Err`
/// records nothing and moves on (a repair that could not be applied here is
/// not fatal to the caller; an explicit `loom repair --fix` still reports it
/// to a human).
pub(crate) fn repair_workspace_from(
    repo_root: &Path,
    issues: Vec<RepairIssue>,
) -> Result<Vec<AppliedRepair>> {
    let mut applied = Vec::new();

    for issue in &issues {
        let Some(fix) = WorkspaceFix::classify(&issue.description) else {
            continue;
        };
        if !fix.unattended() {
            continue;
        }
        if let Ok(true) = fix.apply(repo_root, false) {
            applied.push(AppliedRepair {
                description: issue.description.clone(),
            });
        }
    }

    Ok(applied)
}

/// The workspace-shape repairs, one variant per arm `fix_workspace_issue`
/// used to dispatch inline.
pub(super) enum WorkspaceFix {
    WorkSymlink,
    InvalidWork,
    GitignoreWork,
    GitignoreWorktrees,
    PreCommitHook,
    HooksAndSettings,
    HookScripts,
}

impl WorkspaceFix {
    /// The single substring matcher. `None` = not a workspace-family issue.
    pub(super) fn classify(description: &str) -> Option<Self> {
        if description.contains("is a symlink (->") {
            Some(Self::WorkSymlink)
        } else if description.contains("exists but is neither") {
            Some(Self::InvalidWork)
        } else if description.contains(".loom/work not found in .gitignore")
            || description.contains(".work not found in .gitignore")
        {
            Some(Self::GitignoreWork)
        } else if description.contains(".worktrees not found in .gitignore") {
            Some(Self::GitignoreWorktrees)
        } else if description.contains("pre-commit hook not installed") {
            Some(Self::PreCommitHook)
        } else if description.contains("Project .claude/settings.json incomplete")
            || description.contains("Hooks found in .claude/settings.json")
        {
            Some(Self::HooksAndSettings)
        } else if description.contains("Loom hook scripts")
            || description.contains("Codex hook installation")
        {
            Some(Self::HookScripts)
        } else {
            None
        }
    }

    /// Whether `loom init` may apply this without being asked. `InvalidWork`
    /// is the only `false`: its fix is a recursive delete of the state
    /// directory, keyed on shape detection a sibling plan stage rewrites, so
    /// it stays with an explicit `loom repair --fix`.
    fn unattended(&self) -> bool {
        !matches!(self, Self::InvalidWork)
    }

    /// `verbose` only affects `HooksAndSettings`, which is the sole variant
    /// with output of its own; every other arm ignores it.
    pub(super) fn apply(&self, repo_root: &Path, verbose: bool) -> Result<bool> {
        match self {
            Self::WorkSymlink => fix_work_symlink(repo_root).map(|()| true),
            Self::InvalidWork => fix_invalid_work(repo_root).map(|()| true),
            Self::GitignoreWork => fix_gitignore_work(repo_root).map(|()| true),
            Self::GitignoreWorktrees => fix_gitignore_worktrees(repo_root).map(|()| true),
            Self::PreCommitHook => crate::git::install_pre_commit_hook(repo_root).map(|_| true),
            Self::HooksAndSettings => super::hooks::fix_hooks(repo_root, verbose).map(|()| true),
            Self::HookScripts => super::hooks::install_hook_assets().map(|()| true),
        }
    }
}
