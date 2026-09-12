//! Builds the "Changes by Stage" section of the review document.

use std::collections::HashMap;

use crate::fs::memory::{MemoryEntry, MemoryEntryType};

use super::stages::{render_stage_section, StageInfo};

/// Append the "Changes by Stage" section: one subsection per stage with
/// recorded memory entries, skipping any stage that has none.
pub(super) fn append_changes_by_stage_section(
    doc: &mut String,
    stages: &[StageInfo],
    journals: &HashMap<String, Vec<MemoryEntry>>,
) {
    doc.push_str("## Changes by Stage\n\n");

    let mut has_any_stage = false;
    for stage in stages {
        let entries: Vec<&MemoryEntry> = journals
            .get(&stage.id)
            .map(|v| {
                v.iter()
                    .filter(|entry| entry.entry_type != MemoryEntryType::Receipt)
                    .collect()
            })
            .unwrap_or_default();

        // Skip stages with no memory entries
        if entries.is_empty() {
            continue;
        }

        has_any_stage = true;
        doc.push_str(&render_stage_section(stage, &entries));
    }

    if !has_any_stage {
        doc.push_str("No stage memory recorded.\n\n");
    }
}
