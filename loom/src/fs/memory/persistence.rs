//! Persistence operations for memory journals (listing, archiving).

use super::storage::{memory_dir, memory_file_path};
use super::types::MemoryEntryType;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// List all memory journals in the work directory
pub fn list_journals(work_dir: &Path) -> Result<Vec<String>> {
    let memory_path = memory_dir(work_dir);

    if !memory_path.exists() {
        return Ok(Vec::new());
    }

    let mut journals = Vec::new();

    for entry in fs::read_dir(&memory_path)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "md") {
            if let Some(stem) = path.file_stem() {
                journals.push(stem.to_string_lossy().to_string());
            }
        }
    }

    Ok(journals)
}

/// Copy memory journal to crash recovery location
pub fn preserve_for_crash(work_dir: &Path, stage_id: &str) -> Result<Option<PathBuf>> {
    let source = memory_file_path(work_dir, stage_id);

    if !source.exists() {
        return Ok(None);
    }

    let crashes_dir = work_dir.join("crashes");
    if !crashes_dir.exists() {
        fs::create_dir_all(&crashes_dir)?;
    }

    let dest = crashes_dir.join(format!("memory-{stage_id}.md"));
    fs::copy(&source, &dest)
        .with_context(|| format!("Failed to preserve memory for crash: {}", source.display()))?;

    Ok(Some(dest))
}

/// Extract key notes from memory for review on completion
pub fn extract_key_notes(journal: &super::types::MemoryJournal) -> Vec<String> {
    let mut key_notes = Vec::new();

    // Extract all decisions as key notes
    for entry in &journal.entries {
        if entry.entry_type == MemoryEntryType::Decision {
            let note = if let Some(ctx) = &entry.context {
                format!("{} ({})", entry.content, ctx)
            } else {
                entry.content.clone()
            };
            key_notes.push(note);
        }
    }

    // Extract all changes as key notes (important for tracking work done)
    for entry in &journal.entries {
        if entry.entry_type == MemoryEntryType::Change {
            key_notes.push(entry.content.clone());
        }
    }

    key_notes
}

/// Validate memory entry content
pub fn validate_content(content: &str) -> Result<()> {
    if content.is_empty() {
        bail!("Memory entry content cannot be empty");
    }

    if content.len() > 2000 {
        bail!(
            "Memory entry content too long: {} characters (max 2000)",
            content.len()
        );
    }

    Ok(())
}

/// Validate evidence references attached to a memory entry.
pub fn validate_evidence(evidence: &[String]) -> Result<()> {
    if evidence.len() > 16 {
        bail!(
            "Too many memory evidence references: {} (max 16)",
            evidence.len()
        );
    }

    for reference in evidence {
        if reference.is_empty() {
            bail!("Memory evidence reference cannot be empty");
        }
        let char_count = reference.chars().count();
        if char_count > 256 {
            bail!(
                "Memory evidence reference too long: {} characters (max 256)",
                char_count
            );
        }
        if reference.contains('`') {
            bail!("Memory evidence reference cannot contain a backtick (`): {reference}");
        }
        if reference.contains('\n') {
            bail!("Memory evidence reference cannot contain a newline: {reference}");
        }
    }

    Ok(())
}

#[cfg(test)]
#[path = "tests/persistence.rs"]
mod tests;
