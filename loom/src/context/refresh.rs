//! Freshness evaluation and structural-layer rebuilds for the context store.
//!
//! [`evaluate`] answers "is the cached catalog current?" without touching
//! disk beyond a read. [`refresh`] acts on that answer: it rebuilds and
//! persists the structural layer when stale, and is a no-op otherwise. The
//! semantic (source-graph) layer lives in `source_graph`.

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::context::fingerprint::{fingerprint_tree, tree_revision, FileFingerprint};
use crate::context::ingest::IngestReport;
use crate::context::schema::Freshness;
use crate::context::store::{ContextStore, StoreState};
use crate::fs::knowledge::catalog;

mod semantic;
pub mod snapshot;
mod source_graph;

pub use semantic::{SemanticLayer, SemanticOutcome, SOURCE_GRAPH_PREFIX};
pub use snapshot::{ensure_snapshot, SnapshotAction, SnapshotOutcome, SnapshotPolicy};
pub(crate) use source_graph::{clean_generation, working_tree};
pub use source_graph::{
    mark_semantic_stale, reconcile_source_graph, SourceGraphCounters, SourceGraphOutcome,
    SourceGraphScope,
};

// Crate-visible only: `commands::hook::reconcile_graph` (A.12/A.22's
// checkout-scope background reconcile) needs the exact same "always ensure a
// Base, and add a `_local` overlay when dirty" policy this implements, and must call
// it rather than re-derive it. See the function's own doc comment for the
// full reasoning; no other item in `semantic` was widened for this.
pub(crate) use semantic::reconcile_semantic_best_effort;

/// One entry of the extractor registry the semantic refresh drives.
///
/// Named here rather than in `context::extract` because the boxing is the
/// refresh driver's requirement, not the trait's: the driver builds the whole
/// registry once and hands slices of it down its own call chain, which needs
/// `Send + Sync` for no reason a single extractor implementation cares about.
pub(crate) type BoxedExtractor =
    Box<dyn crate::context::extract::SourceGraphExtractor + Send + Sync>;

struct Evaluated {
    state: StoreState,
    fingerprints: Vec<FileFingerprint>,
}

/// What a refresh actually did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefreshOutcome {
    /// False when the cached catalog was already current.
    pub rebuilt: bool,
    /// Structural (catalog) freshness after this call.
    pub structural: Freshness,
    /// What the semantic (source graph) half of this call did - which layer it
    /// wrote, how big it is, and how fresh it now is.
    pub semantic: SemanticOutcome,
    /// Present only when a rebuild happened.
    pub report: Option<IngestReport>,
}

/// Decide whether the structural (catalog) layer is stale, given the stored
/// freshness record and the freshly computed tree revision.
fn structural_freshness(stored: &Freshness, current_revision: &str) -> Freshness {
    if stored.revision.is_empty() {
        Freshness::never_built("catalog has never been built")
    } else if stored.revision != current_revision {
        Freshness {
            revision: stored.revision.clone(),
            computed_at: stored.computed_at,
            stale: true,
            detail: Some("knowledge tree changed since the catalog was built".to_string()),
        }
    } else {
        Freshness {
            stale: false,
            detail: None,
            ..stored.clone()
        }
    }
}

/// Compare the cached state against the knowledge tree WITHOUT rebuilding.
///
/// `structural.stale` is true when the stored structural revision differs
/// from the freshly computed tree revision, when nothing has been built yet,
/// or when the persisted catalog on disk does not match what `state.json`
/// claims was built (missing, deleted, or overwritten out from under it —
/// `save_catalog` and `save_state` are two independent writes with no
/// cross-file transaction, so this is the only place that catches the two
/// files disagreeing).
///
/// The semantic layer is still owned by the source-graph stage — this
/// function never rebuilds it, and an absent semantic revision is reported as
/// never built, same as always. What DOES happen now: when a semantic
/// revision has been recorded, it is checked against `git rev-parse HEAD` for
/// the project `knowledge_root` belongs to (`semantic_freshness_against_head`
/// below), and reported `stale: true` when HEAD has moved past it. This is
/// display plus a trigger for a later background reconcile — NOT a rebuild:
/// [`refresh`] only ever rebuilds the STRUCTURAL layer when it finds
/// `structural.stale`, and never reads this function's semantic verdict to
/// decide whether to rebuild anything, so a moved HEAD reported here cannot
/// start one. `revision` itself always stays the STORED value — that is the
/// layer actually on disk, and what `load_resolved_graph`
/// (`context/retrieve.rs`) keys the base read by; only `stale`/`detail`
/// change. A knowledge root whose project cannot be resolved, or a project
/// with no git HEAD (not a repository, no commits, `git` unavailable),
/// degrades to passing the stored semantic freshness through unchanged — a
/// missing git repository is data, not a crash
/// (`refresh/source_graph.rs`'s module doc).
pub fn evaluate(store: &ContextStore, knowledge_root: &Path) -> Result<StoreState> {
    Ok(evaluate_inner(store, knowledge_root)?.state)
}

