//! Shared review generation for the CLI and plan finalization.
use crate::fs::memory::{list_journals, read_journal, MemoryEntry, MemoryEntryType};
use crate::fs::work_dir::load_config;
use anyhow::{Context, Result};
use changes_section::append_changes_by_stage_section;
use chrono::Utc;
use stages::{load_stage_infos, StageInfo};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
mod changes_section;
mod stages;
mod summary;

/// Generate a review in an explicit checkout. Automatic finalization never requests AI.
pub fn generate(
    work_dir: &Path,
    project_root: &Path,
    output_root: &Path,
    ai_summary: bool,
) -> Result<PathBuf> {
    let config = load_config(work_dir)?.context("No active plan. Run 'loom init' first.")?;
    let plan_id = config.plan_id().unwrap_or("unknown");
    let plan_name = config.get_plan_str("plan_name").unwrap_or(plan_id);
    let description = plan_description(config.source_path(), project_root, ai_summary);
    let stages = load_stage_infos(&work_dir.join("stages"))?;
    let journals = load_journals(work_dir)?;
    let doc = render(plan_id, plan_name, &description, &stages, &journals);
    let plans_dir = output_root.join("doc/plans");
    fs::create_dir_all(&plans_dir).context("Failed to create doc/plans directory")?;
    let output = plans_dir.join(format!("REVIEW-{plan_id}.md"));
    crate::fs::locking::locked_write(&output, &doc)?;
    Ok(output)
}
fn plan_description(source: Option<PathBuf>, root: &Path, ai: bool) -> String {
    let Some(path) = source else {
        return "No plan description available.".to_string();
    };
    let path = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    if ai {
        return summary::summarize_plan(&path);
    }
    fs::read_to_string(path)
        .map(|text| summary::fallback_description(&text))
        .unwrap_or_else(|_| "No plan description available.".to_string())
}
fn load_journals(work_dir: &Path) -> Result<HashMap<String, Vec<MemoryEntry>>> {
    // Load all memory journals, keyed by stage_id
    let journal_names = list_journals(work_dir).context("Failed to list memory journals")?;

    let mut journals: HashMap<String, Vec<MemoryEntry>> = HashMap::new();
    for stage_id in &journal_names {
        let journal = read_journal(work_dir, stage_id)
            .with_context(|| format!("Failed to read memory journal for stage '{stage_id}'"))?;
        if !journal.entries.is_empty() {
            journals.insert(stage_id.clone(), journal.entries);
        }
    }

    Ok(journals)
}
fn render(
    plan_id: &str,
    plan_name: &str,
    plan_description: &str,
    stages: &[StageInfo],
    journals: &HashMap<String, Vec<MemoryEntry>>,
) -> String {
    // Generate the review document
    let timestamp = Utc::now().format("%Y-%m-%d %H:%M UTC").to_string();

    let mut doc = String::new();
    doc.push_str(&format!("# Code Review: {}\n\n", plan_name));
    doc.push_str(&format!(
        "**Plan:** {} | **Generated:** {}\n\n",
        plan_id, timestamp
    ));

    // Summary section
    doc.push_str("## Summary\n\n");
    doc.push_str(plan_description);
    doc.push_str("\n\n");

    // Changes by Stage
    append_changes_by_stage_section(&mut doc, stages, journals);

    append_questions(&mut doc, stages, journals);
    doc
}

fn append_questions(
    doc: &mut String,
    stages: &[StageInfo],
    journals: &HashMap<String, Vec<MemoryEntry>>,
) {
    // Open Questions — collect all question entries across all stages
    doc.push_str("## Open Questions\n\n");

    let mut all_questions: Vec<(&str, &MemoryEntry)> = Vec::new();
    for stage in stages {
        if let Some(entries) = journals.get(&stage.id) {
            for entry in entries {
                if entry.entry_type == MemoryEntryType::Question {
                    all_questions.push((&stage.name, entry));
                }
            }
        }
    }

    if all_questions.is_empty() {
        doc.push_str("No open questions.\n");
    } else {
        for (stage_name, entry) in &all_questions {
            doc.push_str(&format!("- **[{}]** {}\n", stage_name, entry.content));
        }
    }

    doc.push('\n');
}
