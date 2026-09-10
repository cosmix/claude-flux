//! Generate a structured code review document from stage memory journals.

use anyhow::{Context, Result};
use chrono::Utc;
use colored::Colorize;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::claude::find_claude_path;
use crate::commands::common::resolve_work_dir;
use crate::fs::memory::{list_journals, read_journal, MemoryEntry, MemoryEntryType};
use crate::fs::work_dir::load_config;
use crate::git::worktree::find_worktree_root_from_cwd;
use crate::parser::frontmatter::extract_frontmatter_field;

use super::changes_section::append_changes_by_stage_section;

/// Information extracted from a stage file.
pub(super) struct StageInfo {
    pub(super) id: String,
    name: String,
    description: String,
    status: String,
    /// Original filename, used for sorting.
    _filename: String,
}

/// Load all stage files from `.loom/work/stages/` sorted by filename.
fn load_stage_infos(stages_dir: &Path) -> Result<Vec<StageInfo>> {
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

    let mut stages = Vec::new();
    for (filename, path) in entries {
        let content = fs::read_to_string(&path)
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

        stages.push(StageInfo {
            id,
            name,
            description,
            status,
            _filename: filename,
        });
    }

    Ok(stages)
}

/// Summarize a plan file by passing its content to Claude Haiku.
///
/// Falls back to the plan's first paragraph if claude is unavailable.
fn summarize_plan(plan_path: &Path) -> String {
    let content = match fs::read_to_string(plan_path) {
        Ok(c) => c,
        Err(_) => return "No plan description available.".to_string(),
    };

    let claude_path = match find_claude_path() {
        Ok(p) => p,
        Err(_) => return fallback_description(&content),
    };

    let prompt = "Summarize this execution plan in 2-4 concise sentences. \
                  Focus on what the plan accomplishes, not its structure. \
                  Output only the summary, no preamble.";

    let result = Command::new(&claude_path)
        .args(["-p", "--model", "haiku", prompt])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(ref mut stdin) = child.stdin {
                let _ = stdin.write_all(content.as_bytes());
            }
            child.wait_with_output()
        });

    match result {
        Ok(output) if output.status.success() => {
            let summary = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if summary.is_empty() {
                fallback_description(&content)
            } else {
                summary
            }
        }
        _ => fallback_description(&content),
    }
}

/// Extract the first paragraph after the heading as a fallback description.
fn fallback_description(content: &str) -> String {
    let metadata_marker = "<!-- loom METADATA -->";
    let body = if let Some(idx) = content.find(metadata_marker) {
        &content[..idx]
    } else {
        content
    };

    let mut lines = body.lines().peekable();
    // Skip to first heading
    while let Some(line) = lines.peek() {
        if line.trim_start().starts_with('#') {
            lines.next();
            break;
        }
        lines.next();
    }
    // Skip blank lines after heading
    while let Some(line) = lines.peek() {
        if line.trim().is_empty() {
            lines.next();
        } else {
            break;
        }
    }
    // Collect until next blank line (first paragraph only)
    let mut paragraph = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            break;
        }
        paragraph.push(line);
    }

    let trimmed = paragraph.join("\n").trim().to_string();
    if trimmed.is_empty() {
        "No plan description available.".to_string()
    } else {
        trimmed
    }
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

    // Files Changed
    out.push_str("#### Files Changed\n\n");
    let changes: Vec<&MemoryEntry> = entries
        .iter()
        .copied()
        .filter(|e| e.entry_type == MemoryEntryType::Change)
        .collect();
    if changes.is_empty() {
        out.push_str("No changes recorded.\n");
    } else {
        for entry in &changes {
            out.push_str(&format!("{}\n", format_entry_bullet(entry)));
        }
    }

    // Key Decisions
    out.push_str("\n#### Key Decisions\n\n");
    let decisions: Vec<&MemoryEntry> = entries
        .iter()
        .copied()
        .filter(|e| e.entry_type == MemoryEntryType::Decision)
        .collect();
    if decisions.is_empty() {
        out.push_str("No decisions recorded.\n");
    } else {
        for entry in &decisions {
            out.push_str(&format!("{}\n", format_entry_bullet(entry)));
        }
    }

    // Notes (most recent 10)
    out.push_str("\n#### Notes\n\n");
    let notes: Vec<&MemoryEntry> = entries
        .iter()
        .copied()
        .filter(|e| e.entry_type == MemoryEntryType::Note)
        .collect();
    if notes.is_empty() {
        out.push_str("No notes recorded.\n");
    } else {
        for entry in notes.iter().rev().take(10) {
            out.push_str(&format!("{}\n", format_entry_bullet(entry)));
        }
    }

    out.push('\n');
    out
}