fn evaluate_inner(store: &ContextStore, knowledge_root: &Path) -> Result<Evaluated> {
    let stored = store.load_state()?;

    let fingerprints = fingerprint_once(knowledge_root)?;
    let current_revision = tree_revision(&fingerprints);

    let mut structural = structural_freshness(&stored.structural, &current_revision);

    // A structural revision was recorded, meaning a catalog was supposedly
    // built — confirm the catalog on disk still backs that claim. Skip this
    // when nothing has ever been built: that case already has its own detail
    // message above, and an absent catalog there is expected, not a mismatch.
    if !stored.structural.revision.is_empty() {
        let catalog = store.load_catalog()?;
        let catalog_is_missing_or_stale = match &catalog {
            None => true,
            Some(catalog) => catalog.revision != stored.catalog_revision,
        };
        if catalog_is_missing_or_stale {
            structural.stale = true;
            structural.detail = Some(
                "cached catalog is missing or does not match the recorded revision".to_string(),
            );
        }
    }

    // The semantic layer is populated by the source-graph stage, not this
    // one. Never invent a revision for it, and never rebuild it here — see
    // the doc comment above and on `semantic_freshness_against_head`.
    let semantic = if stored.semantic.revision.is_empty() {
        Freshness::never_built("source graph not built; see the source-graph stage")
    } else {
        semantic_freshness_against_head(knowledge_root, stored.semantic.clone())
    };

    Ok(Evaluated {
        state: StoreState {
            structural,
            semantic,
            catalog_revision: stored.catalog_revision,
        },
        fingerprints,
    })
}

fn fingerprint_once(knowledge_root: &Path) -> Result<Vec<FileFingerprint>> {
    #[cfg(test)]
    FINGERPRINT_PASSES.with(|count| count.set(count.get().saturating_add(1)));
    fingerprint_tree(knowledge_root)
}

