//! Knowledge command - manage curated codebase knowledge.
pub mod annotate;
pub mod check;
pub mod context;
pub mod eval;
pub mod sync;
pub mod telemetry;

use crate::fs::knowledge::{KnowledgeDir, KnowledgeTarget, SectionOutcome};
use crate::fs::work_dir::WorkDir;
use anyhow::{bail, Context, Result};
use colored::Colorize;

fn read_content_from_stdin() -> Result<String> {
    use std::io::Read;
    let limit = (crate::validation::MAX_KNOWLEDGE_CONTENT_LENGTH + 1) as u64;
    let mut buffer = String::new();
    std::io::stdin()
        .take(limit)
        .read_to_string(&mut buffer)
        .context("Failed to read from stdin")?;
    let trimmed = buffer.trim().to_string();
    if trimmed.is_empty() {
        bail!("No content received from stdin");
    }
    crate::validation::validate_knowledge_content(&trimmed)?;
    Ok(trimmed)
}

/// Resolve inline content, `-`, or an omitted argument to validated markdown.
fn resolve_content(content: Option<String>) -> Result<String> {
    let content = match content {
        Some(c) if c == "-" => read_content_from_stdin()?,
        Some(c) => c,
        None => read_content_from_stdin()?,
    };
    crate::validation::validate_knowledge_content(&content)?;
    Ok(content)
}

/// Open the knowledge directory of the cwd's project root, initializing it on
/// first use. Cwd-relative on purpose: a worktree agent writes to its own tree.
fn open_knowledge_dir() -> Result<KnowledgeDir> {
    let work_dir = WorkDir::new(".")?;
    let project_root = work_dir
        .project_root()
        .context("Could not determine project root")?;
    let knowledge = KnowledgeDir::new(project_root);

    if !knowledge.exists() {
        knowledge
            .initialize()
            .context("Failed to initialize knowledge directory")?;
    }
    Ok(knowledge)
}

pub fn update(file: String, content: Option<String>) -> Result<()> {
    let content = resolve_content(content)?;
    let knowledge = open_knowledge_dir()?;
    let target = KnowledgeTarget::parse(&file)?;
    knowledge.append_target(&target, &content)?;

    println!(
        "{} Appended to {}",
        "✓".green().bold(),
        target.display_name()
    );

    Ok(())
}

/// Normalize a heading argument to the bare text `replace_section_target`
/// matches on, independent of what ATX level (`##` through `######`) it is
/// written at.
///
/// Callers reach for this command right after `loom knowledge update`, whose
/// content carries the heading's own `#`-prefix inline, so they pass it here
/// too.
fn normalize_heading(heading: &str) -> Result<String> {
    let normalized = heading.trim().trim_start_matches('#').trim().to_string();
    if normalized.is_empty() {
        bail!("Section heading cannot be empty");
    }
    if normalized.contains('\n') {
        bail!("Section heading must be a single line, got: {normalized:?}");
    }
    Ok(normalized)
}

/// Drop a `#{2,6} <heading>` line repeated at the top of the body, at
/// whatever level the caller happened to paste it at.
///
/// `replace_section_target` writes the heading itself, at the level it finds
/// it (or `## ` when appending fresh); leaving the caller's copy in place
/// would produce the doubled heading this command exists to prevent. The
/// newline/prefix check keeps `## Merge` from matching `## Merge Flow`.
fn strip_repeated_heading(content: &str, heading: &str) -> String {
    let rest = content.trim_start();
    for level in 2..=6 {
        let marker = format!("{} {heading}", "#".repeat(level));
        if let Some(after) = rest.strip_prefix(&marker) {
            if after.is_empty() || after.starts_with('\n') {
                return after.trim_start_matches('\n').trim_end().to_string();
            }
        }
    }
    rest.trim_end().to_string()
}

/// Overwrite a `#{2,6} <heading>` section in place — the correction path for
/// stale knowledge, at whatever level the heading actually lives (a heading
/// nested as `### ` under an `## ` group is matched and corrected in place,
/// not just an exact `## `).
///
/// `update` appends, which leaves the stale text sitting above its own fix
/// (see `doc/loom/knowledge/mistakes/knowledge-base-drift.md`). This replaces
/// the section body instead, and falls back to appending a new `## ` section
/// when no heading at any level matches — reported distinctly, because a
/// heading typo (or wrong level) would otherwise look like a successful
/// correction.
pub fn replace_section(file: String, heading: String, content: Option<String>) -> Result<()> {
    let content = resolve_content(content)?;
    let heading = normalize_heading(&heading)?;
    let content = strip_repeated_heading(&content, &heading);
    if content.is_empty() {
        bail!("Replacement content cannot be empty (the heading line alone is not a section body)");
    }

    let knowledge = open_knowledge_dir()?;
    let target = KnowledgeTarget::parse(&file)?;
    let outcome = knowledge.replace_section_target(&target, &heading, &content)?;

    match outcome {
        SectionOutcome::Replaced { level } => {
            let marker = "#".repeat(level);
            println!(
                "{} Replaced \"{marker} {heading}\" in {}",
                "✓".green().bold(),
                target.display_name()
            );
        }
        SectionOutcome::Appended => {
            println!(
                "{} No \"{heading}\" section in {} - appended it as a new \"## {heading}\" \
                 (check the heading text; any level from ## to ###### is matched)",
                "!".yellow().bold(),
                target.display_name()
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "tests_legacy.rs"]
mod tests_legacy;

#[cfg(test)]
#[path = "tests_replace_section_levels.rs"]
mod tests_replace_section_levels;

#[cfg(test)]
#[path = "tests_annotate.rs"]
mod tests_annotate;
