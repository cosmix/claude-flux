use crate::fs::memory::{MemoryEntry, MemoryEntryType};
use crate::parser::frontmatter::extract_frontmatter_field;
use anyhow::{Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Information extracted from a stage file.
pub(super) struct StageInfo {
    pub(super) id: String,
    pub(super) name: String,
    description: String,
    status: String,
    /// Original filename, used for sorting.
    _filename: String,
}

/// Load all stage files from `.loom/work/stages/` sorted by filename.
pub(super) fn load_stage_infos(stages_dir: &Path) -> Result<Vec<StageInfo>> {
    if !stages_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries: Vec<(String, PathBuf)> = fs::read_dir(stages_dir)
        .context("Failed to read stages directory")?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|ext| ext == "md")
        })
        .map(|e| {
            let filename = e.file_name().to_string_lossy().to_string();
            (filename, e.path())
        })
        .collect();

    // Sort by filename so depth-prefixed names come out in topological order
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    entries
        .into_iter()
        .map(|(filename, path)| load_stage(filename, &path))
        .collect()
}
fn load_stage(filename: String, path: &Path) -> Result<StageInfo> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read stage file: {}", path.display()))?;
    let id = extract_frontmatter_field(&content, "id")
        .ok()
        .flatten()
        .unwrap_or_else(|| filename.trim_end_matches(".md").to_string());

    let name = extract_frontmatter_field(&content, "name")
        .ok()
        .flatten()
        .unwrap_or_else(|| id.clone());

    let description = extract_frontmatter_field(&content, "description")
        .ok()
        .flatten()
        .unwrap_or_default();

    let status = extract_frontmatter_field(&content, "status")
        .ok()
        .flatten()
        .unwrap_or_else(|| "Unknown".to_string());

    Ok(StageInfo {
        id,
        name,
        description,
        status,
        _filename: filename,
    })
}

/// Format a single bullet point for a memory entry.
fn format_entry_bullet(entry: &MemoryEntry) -> String {
    match &entry.context {
        Some(ctx) => format!("- {} *({})*", entry.content, ctx),
        None => format!("- {}", entry.content),
    }
}

/// Render the "Changes by Stage" section for one stage.
pub(super) fn render_stage_section(stage: &StageInfo, entries: &[&MemoryEntry]) -> String {
    let mut out = String::new();

    out.push_str(&format!("### {} ({})\n\n", stage.name, stage.id));
    out.push_str(&format!("**Status:** {}  \n", stage.status));
    if !stage.description.is_empty() {
        out.push_str(&format!("**Purpose:** {}\n\n", stage.description));
    } else {
        out.push('\n');
    }

    append_entries(
        &mut out,
        entries,
        MemoryEntryType::Change,
        "Files Changed",
        "No changes recorded.",
        false,
    );
    out.push('\n');
    append_entries(
        &mut out,
        entries,
        MemoryEntryType::Decision,
        "Key Decisions",
        "No decisions recorded.",
        false,
    );
    out.push('\n');
    append_entries(
        &mut out,
        entries,
        MemoryEntryType::Note,
        "Notes",
        "No notes recorded.",
        true,
    );
    out.push('\n');
    out
}

fn append_entries(
    out: &mut String,
    entries: &[&MemoryEntry],
    kind: MemoryEntryType,
    heading: &str,
    empty: &str,
    recent: bool,
) {
    out.push_str(&format!("#### {heading}\n\n"));
    let mut selected: Vec<_> = entries
        .iter()
        .copied()
        .filter(|e| e.entry_type == kind)
        .collect();
    if recent {
        selected.reverse();
        selected.truncate(10);
    }
    if selected.is_empty() {
        out.push_str(&format!("{empty}\n"));
    }
    for entry in selected {
        out.push_str(&format!("{}\n", format_entry_bullet(entry)));
    }
}
