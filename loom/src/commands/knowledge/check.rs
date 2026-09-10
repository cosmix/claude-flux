//! `loom knowledge check` — report knowledge-base diagnostics WITHOUT ever
//! opening the context store, which is what makes it safe to run as a loom
//! stage's acceptance criterion.
//!
//! `loom knowledge sync` resolves through `context::retrieve::resolve_roots`
//! (`commands/knowledge/context.rs:29`), which calls `ContextStore::open`
//! (`context/store.rs:49`). `open` itself only COMPUTES the cache root: it
//! follows `WorkDir::main_project_root` OUT of a worktree, through the
//! state directory symlink, to the MAIN repository, and joins `.loom/cache`. The
//! write happens later, when `sync`'s `refresh` (`context/refresh.rs:218`)
//! calls `ContextStore::save_catalog` (`context/store.rs:108`) against that
//! root — a write that escapes worktree isolation, and both settings
//! emitters strip `.loom` from `allow_write`, so `sync` can never sit in a
//! stage's acceptance list without tripping the sandbox.
//! `catalog::build(root: &Path)` (`fs/knowledge/catalog.rs`) is pure: it reads
//! the tree and returns diagnostics, writing nothing. `check` resolves ONLY
//! the knowledge root — never the context store — and calls `catalog::build`
//! directly. `skills/loom-plan-writer/SKILL.md` already names `loom knowledge
//! check` as knowledge-distill's acceptance criterion, so this command fills a
//! contract that already ships; do not "simplify" it back into
//! `context::resolve()`.

use crate::context::untrusted::inline_safe;
use crate::fs::knowledge::catalog::size::MAX_INDEX_BYTES;
use crate::fs::knowledge::catalog::{self, Catalog, CatalogIssue};
use crate::fs::knowledge::types::INDEX_FILENAME;
use crate::fs::knowledge::KnowledgeDir;
use crate::fs::work_dir::WorkDir;
use anyhow::{Context, Result};
use colored::Colorize;
use std::path::Path;

/// Report the knowledge base's diagnostics. Resolves the knowledge root
/// read-only (see the module doc) and never initializes or mutates it.
///
/// Always exits 0, except `strict` combined with at least one non-review issue,
/// which exits 1 after all printing.
pub fn check(strict: bool, json: bool) -> Result<()> {
    let root = knowledge_root()?;
    if !root.exists() {
        report_missing_root(&root, json)?;
        return Ok(());
    }

    let catalog = catalog::build(&root)?;
    if json {
        print_json(&root, &catalog)?;
    } else {
        print_human(&root, &catalog);
    }

    let strict_count = strict_issue_count(&catalog.issues);
    if strict && strict_count > 0 {
        eprintln!(
            "loom knowledge check: FAIL - {} issue(s) found under {}",
            strict_count,
            root.display()
        );
        std::process::exit(1);
    }
    Ok(())
}

/// `doc/loom/knowledge` under the current project root, resolved WITHOUT
/// `KnowledgeDir::initialize()` and WITHOUT touching the context store.
///
/// Deliberately not `super::open_knowledge_dir()`: that helper initializes a
/// missing directory, which is exactly the mutation a read-only diagnostic
/// command must not perform.
fn knowledge_root() -> Result<std::path::PathBuf> {
    let work_dir = WorkDir::new(".")?;
    let project_root = work_dir
        .project_root()
        .context("Could not determine project root")?;
    Ok(KnowledgeDir::new(project_root).root().to_path_buf())
}

fn report_missing_root(root: &Path, json: bool) -> Result<()> {
    if json {
        // Reuses `json_payload` against an empty catalog rather than
        // hand-writing a second `{root, issues, count}` literal, so the two
        // JSON shapes cannot drift apart.
        let empty = Catalog {
            revision: String::new(),
            chunks: Vec::new(),
            issues: Vec::new(),
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&json_payload(root, &empty))?
        );
    } else {
        println!(
            "{} No knowledge directory at {}",
            "─".dimmed(),
            root.display()
        );
    }
    Ok(())
}

/// The JSON payload's shape. Issues are structured data a machine parses, so
/// unlike [`issue_line`] this must NOT route any field through
/// [`inline_safe`] — flattening is for the human-readable stdout line only.
fn json_payload(root: &Path, catalog: &Catalog) -> serde_json::Value {
    let issues: Vec<_> = catalog
        .issues
        .iter()
        .filter(|issue| !issue.is_review_only())
        .collect();
    let review: Vec<_> = catalog
        .issues
        .iter()
        .filter(|issue| issue.is_review_only())
        .collect();
    serde_json::json!({
        "root": root,
        "issues": issues,
        "review": review,
        "count": strict_issue_count(&catalog.issues),
    })
}

fn strict_issue_count(issues: &[CatalogIssue]) -> usize {
    issues
        .iter()
        .filter(|issue| !issue.is_review_only())
        .count()
}

fn print_json(root: &Path, catalog: &Catalog) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&json_payload(root, catalog))?
    );
    Ok(())
}

fn print_human(root: &Path, catalog: &Catalog) {
    println!("{}", human_report(root, catalog));
}

