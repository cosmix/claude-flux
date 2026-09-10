use anyhow::Result;
use colored::Colorize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::fs::memory::{
    list_journals, query_entries, read_journal, read_pending, MemoryEntry, MemoryEntryType,
    MemoryJournal,
};
use crate::git::worktree::find_worktree_root_from_cwd;

use super::super::formatters::{format_entry_compact, format_entry_full};
use super::work_dir::{readonly_work_dir, validate_stage_id};

fn current_worktree_stage() -> Option<(PathBuf, String)> {
    let cwd = std::env::current_dir().ok()?;
    let worktree_root = find_worktree_root_from_cwd(&cwd)?;
    let stage = worktree_root.file_name()?.to_str()?.to_string();
    Some((worktree_root, stage))
}

pub(super) fn read_journal_with_pending(work_dir: &Path, stage: &str) -> Result<MemoryJournal> {
    let mut journal = read_journal(work_dir, stage)?;

    let Some((worktree_root, worktree_stage)) = current_worktree_stage() else {
        return Ok(journal);
    };
    if worktree_stage != stage {
        return Ok(journal);
    }

    if let Ok(pending) = read_pending(&worktree_root) {
        journal.entries.extend(pending);
        journal.entries.sort_by_key(|entry| entry.timestamp);
    }

    Ok(journal)
}

pub(super) fn spool_only_stage_with_pending(journals: &[String]) -> Option<String> {
    let (worktree_root, stage) = current_worktree_stage()?;
    if journals.contains(&stage) {
        return None;
    }
    let pending = read_pending(&worktree_root).ok()?;
    if pending.is_empty() {
        return None;
    }
    Some(stage)
}

pub fn query(search: String, stage_id: Option<String>) -> Result<()> {
    if let Some(ref id) = stage_id {
        validate_stage_id(id)?;
    }

    let Some(work_dir) = readonly_work_dir() else {
        println!(
            "{} No memory recorded yet (no state directory found)",
            "ℹ".blue()
        );
        return Ok(());
    };

    let stages_to_search: Vec<String> = match stage_id {
        Some(id) => vec![id],
        None => list_journals(&work_dir)?,
    };

    if stages_to_search.is_empty() {
        println!("{} No memory journals found", "ℹ".blue());
        return Ok(());
    }

    let mut total_results = 0;
    for stage in &stages_to_search {
        total_results += query_stage(&work_dir, stage, &search)?;
    }

    if total_results == 0 {
        println!(
            "{} No entries found matching '{}'",
            "ℹ".blue(),
            search.cyan()
        );
    } else {
        println!("\n{} {} total results", "Found".bold(), total_results);
    }

    Ok(())
}

fn query_stage(work_dir: &Path, stage: &str, search: &str) -> Result<usize> {
    let journal = read_journal_with_pending(work_dir, stage)?;
    let results = query_entries(&journal, search);

    if results.is_empty() {
        return Ok(0);
    }

    let count = results.len();
    println!("\n{} ({})", stage.bold(), count);
    println!("{}", "─".repeat(60));

    for entry in &results {
        println!("{}", format_entry_compact(entry));
    }

    Ok(count)
}

fn print_journal_entries(
    work_dir: &Path,
    stage: &str,
    type_filter: Option<MemoryEntryType>,
    limit: usize,
) -> Result<usize> {
    let journal = read_journal_with_pending(work_dir, stage)?;

    let entries: Vec<_> = journal
        .entries
        .iter()
        .filter(|e| type_filter.is_none_or(|t| e.entry_type == t))
        .collect();

    if entries.is_empty() {
        return Ok(0);
    }

    println!(
        "\n{} ({} {})",
        stage.bold(),
        entries.len(),
        if entries.len() == 1 {
            "entry"
        } else {
            "entries"
        }
    );
    println!("{}", "─".repeat(60));

    for entry in entries.iter().rev().take(limit) {
        println!("{}", format_entry_compact(entry));
    }

    if entries.len() > limit {
        println!("  {} {} more...", "...".dimmed(), entries.len() - limit);
    }

    Ok(entries.len())
}

pub fn list(stage_id: Option<String>, entry_type: Option<String>, json: bool) -> Result<()> {
    if let Some(ref id) = stage_id {
        validate_stage_id(id)?;
    }

    let Some(work_dir) = readonly_work_dir() else {
        if json {
            println!("[]");
            return Ok(());
        }
        println!(
            "{} No memory recorded yet (no state directory found)",
            "ℹ".blue()
        );
        return Ok(());
    };
    let type_filter: Option<MemoryEntryType> = entry_type.map(|t| t.parse()).transpose()?;

    if json {
        println!(
            "{}",
            list_json_output(&work_dir, stage_id.as_deref(), type_filter)?
        );
        return Ok(());
    }

    if let Some(stage) = stage_id {
        return list_single_stage(&work_dir, &stage, type_filter);
    }

    list_all_stages(&work_dir, type_filter)
}

pub(super) fn list_json_output(
    work_dir: &Path,
    stage_id: Option<&str>,
    type_filter: Option<MemoryEntryType>,
) -> Result<String> {
    let mut journals = match stage_id {
        Some(stage) => vec![stage.to_string()],
        None => list_journals(work_dir)?,
    };
    if stage_id.is_none() {
        if let Some(stage) = spool_only_stage_with_pending(&journals) {
            journals.push(stage);
        }
        journals.sort();
    }

    let mut entries = Vec::new();
    for stage in journals {
        let journal = read_journal_with_pending(work_dir, &stage)?;
        entries.extend(
            journal
                .entries
                .into_iter()
                .filter(|entry| type_filter.is_none_or(|kind| entry.entry_type == kind)),
        );
    }
    Ok(serde_json::to_string(&entries)?)
}

