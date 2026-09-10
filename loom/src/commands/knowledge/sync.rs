//! `loom knowledge sync` — upgrade a flat knowledge tree, refresh `INDEX.md`,
//! and rebuild derived context artifacts.
//!
//! `sync` is the single, explicit flat-to-hierarchical upgrade: a knowledge
//! directory that predates the tiered layout has no `INDEX.md`, and this is the
//! one command that creates it. Nothing else migrates a flat dir — `update` and
//! every retrieval path leave it flat forever.
//!
//! On an ALREADY-hierarchical directory `sync` still regenerates `INDEX.md`
//! unconditionally, not just on the one-time upgrade. `update` and
//! `replace-section` each refresh it too after their own write
//! (`KnowledgeDir::refresh_index_if_hierarchical`, `fs/knowledge/dir.rs`), but
//! CLAUDE.md Rule 12 also sanctions editing `doc/loom/knowledge/*.md` directly
//! with Edit/Write in an interactive session — a path that writes no index at
//! all. `sync` is the command an agent reaches for to fix that up, so it must
//! not be a no-op on a healthy tree; see [`sync`]'s own doc for why the write
//! failure there is best-effort.
//!
//! Everything after that is derived state: `refresh` rebuilds the catalog when
//! stale and persists it to the cache, and is a no-op when the catalog is
//! already current.

use super::context::resolve;
use crate::context::refresh::{refresh, RefreshOutcome, SemanticLayer, SemanticOutcome};
use crate::fs::knowledge::{KnowledgeDir, KnowledgeLayout, INDEX_FILENAME};
use anyhow::{Context, Result};
use colored::Colorize;
use std::path::Path;

/// Upgrade a flat knowledge tree (or refresh `INDEX.md` on an already-
/// hierarchical one), then rebuild derived context artifacts.
///
/// The refresh-on-already-hierarchical write is best-effort: a failure there
/// is logged to stderr and does NOT fail the sync. The catalog rebuild below
/// is the substantive work this command exists for, and an index that stays
/// one edit behind is a cosmetic loss, not a reason to make an otherwise
/// successful `sync` exit non-zero — matching
/// `KnowledgeDir::refresh_index_if_hierarchical`'s own posture
/// (`fs/knowledge/dir.rs`) rather than inventing a new one. The UPGRADE write
/// (flat to hierarchical) stays a hard failure: a flat tree that cannot get an
/// index at all is a real problem worth surfacing, not a cosmetic one.
///
/// A [`refresh`] failure (the derived context catalog under `.loom/cache/`)
/// still fails `sync` — that rebuild is real, un-cosmetic work — but the
/// index step above has already run by the time `refresh` is even called, so
/// letting the bare `refresh` error stand on its own would read as "sync did
/// nothing" to a caller who only checks the exit code. `catalog_failure_context`
/// annotates the error to say the index step already completed, leaving the
/// underlying cause (still the real failure reason) in the chain beneath it.
pub fn sync(structural_only: bool, json: bool) -> Result<()> {
    let (knowledge_root, store) = resolve()?;
    let upgraded = upgrade_flat_layout(&knowledge_root)?;
    if !upgraded {
        refresh_index_best_effort(&knowledge_root);
    }
    let outcome = refresh(&store, &knowledge_root, structural_only)
        .with_context(|| catalog_failure_context(upgraded))?;

    if json {
        print_json(&outcome, upgraded)
    } else {
        print_human(&outcome, upgraded);
        Ok(())
    }
}

/// Create `INDEX.md` on a flat (pre-hierarchy) knowledge directory, turning it
/// hierarchical. Returns whether the upgrade happened. This is the one place
/// in loom that MIGRATES a flat dir to hierarchical; every read and update
/// path leaves a flat dir flat. It does NOT describe everything [`sync`]'s
/// call site does on the `false` (already-hierarchical) branch — see
/// [`refresh_index_best_effort`], which [`sync`] calls right after this one.
fn upgrade_flat_layout(knowledge_root: &Path) -> Result<bool> {
    let knowledge = KnowledgeDir::from_root(knowledge_root);
    if knowledge.layout() == KnowledgeLayout::Hierarchical {
        return Ok(false);
    }
    knowledge.write_index()?;
    Ok(true)
}

/// Regenerate `INDEX.md` for a directory [`upgrade_flat_layout`] found
/// already hierarchical, so `sync` picks up whatever the tree currently holds
/// — edited blurbs, a resized tier-2 file, a new topic file written directly
/// with Edit/Write — instead of only ever writing the index on the one-time
/// flat-to-hierarchical upgrade.
///
/// Best-effort by design, NOT propagated as an error: see [`sync`]'s doc
/// comment for why, and `KnowledgeDir::refresh_index_if_hierarchical`
/// (`fs/knowledge/dir.rs`) for the identical posture this mirrors rather than
/// reinvents.
///
/// Called from [`sync`] OUTSIDE any locked read-modify-write — `write_index`
/// takes the same parent-directory lock every tier-1/tier-2 write takes
/// (`INDEX.md` and those files share `knowledge_root` as their parent), and
/// `fs/locking.rs`'s `flock` is per-open-file-description: a second exclusive
/// lock request on that directory from a thread that already holds one blocks
/// forever rather than erroring. `sync` never wraps this call in a lock of its
/// own, so this is safe as written; a future caller must keep it that way
/// (see `doc/loom/knowledge/mistakes/knowledge-cli-invariants.md`).
fn refresh_index_best_effort(knowledge_root: &Path) {
    let knowledge = KnowledgeDir::from_root(knowledge_root);
    if let Err(error) = knowledge.write_index() {
        eprintln!("warning: failed to refresh {INDEX_FILENAME}: {error:#}");
    }
}

