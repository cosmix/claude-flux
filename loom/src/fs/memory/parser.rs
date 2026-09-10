//! Parsing and formatting functions for memory journal entries.

use super::types::{
    EntryBuilder, MemoryEntry, MemoryEntryType, MemoryJournal, Receipt, ReceiptOutcome,
};
use anyhow::Result;
use chrono::{DateTime, NaiveDateTime, Utc};

/// Parse a memory journal from markdown content.
pub fn parse_journal(content: &str, stage_id: &str) -> Result<MemoryJournal> {
    let mut journal = MemoryJournal {
        stage_id: stage_id.to_string(),
        ..Default::default()
    };
    let mut current_entry = None;

    for line in content.lines() {
        if line.starts_with("<!--") || line.starts_with("# Memory Journal") || line == "---" {
            finish_entry(&mut current_entry, &mut journal);
            continue;
        }
        if line.starts_with("**Stage**:") {
            journal.stage_id = line.trim_start_matches("**Stage**:").trim().to_string();
            continue;
        }
        if line.starts_with("## Summary") {
            finish_entry(&mut current_entry, &mut journal);
            continue;
        }
        if let Some(header) = line.strip_prefix("### ") {
            finish_entry(&mut current_entry, &mut journal);
            current_entry = parse_header(header);
            if current_entry.is_none() {
                tracing::debug!(header, "Skipping malformed memory journal entry heading");
            }
            continue;
        }

        if let Some(builder) = &mut current_entry {
            parse_entry_line(builder, line);
        }
    }

    finish_entry(&mut current_entry, &mut journal);
    Ok(journal)
}

fn finish_entry(current: &mut Option<EntryBuilder>, journal: &mut MemoryJournal) {
    let Some(builder) = current.take() else {
        return;
    };
    let id = builder.id.clone();
    match builder.build() {
        Some(entry) => journal.entries.push(entry),
        None => tracing::debug!(id, "Skipping malformed memory journal entry"),
    }
}

fn parse_header(header: &str) -> Option<EntryBuilder> {
    let (heading, id) = header.rsplit_once(" id=")?;
    if !is_event_id(id) {
        return None;
    }
    let timestamp_start = heading.rfind('[')?;
    let timestamp = heading.get(timestamp_start + 1..)?.strip_suffix(']')?;
    let timestamp = NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M:%S").ok()?;
    let type_name = heading.get(..timestamp_start)?.split_whitespace().last()?;
    let entry_type = type_name.parse::<MemoryEntryType>().ok()?;

    Some(EntryBuilder {
        id: id.to_string(),
        timestamp: DateTime::from_naive_utc_and_offset(timestamp, Utc),
        entry_type,
        content: String::new(),
        context: None,
        session: None,
        evidence: Vec::new(),
        receipt: None,
        in_metadata: false,
        valid: true,
    })
}

fn is_event_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn parse_entry_line(builder: &mut EntryBuilder, line: &str) {
    if let Some(context) = line.strip_prefix("*Context:*") {
        builder.context = Some(context.trim().to_string());
        builder.in_metadata = true;
    } else if let Some(evidence) = line.strip_prefix("*Evidence:*") {
        match parse_evidence(evidence.trim()) {
            Some(evidence) => builder.evidence = evidence,
            None => builder.valid = false,
        }
        builder.in_metadata = true;
    } else if let Some(session) = line.strip_prefix("*Session:*") {
        builder.session = Some(session.trim().to_string());
        builder.in_metadata = true;
    } else if let Some(receipt) = line.strip_prefix("*Receipt:*") {
        builder.receipt = parse_receipt(receipt.trim());
        builder.valid &= builder.receipt.is_some();
        builder.in_metadata = true;
    } else if !builder.in_metadata {
        if !builder.content.is_empty() {
            builder.content.push('\n');
        }
        builder.content.push_str(line);
    }
}

fn parse_evidence(value: &str) -> Option<Vec<String>> {
    if value.is_empty() {
        return Some(Vec::new());
    }
    let mut evidence = Vec::new();
    let mut remaining = value;
    loop {
        let quoted = remaining.strip_prefix('`')?;
        let end = quoted.find('`')?;
        let reference = quoted.get(..end)?;
        if reference.is_empty() {
            return None;
        }
        evidence.push(reference.to_string());
        remaining = quoted.get(end + 1..)?;
        if remaining.is_empty() {
            return Some(evidence);
        }
        remaining = remaining.strip_prefix(", ")?;
    }
}

fn parse_receipt(value: &str) -> Option<Receipt> {
    let value = value.strip_prefix("event=")?;
    let (event_id, value) = value.split_once(" outcome=")?;
    let (outcome, target) = value.split_once(" target=")?;
    Some(Receipt {
        event_id: event_id.to_string(),
        outcome: outcome.parse::<ReceiptOutcome>().ok()?,
        target: (target != "-").then(|| target.to_string()),
    })
}

/// Format a memory entry for markdown output.
pub fn format_entry(entry: &MemoryEntry) -> String {
    let mut output = format!(
        "### {} {} [{}] id={}\n\n{}\n\n",
        entry.entry_type.emoji(),
        entry.entry_type.display_name(),
        entry.timestamp.format("%Y-%m-%d %H:%M:%S"),
        entry.id,
        entry.content
    );
    let has_metadata = entry.context.is_some()
        || !entry.evidence.is_empty()
        || entry.session.is_some()
        || entry.receipt.is_some();

    if let Some(context) = &entry.context {
        output.push_str(&format!("*Context:* {context}\n"));
    }
    if !entry.evidence.is_empty() {
        let evidence = entry
            .evidence
            .iter()
            .map(|reference| format!("`{reference}`"))
            .collect::<Vec<_>>()
            .join(", ");
        output.push_str(&format!("*Evidence:* {evidence}\n"));
    }
    if let Some(session) = &entry.session {
        output.push_str(&format!("*Session:* {session}\n"));
    }
    if let Some(receipt) = &entry.receipt {
        let target = receipt.target.as_deref().unwrap_or("-");
        output.push_str(&format!(
            "*Receipt:* event={} outcome={} target={}\n",
            receipt.event_id, receipt.outcome, target
        ));
    }
    if has_metadata {
        output.push('\n');
    }
    output.push_str("---\n\n");
    output
}
