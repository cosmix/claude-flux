//! Export and formatting functions for memory journal content.

use super::query::get_recent_entries;
use super::storage::read_journal;
use super::types::{MemoryEntry, MemoryEntryType};
use crate::utils::truncate_for_display;
use std::path::Path;

/// Format memory entries for embedding in a signal
pub fn format_memory_for_signal(
    work_dir: &Path,
    stage_id: &str,
    max_entries: usize,
) -> Option<String> {
    let journal = read_journal(work_dir, stage_id).ok()?;

    if journal.entries.is_empty() {
        return None;
    }

    let recent = get_recent_entries(&journal, max_entries);
    if recent.is_empty() {
        return None;
    }

    let mut output = String::new();

    // Group by type for better organization
    let notes: Vec<_> = recent
        .iter()
        .filter(|e| e.entry_type == MemoryEntryType::Note)
        .collect();
    let decisions: Vec<_> = recent
        .iter()
        .filter(|e| e.entry_type == MemoryEntryType::Decision)
        .collect();
    let questions: Vec<_> = recent
        .iter()
        .filter(|e| e.entry_type == MemoryEntryType::Question)
        .collect();
    let changes: Vec<_> = recent
        .iter()
        .filter(|e| e.entry_type == MemoryEntryType::Change)
        .collect();

    if !notes.is_empty() {
        output.push_str("### Notes\n\n");
        for entry in notes {
            output.push_str(&format!(
                "- **[{}]** {}\n",
                entry.timestamp.format("%H:%M"),
                truncate_for_display(&entry.content, 150)
            ));
        }
        output.push('\n');
    }

    if !decisions.is_empty() {
        output.push_str("### Decisions\n\n");
        for entry in decisions {
            output.push_str(&format!(
                "- **[{}]** {}\n",
                entry.timestamp.format("%H:%M"),
                truncate_for_display(&entry.content, 150)
            ));
            if let Some(ctx) = &entry.context {
                output.push_str(&format!(
                    "  - *Rationale:* {}\n",
                    truncate_for_display(ctx, 100)
                ));
            }
        }
        output.push('\n');
    }

    if !questions.is_empty() {
        output.push_str("### Open Questions\n\n");
        for entry in questions {
            output.push_str(&format!(
                "- **[{}]** {}\n",
                entry.timestamp.format("%H:%M"),
                truncate_for_display(&entry.content, 150)
            ));
        }
        output.push('\n');
    }

    if !changes.is_empty() {
        output.push_str("### Changes\n\n");
        for entry in changes {
            output.push_str(&format!(
                "- **[{}]** {}\n",
                entry.timestamp.format("%H:%M"),
                truncate_for_display(&entry.content, 150)
            ));
        }
        output.push('\n');
    }

    Some(output)
}

/// Format memory for inclusion in handoff file
pub fn format_memory_for_handoff(work_dir: &Path, stage_id: &str) -> Option<String> {
    let journal = read_journal(work_dir, stage_id).ok()?;
    let entries: Vec<_> = journal
        .entries
        .iter()
        .filter(|entry| entry.entry_type != MemoryEntryType::Receipt)
        .collect();

    if entries.is_empty() {
        return None;
    }

    let mut output = String::new();
    output.push_str("## Stage Memory\n\n");
    output.push_str(&format!(
        "Memory journal from stage {} ({} entries).\n\n",
        stage_id,
        entries.len()
    ));

    // Include all decisions, questions, and changes (they're important for handoffs)
    let decisions: Vec<_> = entries
        .iter()
        .copied()
        .filter(|e| e.entry_type == MemoryEntryType::Decision)
        .collect();
    let questions: Vec<_> = entries
        .iter()
        .copied()
        .filter(|e| e.entry_type == MemoryEntryType::Question)
        .collect();
    let changes: Vec<_> = entries
        .iter()
        .copied()
        .filter(|e| e.entry_type == MemoryEntryType::Change)
        .collect();

    if !decisions.is_empty() {
        output.push_str("### Decisions Made\n\n");
        for entry in &decisions {
            output.push_str(&format!(
                "- **[{}]** {}\n",
                entry.timestamp.format("%H:%M"),
                entry.content
            ));
            if let Some(ctx) = &entry.context {
                output.push_str(&format!("  - *Rationale:* {ctx}\n"));
            }
        }
        output.push('\n');
    }

    if !questions.is_empty() {
        output.push_str("### Open Questions\n\n");
        for entry in &questions {
            output.push_str(&format!(
                "- **[{}]** {}\n",
                entry.timestamp.format("%H:%M"),
                entry.content
            ));
        }
        output.push('\n');
    }

    if !changes.is_empty() {
        output.push_str("### Files Changed\n\n");
        for entry in &changes {
            output.push_str(&format!(
                "- **[{}]** {}\n",
                entry.timestamp.format("%H:%M"),
                entry.content
            ));
        }
        output.push('\n');
    }

    append_recent_notes(&mut output, &entries);

    Some(output)
}

/// Append the "Recent Notes" section: the last 5 note entries, most recent first.
fn append_recent_notes(output: &mut String, entries: &[&MemoryEntry]) {
    let notes: Vec<_> = entries
        .iter()
        .copied()
        .filter(|e| e.entry_type == MemoryEntryType::Note)
        .collect();
    if !notes.is_empty() {
        output.push_str("### Recent Notes\n\n");
        for entry in notes.iter().rev().take(5) {
            output.push_str(&format!(
                "- **[{}]** {}\n",
                entry.timestamp.format("%H:%M"),
                truncate_for_display(&entry.content, 200)
            ));
        }
        output.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_for_display_short() {
        let result = truncate_for_display("hello world", 20);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_truncate_for_display_exact() {
        let result = truncate_for_display("hello", 5);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_truncate_for_display_long() {
        let result = truncate_for_display("hello world", 8);
        assert_eq!(result, "hello w…");
    }

    #[test]
    fn test_truncate_for_display_multiline() {
        let result = truncate_for_display("hello\nworld", 20);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_truncate_for_display_utf8_emoji() {
        // Emoji are multi-byte (4 bytes each)
        // "🎉🎊🎁" = 3 chars but 12 bytes
        let input = "🎉🎊🎁🎈🎂";
        let result = truncate_for_display(input, 4);
        // Should truncate to 3 chars + ellipsis
        assert_eq!(result, "🎉🎊🎁…");
        // Verify no panic on multi-byte boundary
    }

    #[test]
    fn test_truncate_for_display_utf8_cjk() {
        // CJK characters are 3 bytes each
        let input = "你好世界";
        let result = truncate_for_display(input, 3);
        assert_eq!(result, "你好…");
    }

    #[test]
    fn test_truncate_for_display_utf8_mixed() {
        // Mix ASCII and multi-byte
        // "hello🎉world" = 11 chars (5 ASCII + 1 emoji + 5 ASCII)
        let input = "hello🎉world";
        let result = truncate_for_display(input, 8);
        // max=8: take 7 chars + ellipsis = "hello🎉w…"
        assert_eq!(result, "hello🎉w…");
    }
}
