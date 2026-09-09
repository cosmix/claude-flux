use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use super::build_index;

/// Refresh each client's keyword index without crossing their skill roots.
pub fn execute() -> Result<()> {
    execute_default(true)
}

pub fn execute_quiet() -> Result<()> {
    execute_default(false)
}

fn execute_default(verbose: bool) -> Result<()> {
    let home = dirs::home_dir().context("Cannot determine home directory")?;
    let codex = std::env::var_os("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    execute_in_agent_dir(&home.join(".claude"), verbose)?;
    execute_in_agent_dir(&codex, verbose)
}

pub fn execute_in_agent_dir(root: &Path, verbose: bool) -> Result<()> {
    let skills_dir = root.join("skills");
    let catalog_dir = crate::skills::catalog_dir_for(&skills_dir);
    if !skills_dir.is_dir() && !catalog_dir.is_dir() {
        return Ok(());
    }
    let (index, count) = build_index(&[&skills_dir, &catalog_dir])?;
    let output_dir = root.join("hooks/loom");
    let output_file = output_dir.join("skill-keywords.json");
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("Failed to create {}", output_dir.display()))?;
    let json = serde_json::to_string_pretty(&index)?;
    fs::write(&output_file, json)
        .with_context(|| format!("Failed to write {}", output_file.display()))?;
    if verbose {
        println!(
            "Built {} skill keywords from {count} skills in {}",
            index.len(),
            root.display()
        );
    }
    Ok(())
}