/// Execute the `loom review` command.
///
/// `ai_summary` opts into a headless Claude Haiku (`claude -p`) summary of the
/// plan. It is off by default so `loom review` never silently incurs headless
/// API charges; without it the plan's first paragraph is used as the summary.
/// Resolve where the review document should be written.
///
/// Inside a loom worktree this is the WORKTREE root, not the main repo:
/// `.loom/work` is a symlink to the main repo's state dir, so resolving through
/// it (as `main_project_root()` does) would silently write into the main repo's
/// `doc/plans/` — invisible from the worktree that ran the command.
fn resolve_output_root(cwd: &Path, project_root: &Path) -> PathBuf {
    find_worktree_root_from_cwd(cwd).unwrap_or_else(|| project_root.to_path_buf())
}

pub fn execute(ai_summary: bool) -> Result<()> {
    let workspace = resolve_work_dir()?;
    let work_dir = workspace.root().to_path_buf();

    // Resolve the project root (follow symlinks for worktrees)
    let project_root: PathBuf = workspace
        .main_project_root()
        .context("Could not determine project root")?;

    // Load config.toml
    let config = load_config(&work_dir)?.context("No active plan. Run 'loom init' first.")?;

    let plan_id = config.plan_id().unwrap_or("unknown").to_string();

    let plan_name = config
        .get_plan_str("plan_name")
        .unwrap_or(&plan_id)
        .to_string();

    let source_path = config.source_path();

    println!(
        "{} Generating code review for plan '{}'...",
        "→".cyan().bold(),
        plan_name.bold()
    );

    // Summarize the plan. With --ai-summary, use a headless Claude Haiku call
    // (falls back to first paragraph on failure); otherwise read the plan's
    // first paragraph directly so no headless `claude -p` charge is incurred.
    let plan_description = match &source_path {
        Some(path) => {
            let resolved = if path.is_absolute() {
                path.clone()
            } else {
                project_root.join(path)
            };
            if ai_summary {
                println!(
                    "{} Summarizing plan with Claude Haiku...",
                    "→".cyan().bold()
                );
                summarize_plan(&resolved)
            } else {
                match fs::read_to_string(&resolved) {
                    Ok(content) => fallback_description(&content),
                    Err(_) => "No plan description available.".to_string(),
                }
            }
        }
        None => "No plan description available.".to_string(),
    };

    // Load stage infos
    let stages_dir = work_dir.join("stages");
    let stages = load_stage_infos(&stages_dir).context("Failed to load stage files")?;

    // Load all memory journals, keyed by stage_id
    let journal_names = list_journals(&work_dir).context("Failed to list memory journals")?;

    let mut journals: HashMap<String, Vec<MemoryEntry>> = HashMap::new();
    for stage_id in &journal_names {
        let journal = read_journal(&work_dir, stage_id)
            .with_context(|| format!("Failed to read memory journal for stage '{stage_id}'"))?;
        if !journal.entries.is_empty() {
            journals.insert(stage_id.clone(), journal.entries);
        }
    }

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
    doc.push_str(&plan_description);
    doc.push_str("\n\n");

    // Changes by Stage
    append_changes_by_stage_section(&mut doc, &stages, &journals);

    // Open Questions — collect all question entries across all stages
    doc.push_str("## Open Questions\n\n");

    let mut all_questions: Vec<(&str, &MemoryEntry)> = Vec::new();
    for stage in &stages {
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

    // Write to doc/plans/REVIEW-{plan_id}.md in the CURRENT checkout: the
    // worktree's doc/plans/ when run inside a worktree, else the main repo's.
    let cwd = env::current_dir().context("Failed to get current directory")?;
    let output_root = resolve_output_root(&cwd, &project_root);
    let plans_dir = output_root.join("doc").join("plans");
    fs::create_dir_all(&plans_dir).context("Failed to create doc/plans directory")?;

    let output_filename = format!("REVIEW-{}.md", plan_id);
    let output_path = plans_dir.join(&output_filename);

    fs::write(&output_path, &doc)
        .with_context(|| format!("Failed to write review document: {}", output_path.display()))?;

    println!(
        "{} Review document written to {}",
        "✓".green().bold(),
        output_path
            .strip_prefix(&output_root)
            .unwrap_or(&output_path)
            .display()
            .to_string()
            .cyan()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_output_root_inside_worktree() {
        let cwd = Path::new("/repo/.worktrees/my-stage/loom/src");
        let project_root = PathBuf::from("/repo");
        assert_eq!(
            resolve_output_root(cwd, &project_root),
            PathBuf::from("/repo/.worktrees/my-stage")
        );
    }

    #[test]
    fn test_resolve_output_root_in_main_repo() {
        let cwd = Path::new("/repo/loom/src");
        let project_root = PathBuf::from("/repo");
        assert_eq!(resolve_output_root(cwd, &project_root), project_root);
    }
}
