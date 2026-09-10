//! Output formatting utilities for memory commands.

use colored::Colorize;

use crate::commands::common::truncate_for_display;
use crate::fs::memory::MemoryEntry;

/// Format a single entry for list/query display (compact format)
pub fn format_entry_compact(entry: &MemoryEntry) -> String {
    let time = entry.timestamp.format("%H:%M:%S").to_string();
    let type_emoji = entry.entry_type.emoji();

    let main_line = format!(
        "{} {} {} {}",
        time.dimmed(),
        type_emoji,
        entry.entry_type.display_name().cyan(),
        truncate_for_display(&entry.content, 50)
    );

    let mut output = if let Some(ctx) = &entry.context {
        format!(
            "{}\n  {} {}",
            main_line,
            "→".dimmed(),
            truncate_for_display(ctx, 48).yellow()
        )
    } else {
        main_line
    };
    if let Some(receipt) = &entry.receipt {
        output.push_str(&format!(
            "\n  {} event={} outcome={} target={}",
            "→".dimmed(),
            receipt.event_id,
            receipt.outcome,
            receipt.target.as_deref().unwrap_or("-")
        ));
    }
    output
}

/// Format a single entry for show display (full format)
pub fn format_entry_full(entry: &MemoryEntry) -> String {
    let time = entry.timestamp.format("%Y-%m-%d %H:%M:%S").to_string();
    let type_emoji = entry.entry_type.emoji();

    let mut output = format!(
        "\n{} {} {}\n{}\n{}",
        type_emoji,
        entry.entry_type.display_name().bold(),
        time.dimmed(),
        "─".repeat(40),
        entry.content
    );

    if let Some(ctx) = &entry.context {
        output.push_str(&format!("\n\n{} {}", "Context:".cyan(), ctx));
    }
    output.push_str(&format!("\n\n{} {}", "ID:".cyan(), entry.id));
    if !entry.evidence.is_empty() {
        output.push_str(&format!(
            "\n{} {}",
            "Evidence:".cyan(),
            entry.evidence.join(", ")
        ));
    }
    if let Some(session) = &entry.session {
        output.push_str(&format!("\n{} {}", "Session:".cyan(), session));
    }
    if let Some(receipt) = &entry.receipt {
        output.push_str(&format!(
            "\n{} event={} outcome={} target={}",
            "Receipt:".cyan(),
            receipt.event_id,
            receipt.outcome,
            receipt.target.as_deref().unwrap_or("-")
        ));
    }

    output
}

/// Format a success message for recording an entry
pub fn format_record_success(entry: &MemoryEntry, stage_id: &str) -> String {
    format!(
        "✓ Recorded {} {} for stage {}\n  {}",
        entry.entry_type,
        entry.id,
        stage_id,
        truncate_for_display(&entry.content, 60)
    )
}