/// Context attached to a [`refresh`] failure, naming which of `sync`'s two
/// jobs actually failed.
///
/// `refresh` rebuilds the derived context catalog — cached state under
/// `.loom/cache/` that a re-run rebuilds from the knowledge tree, not the
/// knowledge tree itself. By the time `sync` calls it, the index half above
/// has already run, so a bare propagation of `refresh`'s error would read as
/// total failure (see `loom-bugs.txt` BUG 4) even though `doc/loom/knowledge/`
/// is intact. The two branches mirror what the caller can actually promise:
/// `upgraded` came back from a hard-failing write ([`upgrade_flat_layout`]),
/// so "created" is a fact; the `false` branch instead went through
/// [`refresh_index_best_effort`], which can itself fail silently (a stderr
/// warning, not a returned error), so it only claims the step ran — never
/// that `INDEX.md` is correct.
fn catalog_failure_context(upgraded: bool) -> String {
    let index_step = if upgraded {
        "the knowledge index was created (flat directory upgraded to hierarchical)"
    } else {
        "the knowledge index step completed before this failure"
    };
    format!(
        "{index_step}; rebuilding the derived context catalog failed \
         (cached state under .loom/cache/ that a re-run rebuilds)"
    )
}

fn print_json(outcome: &RefreshOutcome, upgraded: bool) -> Result<()> {
    let payload = serde_json::json!({
        "upgraded": upgraded,
        "rebuilt": outcome.rebuilt,
        "revision": outcome.structural.revision,
        "files": outcome.report.as_ref().map(|report| report.files),
        "chunks": outcome.report.as_ref().map(|report| report.chunks),
        "issues": outcome.report.as_ref().map(|report| report.issues.len()),
        "semantic": semantic_json(&outcome.semantic),
    });
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(())
}

/// The source-graph half of the `--json` payload. `layer` is the
/// machine-readable discriminator; a caller must never have to substring-match
/// `detail` prose to learn which layer it got.
fn semantic_json(semantic: &SemanticOutcome) -> serde_json::Value {
    let (layer, revision, plan, stage, detail) = match &semantic.layer {
        SemanticLayer::Base { revision } => (
            "base",
            Some(revision.as_str()),
            None,
            None,
            semantic.freshness.detail.as_deref(),
        ),
        SemanticLayer::BaseAndLocalOverlay {
            revision,
            plan,
            stage,
        } => (
            "base-and-local-overlay",
            Some(revision.as_str()),
            Some(plan.as_str()),
            Some(stage.as_str()),
            semantic.freshness.detail.as_deref(),
        ),
        SemanticLayer::Skipped { reason } => ("skipped", None, None, None, Some(reason.as_str())),
    };
    serde_json::json!({
        "action": semantic.action().map(|action| action.as_str()).unwrap_or("skipped"),
        "layer": layer,
        "revision": revision,
        "plan": plan,
        "stage": stage,
        "files": semantic.counters.files_enumerated,
        "parsed": semantic.counters.files_parsed,
        "reused": semantic.counters.files_reused,
        "deleted": semantic.counters.files_deleted,
        "elapsed_ms": semantic.elapsed_ms(),
        "nodes": semantic.nodes,
        "edges": semantic.edges,
        "counters": &semantic.counters,
        "stale": semantic.freshness.stale,
        "detail": detail,
    })
}

fn print_human(outcome: &RefreshOutcome, upgraded: bool) {
    if upgraded {
        println!(
            "{} Upgraded the knowledge directory to the hierarchical layout (created {INDEX_FILENAME})",
            "✓".green().bold()
        );
    }
    if outcome.rebuilt {
        println!("{} Rebuilt the context catalog", "✓".green().bold());
    } else {
        println!("{} Catalog already current", "─".dimmed());
    }
    println!("  {} {}", "Revision:".cyan(), outcome.structural.revision);

    if let Some(report) = &outcome.report {
        println!("  {} {}", "Files:".cyan(), report.files);
        println!("  {} {}", "Chunks:".cyan(), report.chunks);
        println!("  {} {}", "Issues:".cyan(), report.issues.len());
    }
    print_semantic(&outcome.semantic);
}

/// One honest line about the source-graph (semantic) layer. `loom knowledge
/// sync` drives BOTH layers, and printing only the catalog is what made the
/// command look like it did nothing.
fn print_semantic(semantic: &SemanticOutcome) {
    println!("{}", semantic.describe());
}

#[cfg(test)]
#[path = "tests_sync.rs"]
mod tests;
