//! Listing unresolved memory events across the current plan.

use anyhow::Result;
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

use crate::fs::memory::{list_journals, MemoryEntry, MemoryEntryType};

use super::read::{read_journal_with_pending, spool_only_stage_with_pending};
use super::work_dir::{readonly_work_dir, validate_stage_id};

#[derive(Debug, Clone, Serialize)]
pub(super) struct StagedMemoryEntry {
    pub(super) stage: String,
    #[serde(flatten)]
    pub(super) entry: MemoryEntry,
}

#[derive(Debug, Serialize)]
pub(super) struct PendingReport {
    pub(super) pending: Vec<StagedMemoryEntry>,
    pub(super) changes_without_receipt: usize,
    pub(super) receipts: usize,
    #[serde(skip)]
    pub(super) journals: usize,
}

/// Print unresolved notes, decisions, and questions.
pub fn pending(stage_id: Option<String>, json: bool, strict: bool) -> Result<()> {
    if let Some(ref stage) = stage_id {
        validate_stage_id(stage)?;
    }

    let Some(work_dir) = readonly_work_dir() else {
        print_no_journals(json);
        return Ok(());
    };
    let report = pending_report(&work_dir, stage_id.as_deref())?;
    if report.journals == 0 {
        print_no_journals(json);
        return Ok(());
    }

    if json {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        print_human_report(&report);
    }

    if strict_should_fail(strict, report.pending.len()) {
        std::process::exit(1);
    }
    Ok(())
}

fn print_no_journals(json: bool) {
    if json {
        println!(r#"{{"pending":[]}}"#);
    } else {
        println!("no memory journals");
    }
}

fn print_human_report(report: &PendingReport) {
    for pending in &report.pending {
        let preview: String = pending
            .entry
            .content
            .chars()
            .map(|character| {
                if character.is_whitespace() {
                    ' '
                } else {
                    character
                }
            })
            .take(80)
            .collect();
        println!(
            "{}  {}  {}  {}",
            pending.entry.id, pending.entry.entry_type, pending.stage, preview
        );
    }
    println!(
        "{} pending across {} journals",
        report.pending.len(),
        report.journals
    );
    println!(
        "changes without receipts: {}",
        report.changes_without_receipt
    );
}

pub(super) fn strict_should_fail(strict: bool, pending_count: usize) -> bool {
    strict && pending_count > 0
}

pub(super) fn pending_report(work_dir: &Path, stage_id: Option<&str>) -> Result<PendingReport> {
    let (entries, journals) = staged_entries(work_dir)?;
    let settled_ids: HashSet<String> = entries
        .iter()
        .filter_map(|staged| staged.entry.receipt.as_ref())
        .map(|receipt| receipt.event_id.clone())
        .collect();
    let receipts = entries
        .iter()
        .filter(|staged| staged.entry.entry_type == MemoryEntryType::Receipt)
        .count();
    let in_scope =
        |staged: &StagedMemoryEntry| stage_id.is_none_or(|stage| staged.stage.as_str() == stage);
    let changes_without_receipt = entries
        .iter()
        .filter(|staged| {
            in_scope(staged)
                && staged.entry.entry_type == MemoryEntryType::Change
                && !settled_ids.contains(&staged.entry.id)
        })
        .count();
    let pending = entries
        .into_iter()
        .filter(|staged| {
            in_scope(staged)
                && matches!(
                    staged.entry.entry_type,
                    MemoryEntryType::Note | MemoryEntryType::Decision | MemoryEntryType::Question
                )
                && !settled_ids.contains(&staged.entry.id)
        })
        .collect();

    let journals = match stage_id {
        Some(stage) if journals.iter().any(|journal| journal == stage) => 1,
        Some(_) => 0,
        None => journals.len(),
    };
    Ok(PendingReport {
        pending,
        changes_without_receipt,
        receipts,
        journals,
    })
}

/// Read every journal once, adding only the current worktree's undrained spool.
pub(super) fn staged_entries(work_dir: &Path) -> Result<(Vec<StagedMemoryEntry>, Vec<String>)> {
    let mut journals = list_journals(work_dir)?;
    if let Some(stage) = spool_only_stage_with_pending(&journals) {
        journals.push(stage);
    }
    journals.sort();
    journals.dedup();

    let mut seen_ids = HashSet::new();
    let mut entries = Vec::new();
    for stage in &journals {
        let journal = read_journal_with_pending(work_dir, stage)?;
        entries.extend(
            journal
                .entries
                .into_iter()
                .filter(|entry| seen_ids.insert(entry.id.clone()))
                .map(|entry| StagedMemoryEntry {
                    stage: stage.clone(),
                    entry,
                }),
        );
    }
    Ok((entries, journals))
}