#[cfg(test)]
std::thread_local! {
    static FINGERPRINT_PASSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn take_fingerprint_pass_count() -> usize {
    FINGERPRINT_PASSES.with(|count| count.replace(0))
}

/// Check `stored` (a non-empty semantic revision) against the current
/// `git rev-parse HEAD` for the project `knowledge_root` belongs to.
///
/// Read-only: no rebuild, no write, no reconcile. Only `stale`/`detail` are
/// ever changed — `revision` is returned exactly as `stored` held it, because
/// that is the base layer actually on disk (see [`evaluate`]'s doc comment).
/// When the project root cannot be derived from `knowledge_root`, or the
/// project has no resolvable git HEAD, `stored` is returned completely
/// unchanged — this must never invent a verdict from an unusable comparison.
fn semantic_freshness_against_head(knowledge_root: &Path, stored: Freshness) -> Freshness {
    let Some(project_root) = semantic::derive_project_root(knowledge_root) else {
        return stored;
    };
    let Some(head) = source_graph::head_revision(project_root) else {
        return stored;
    };
    if head == stored.revision {
        return stored;
    }

    Freshness {
        revision: stored.revision.clone(),
        computed_at: stored.computed_at,
        stale: true,
        detail: Some(format!(
            "HEAD moved: {} → {}",
            short_revision(&stored.revision),
            short_revision(&head)
        )),
    }
}

/// First 8 characters of `revision`, or the whole string when it is shorter.
///
/// Character-based, not a byte slice: `&revision[..8]` panics both when
/// `revision` has fewer than 8 bytes and when byte index 8 does not fall on a
/// UTF-8 character boundary. This runs inside `evaluate`, reachable from the
/// prompt hook, which is contractually forbidden to ever disturb a session —
/// see `hooks/user-prompt-context.sh`'s fail-open contract.
///
/// `pub(crate)`: `context::retrieve::graph` needs the identical truncation
/// for its A.11 degraded-message (`"source graph base <rev8> missing"`) and
/// must not grow a second, possibly-drifting copy of a panic-safety rule.
pub(crate) fn short_revision(revision: &str) -> String {
    revision.chars().take(8).collect()
}

/// Why the semantic layer was skipped when the caller asked for the catalog only.
const STRUCTURAL_ONLY_REASON: &str = "--structural-only";

/// Rebuild the structural layer when it is stale; persist catalog and state.
/// `structural_only` distinguishes catalog-only from also best-effort
/// reconciling the semantic layer (see
/// `semantic::reconcile_semantic_best_effort_from_knowledge_root`).
pub fn refresh(
    store: &ContextStore,
    knowledge_root: &Path,
    structural_only: bool,
) -> Result<RefreshOutcome> {
    let evaluated = evaluate_inner(store, knowledge_root)?;
    let state = evaluated.state;

    if !state.structural.stale {
        let semantic = if structural_only {
            SemanticOutcome::skipped(state.semantic, STRUCTURAL_ONLY_REASON)
        } else {
            semantic::reconcile_semantic_best_effort_from_knowledge_root(
                store,
                knowledge_root,
                state.semantic,
            )
        };
        return Ok(RefreshOutcome {
            rebuilt: false,
            structural: state.structural,
            semantic,
            report: None,
        });
    }

    let mut outcome = rebuild_and_persist(
        store,
        knowledge_root,
        state.semantic,
        evaluated.fingerprints,
    )?;
    if !structural_only {
        let current = outcome.semantic.freshness.clone();
        outcome.semantic = semantic::reconcile_semantic_best_effort_from_knowledge_root(
            store,
            knowledge_root,
            current,
        );
    }
    Ok(outcome)
}

/// Rebuild the catalog from `knowledge_root`, persist it and the derived
/// state, and return the outcome. Only called from `refresh` once `evaluate`
/// has found the structural layer stale.
fn rebuild_and_persist(
    store: &ContextStore,
    knowledge_root: &Path,
    semantic: Freshness,
    fingerprints: Vec<FileFingerprint>,
) -> Result<RefreshOutcome> {
    // Persist the SAME revision `evaluate` compares against — the tree revision.
    // `catalog.revision` is a different hash over a different subject (chunk ids
    // and their content hashes, not file paths and their bytes); storing it here
    // would compare two hash domains that can never be equal, making every
    // refresh rebuild from scratch. The catalog keeps its own revision inside
    // catalog.json as the catalog's identity.
    let current_revision = tree_revision(&fingerprints);

    let catalog = catalog::build(knowledge_root)?;
    let report = IngestReport {
        knowledge_root: knowledge_root.to_path_buf(),
        files: fingerprints.len(),
        chunks: catalog.chunks.len(),
        revision: catalog.revision.clone(),
        issues: catalog.issues.clone(),
    };
    store.save_catalog(&catalog)?;

    let structural = Freshness {
        revision: current_revision,
        computed_at: Some(Utc::now()),
        stale: false,
        detail: None,
    };

    // `update_state` re-reads `state.json` under the lock rather than reusing
    // the `semantic` snapshot `evaluate` took before `ingest` ran above: a
    // concurrent writer may have changed `semantic` in that gap, and writing
    // the stale snapshot back would revert it. Only assign the fields owned
    // here; `semantic` stays at its on-disk value.
    store.update_state(|state| {
        state.structural = structural.clone();
        state.catalog_revision = catalog.revision.clone();
    })?;

    Ok(RefreshOutcome {
        rebuilt: true,
        structural,
        semantic: SemanticOutcome::skipped(semantic, STRUCTURAL_ONLY_REASON),
        report: Some(report),
    })
}

#[cfg(test)]
#[path = "refresh/tests_freshness.rs"]
mod tests_freshness;

#[cfg(test)]
#[path = "refresh/tests_snapshot.rs"]
mod tests_snapshot;