fn list_single_stage(
    work_dir: &Path,
    stage: &str,
    type_filter: Option<MemoryEntryType>,
) -> Result<()> {
    let shown = print_journal_entries(work_dir, stage, type_filter, 20)?;
    if shown == 0 {
        println!(
            "{} No {} entries in memory journal for stage '{}'",
            "ℹ".blue(),
            type_filter
                .map(|t| t.to_string())
                .unwrap_or_else(|| "matching".to_string()),
            stage
        );
    }
    Ok(())
}

fn list_all_stages(work_dir: &Path, type_filter: Option<MemoryEntryType>) -> Result<()> {
    let mut journals = list_journals(work_dir)?;
    if let Some(spool_only_stage) = spool_only_stage_with_pending(&journals) {
        journals.push(spool_only_stage);
    }

    if journals.is_empty() {
        println!("{} No memory journals found", "ℹ".blue());
        return Ok(());
    }
    journals.sort();

    let current_stage = std::env::var("LOOM_STAGE_ID").ok();
    println!(
        "{} Plan Memory — {} journal{}",
        "📚".bold(),
        journals.len(),
        if journals.len() == 1 { "" } else { "s" }
    );
    if let Some(ref cur) = current_stage {
        println!("{} {}", "Current stage:".dimmed(), cur.cyan());
    }

    let mut total_shown = 0;
    for stage_name in &journals {
        total_shown += print_journal_entries(work_dir, stage_name, type_filter, 20)?;
    }

    if total_shown == 0 {
        println!(
            "\n{} No {} entries found across {} journal(s)",
            "ℹ".blue(),
            type_filter
                .map(|t| t.to_string())
                .unwrap_or_else(|| "matching".to_string()),
            journals.len()
        );
    } else {
        println!(
            "\n{} {} entr{} across {} journal{}",
            "Total:".bold(),
            total_shown,
            if total_shown == 1 { "y" } else { "ies" },
            journals.len(),
            if journals.len() == 1 { "" } else { "s" }
        );
    }

    Ok(())
}

pub fn show(stage_id: Option<String>, all: bool, json: bool) -> Result<()> {
    let Some(work_dir) = readonly_work_dir() else {
        if json {
            println!("{}", if all { "{}" } else { "[]" });
            return Ok(());
        }
        println!(
            "{} No memory recorded yet (no state directory found)",
            "ℹ".blue()
        );
        return Ok(());
    };

    if all {
        if json {
            println!("{}", show_all_json_output(&work_dir)?);
            return Ok(());
        }
        return show_all_journals(&work_dir);
    }

    let stage = match stage_id {
        Some(id) => id,
        None => std::env::var("LOOM_STAGE_ID")
            .map_err(|_| anyhow::anyhow!("No stage ID provided or detected. Use --stage <id>"))?,
    };
    validate_stage_id(&stage)?;

    if json {
        let entries = read_journal_with_pending(&work_dir, &stage)?.entries;
        println!("{}", serde_json::to_string(&entries)?);
        return Ok(());
    }

    show_single_journal(&work_dir, &stage)
}

fn show_all_json_output(work_dir: &Path) -> Result<String> {
    let mut journals = list_journals(work_dir)?;
    if let Some(stage) = spool_only_stage_with_pending(&journals) {
        journals.push(stage);
    }
    journals.sort();

    let mut entries_by_stage: BTreeMap<String, Vec<MemoryEntry>> = BTreeMap::new();
    for stage in journals {
        let entries = read_journal_with_pending(work_dir, &stage)?.entries;
        entries_by_stage.insert(stage, entries);
    }
    Ok(serde_json::to_string(&entries_by_stage)?)
}

fn show_all_journals(work_dir: &Path) -> Result<()> {
    let mut journals = list_journals(work_dir)?;
    if let Some(spool_only_stage) = spool_only_stage_with_pending(&journals) {
        journals.push(spool_only_stage);
    }

    if journals.is_empty() {
        println!("{} No memory journals found", "ℹ".blue());
        return Ok(());
    }
    for stage_name in &journals {
        let journal = read_journal_with_pending(work_dir, stage_name)?;
        if journal.entries.is_empty() {
            continue;
        }
        println!("{}", "═".repeat(60));
        println!("{}", format!("Memory Journal: {stage_name}").bold());
        println!("{} entries", journal.entries.len());
        println!("{}", "═".repeat(60));
        for entry in &journal.entries {
            println!("{}", format_entry_full(entry));
        }
        println!();
    }
    Ok(())
}

fn show_single_journal(work_dir: &Path, stage: &str) -> Result<()> {
    let journal = read_journal_with_pending(work_dir, stage)?;

    if journal.entries.is_empty() {
        println!(
            "{} No entries in memory journal for stage '{}'",
            "ℹ".blue(),
            stage
        );
        return Ok(());
    }

    println!("{}", "═".repeat(60));
    println!("{}", format!("Memory Journal: {stage}").bold());
    println!("{} {}", "Stage:".dimmed(), journal.stage_id);
    println!("{} entries", journal.entries.len());
    println!("{}", "═".repeat(60));

    for entry in &journal.entries {
        println!("{}", format_entry_full(entry));
    }

    println!("\n{}", "═".repeat(60));

    Ok(())
}