/// Build the human-readable report as a single string, mirroring
/// [`json_payload`]'s split from [`print_json`] — `print_human` stays a thin
/// `println!` wrapper, and this is what tests assert against so a deleted
/// or garbled report body fails a test instead of going unnoticed.
fn human_report(root: &Path, catalog: &Catalog) -> String {
    if catalog.issues.is_empty() {
        return format!(
            "{} Knowledge base at {} is clean",
            "✓".green().bold(),
            root.display()
        );
    }
    let mut lines: Vec<String> = catalog.issues.iter().map(decorated_issue_line).collect();
    let strict_count = strict_issue_count(&catalog.issues);
    let review_count = catalog.issues.len() - strict_count;
    lines.push(format!(
        "{} {strict_count} issue(s), {review_count} review note(s) found under {}",
        "!".yellow().bold(),
        root.display()
    ));
    lines.join("\n")
}

fn decorated_issue_line(issue: &CatalogIssue) -> String {
    match issue {
        CatalogIssue::EvidenceChanged { .. } | CatalogIssue::UnverifiableReference { .. } => {
            issue_line(issue)
        }
        _ => format!("{} {}", "!".yellow().bold(), issue_line(issue)),
    }
}

/// One human-readable line per issue. Matched EXHAUSTIVELY — no `_ =>`
/// catch-all — so a future `CatalogIssue` variant fails to compile here
/// instead of silently printing nothing for it.
///
/// Every untrusted field — `heading`, `blurb`, `target`, `source_path`,
/// `verified`, `kind`, and the file path itself — is routed through
/// [`inline_safe`] before it
/// reaches this line. These values come straight from unvalidated knowledge
/// files: `validate_knowledge_content` (`validation.rs:129`) checks only
/// emptiness and length, not control characters, so a heading or blurb can
/// carry an ANSI escape sequence or a bidi override. This is stdout on an
/// agent-facing surface — the same containment `map/views/mod.rs` applies to
/// graph-derived text (`context/untrusted.rs`'s module doc names both
/// surfaces). Do not strip the flattening back out to "simplify" this.
fn issue_line(issue: &CatalogIssue) -> String {
    match issue {
        CatalogIssue::DuplicateHeading {
            file,
            heading,
            occurrences,
        } => format!(
            "{}: heading \"{}\" repeated {occurrences} times",
            safe_path(file),
            inline_safe(heading)
        ),
        CatalogIssue::GenericBlurb { file, blurb } => format!(
            "{}: still has the scaffold blurb \"{}\"",
            safe_path(file),
            inline_safe(blurb)
        ),
        CatalogIssue::BrokenLink { file, target } => format!(
            "{}: link target \"{}\" does not resolve",
            safe_path(file),
            inline_safe(target)
        ),
        CatalogIssue::MissingSourceRef { file, source_path } => format!(
            "{}: source reference \"{}\" does not exist",
            safe_path(file),
            inline_safe(source_path)
        ),
        CatalogIssue::EvidenceChanged { .. } | CatalogIssue::UnverifiableReference { .. } => {
            review_issue_line(issue)
        }
        CatalogIssue::OversizedSection {
            file,
            heading,
            lines,
        } => size_issue_line(&safe_path(file), Some(&inline_safe(heading)), *lines),
        CatalogIssue::OversizedFile { file, lines } => {
            size_issue_line(&safe_path(file), None, *lines)
        }
        CatalogIssue::OversizedIndex { bytes } => format!(
            "{INDEX_FILENAME} is {bytes} bytes, over the {MAX_INDEX_BYTES}-byte budget - trim it"
        ),
    }
}

fn review_issue_line(issue: &CatalogIssue) -> String {
    match issue {
        CatalogIssue::EvidenceChanged {
            file,
            source_path,
            verified,
        } => format!(
            "review: {}: {} changed since {} — re-verify or `loom knowledge annotate {} --verified HEAD`",
            safe_path(file),
            inline_safe(source_path),
            inline_safe(&verified.chars().take(8).collect::<String>()),
            safe_path(file)
        ),
        CatalogIssue::UnverifiableReference {
            file,
            source_path,
            kind,
        } => format!(
            "note: {}: unresolved {} reference \"{}\"",
            safe_path(file),
            inline_safe(kind),
            inline_safe(source_path)
        ),
        _ => unreachable!("review_issue_line only accepts review-only issues"),
    }
}

/// Flatten a `CatalogIssue`'s relative file path the same way its content
/// fields are flattened — the path is built from a directory walk over the
/// knowledge tree, so it carries whatever bytes are in the file name on
/// disk, same as any other field named in [`issue_line`]'s doc comment.
fn safe_path(file: &Path) -> String {
    inline_safe(&file.display().to_string())
}

/// Shared phrasing for the two tier-1 size issues that point at the same
/// CLAUDE.md Rule 12 remedy — keeps `issue_line` well under the function
/// size limit instead of inlining both messages there.
fn size_issue_line(file: &str, heading: Option<&str>, lines: usize) -> String {
    match heading {
        Some(heading) => format!(
            "{file}: section \"{heading}\" is {lines} lines - move the detail to a tier-2 topic file (CLAUDE.md Rule 12)"
        ),
        None => format!(
            "{file}: tier-1 file is {lines} lines - split the detail into a tier-2 topic file (CLAUDE.md Rule 12)"
        ),
    }
}

#[cfg(test)]
#[path = "tests_check.rs"]
mod tests;
