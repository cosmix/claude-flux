//! File storage operations for memory journals.

use super::constants::MEMORY_HEADER;
use super::parser::{format_entry, parse_journal};
use super::types::{MemoryEntry, MemoryJournal};
use anyhow::{Context, Result};
use chrono::Utc;
use fs2::FileExt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Get the memory directory path
pub fn memory_dir(work_dir: &Path) -> PathBuf {
    work_dir.join("memory")
}

/// Get the path to a stage's memory file
pub fn memory_file_path(work_dir: &Path, stage_id: &str) -> PathBuf {
    memory_dir(work_dir).join(format!("{stage_id}.md"))
}

/// Initialize the memory directory
pub fn init_memory_dir(work_dir: &Path) -> Result<()> {
    let memory_path = memory_dir(work_dir);

    if !memory_path.exists() {
        fs::create_dir_all(&memory_path).with_context(|| {
            format!(
                "Failed to create memory directory: {}",
                memory_path.display()
            )
        })?;
    }

    Ok(())
}

/// Create a new memory journal for a stage
pub fn create_journal(work_dir: &Path, stage_id: &str) -> Result<PathBuf> {
    init_memory_dir(work_dir)?;

    let file_path = memory_file_path(work_dir, stage_id);
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&file_path)
        .with_context(|| format!("Failed to create memory journal: {}", file_path.display()))?;
    file.lock_exclusive()
        .with_context(|| format!("Failed to lock memory journal: {}", file_path.display()))?;
    file.set_len(0)
        .with_context(|| format!("Failed to truncate memory journal: {}", file_path.display()))?;
    file.write_all(journal_header(stage_id).as_bytes())
        .with_context(|| format!("Failed to create memory journal: {}", file_path.display()))?;

    Ok(file_path)
}

/// Append an entry to a stage's memory journal
pub fn append_entry(work_dir: &Path, stage_id: &str, entry: &MemoryEntry) -> Result<()> {
    init_memory_dir(work_dir)?;
    let file_path = memory_file_path(work_dir, stage_id);
    let formatted = format_entry(entry);

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)
        .with_context(|| format!("Failed to open memory journal: {}", file_path.display()))?;
    file.lock_exclusive()
        .with_context(|| format!("Failed to lock memory journal: {}", file_path.display()))?;

    if file
        .metadata()
        .with_context(|| format!("Failed to stat memory journal: {}", file_path.display()))?
        .len()
        == 0
    {
        file.write_all(journal_header(stage_id).as_bytes())
            .with_context(|| {
                format!(
                    "Failed to initialize memory journal: {}",
                    file_path.display()
                )
            })?;
    }

    file.write_all(formatted.as_bytes()).with_context(|| {
        format!(
            "Failed to append to memory journal: {}",
            file_path.display()
        )
    })?;

    Ok(())
}

fn journal_header(stage_id: &str) -> String {
    format!(
        "{MEMORY_HEADER}# Memory Journal: {stage_id}\n\n**Stage**: {stage_id}\n**Created**: {}\n\n---\n\n",
        Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    )
}

/// Read a stage's memory journal
pub fn read_journal(work_dir: &Path, stage_id: &str) -> Result<MemoryJournal> {
    let file_path = memory_file_path(work_dir, stage_id);

    if !file_path.exists() {
        return Ok(MemoryJournal {
            stage_id: stage_id.to_string(),
            ..Default::default()
        });
    }

    let content = fs::read_to_string(&file_path)
        .with_context(|| format!("Failed to read memory journal: {}", file_path.display()))?;

    parse_journal(&content, stage_id)
}

/// Write summary to the journal file
pub fn write_summary(work_dir: &Path, stage_id: &str, summary: &str) -> Result<()> {
    let file_path = memory_file_path(work_dir, stage_id);

    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&file_path)
        .with_context(|| format!("Failed to open memory journal: {}", file_path.display()))?;

    file.write_all(summary.as_bytes()).with_context(|| {
        format!(
            "Failed to write summary to memory journal: {}",
            file_path.display()
        )
    })?;

    Ok(())
}
