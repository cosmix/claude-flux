//! Install loom's embedded assets into the Claude and Codex configuration trees.

use anyhow::Result;
use colored::Colorize;
use std::path::{Path, PathBuf};

use crate::assets::install::{default_paths, install_all, InstallPaths, InstallReport};
use crate::skills::SkillLayout;

/// Resolve paths, choose the skill layout, install assets, and print a summary.
pub fn execute(
    claude_dir: Option<PathBuf>,
    codex_dir: Option<PathBuf>,
    skills: Option<SkillLayout>,
) -> Result<()> {
    let refresh_completions = claude_dir.is_none() && codex_dir.is_none();
    let paths = resolve_paths(claude_dir, codex_dir)?;
    let layout = resolve_layout(&paths.claude_dir, skills);
    let report = install_all(&paths, layout, refresh_completions)?;
    print_summary(&report);
    Ok(())
}

/// Fill in whichever directory was not passed explicitly, resolving the
/// shared default paths at most once rather than once per missing directory.
fn resolve_paths(claude_dir: Option<PathBuf>, codex_dir: Option<PathBuf>) -> Result<InstallPaths> {
    match (claude_dir, codex_dir) {
        (Some(claude_dir), Some(codex_dir)) => Ok(InstallPaths {
            claude_dir,
            codex_dir,
        }),
        (claude_dir, codex_dir) => {
            let defaults = default_paths()?;
            Ok(InstallPaths {
                claude_dir: claude_dir.unwrap_or(defaults.claude_dir),
                codex_dir: codex_dir.unwrap_or(defaults.codex_dir),
            })
        }
    }
}

/// Select an explicit layout or preserve the layout of an existing tree.
fn resolve_layout(claude_dir: &Path, skills: Option<SkillLayout>) -> Option<SkillLayout> {
    match skills {
        Some(layout) => Some(layout),
        None if !claude_dir.join("loom-install.toml").exists()
            && !claude_dir.join("skills").exists() =>
        {
            Some(SkillLayout::Core)
        }
        None => None,
    }
}

/// Print the installation counts and every backup path created by the placer.
fn print_summary(report: &InstallReport) {
    crate::utils::print_logo_header("Install Assets");
    let layout = match report.layout {
        SkillLayout::Core => "core",
        SkillLayout::All => "all",
    };
    println!("  {} agents: {}", "✓".green(), report.agents);
    println!("  {} commands: {}", "✓".green(), report.commands);
    println!("  {} Claude hooks updated: {}", "✓".green(), report.hooks);
    println!(
        "  {} Codex hooks updated: {} (review changes with /hooks)",
        "✓".green(),
        report.codex_hooks
    );
    println!(
        "  {} Claude skills: {} resident, {} catalogued ({layout})",
        "✓".green(),
        report.skills_resident,
        report.skills_catalogued
    );
    println!(
        "  {} Codex skills: {} resident, {} catalogued ({layout})",
        "✓".green(),
        report.codex_skills_resident,
        report.codex_skills_catalogued
    );
    for backup in &report.backups {
        println!("  {} backup: {}", "✓".green(), backup.display());
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_layout;
    use crate::skills::SkillLayout;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn resolve_layout_defaults_to_core_on_a_fresh_tree() {
        let temp = TempDir::new().unwrap();
        assert_eq!(resolve_layout(temp.path(), None), Some(SkillLayout::Core));
    }

    #[test]
    fn resolve_layout_returns_none_when_a_tree_already_exists() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("skills")).unwrap();
        assert_eq!(resolve_layout(temp.path(), None), None);
    }

    #[test]
    fn resolve_layout_honours_an_explicit_flag() {
        let temp = TempDir::new().unwrap();
        assert_eq!(
            resolve_layout(temp.path(), Some(SkillLayout::All)),
            Some(SkillLayout::All)
        );
    }
}
