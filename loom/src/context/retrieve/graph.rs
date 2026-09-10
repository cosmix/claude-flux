//! Resolving the source graph for a query's overlay, and detecting the A.11
//! degraded mode: a non-empty semantic revision whose RESOLVED graph —
//! base layer plus whatever overlay this query is scoped to — came back
//! with no content at all. A missing base alone is not this condition when an
//! overlay still provides a complete usable view.
//!
//! Split out of `retrieve.rs` so the top-level pipeline in
//! [`super::retrieve_for_stage`] stays a readable sequence of steps rather
//! than growing this reasoning inline. See
//! `doc/PROPOSAL-retrieval-precision.md` §A.11 for the design this
//! implements.

use crate::context::graph_store::{GraphStore, ResolvedGraph};
use crate::context::local_overlay::OverlayScope;
use crate::context::refresh::{clean_generation, short_revision, working_tree};
use crate::context::store::ContextStore;
use crate::fs::work_dir::WorkDir;
use std::path::Path;

pub(super) struct GraphLoad {
    pub graph: Option<ResolvedGraph>,
    pub degraded: Option<String>,
    pub working_tree_stale: Option<String>,
}

impl GraphLoad {
    fn empty() -> Self {
        Self {
            graph: None,
            degraded: None,
            working_tree_stale: None,
        }
    }
}

/// Load the resolved source graph for `query`'s overlay, degrading to `None`
/// on any error, alongside an A.11 degradation message when the base layer
/// for a non-empty semantic revision could not be found.
///
/// `overlay.resolve` always yields a `(plan, stage)` pair, so this always asks
/// [`GraphStore::resolved`] for the overlay-applied view — never `None` for
/// the stage. That distinction matters on its own: with `None`, `resolved`
/// reads only the base layer, and a base miss there becomes an *empty* graph
/// rather than a missing one, silently dropping an overlay the query should
/// have read. [`OverlayScope::Local`] resolves to the `(plan, stage)` address
/// `local_overlay_key` computes, written through
/// `ensure_snapshot(SnapshotPolicy::LocalCurrent)` by `loom map`
/// (`commands/map.rs`), `loom knowledge sync`, and the prompt hook's
/// background reconcile (`commands/hook/reconcile_graph.rs`) — so a
/// `Local`-scoped query is what lets a caller see working-tree changes beyond
/// the base.
///
/// Retrieval itself never builds or refreshes this graph: `resolve_catalog`
/// calls `refresh` with `structural_only = true`, which skips the semantic
/// reconcile on every call this pipeline makes. So this function only ever
/// reads what a prior `loom map` run, or a merge, already wrote for
/// `semantic_revision`.
///
/// **Real, currently-reachable degraded mode:** a non-empty
/// `semantic_revision` names a layer `state.json` claims was built, but
/// NEITHER half of the resolved view can back that claim: no base file was
/// found for the revision (`graph_store::GraphStore::resolved`'s base half is
/// `unwrap_or_default()` over `load_base`, so a missing base file resolves to
/// an *empty* base rather than an error) AND nothing published an overlay to
/// cover for it either, so the resolved graph has no files at all.
/// [`degraded_reason`] turns THAT combination into the message
/// `super::build_pack_request` carries out to `ContextPack::degraded`. A
/// missing base alone is NOT this condition — see this module's doc comment
/// and [`degraded_reason`]'s own — so a checkout with a healthy overlay, or
/// one with a genuinely empty published base, both read as `Semantic:
/// current`, exactly as they should; only a checkout where the source
/// channel has genuinely nothing to answer with gets flagged.
pub(super) fn load_resolved_graph(
    work_dir_hint: &Path,
    store: &ContextStore,
    semantic_revision: &str,
    overlay: &OverlayScope,
) -> GraphLoad {
    let Some(work_dir) = WorkDir::new(work_dir_hint).ok() else {
        return GraphLoad::empty();
    };
    let Some(project_root) = work_dir.project_root() else {
        return GraphLoad::empty();
    };
    let (plan, stage) = overlay.resolve(project_root);
    let graph_store = GraphStore::new(store.root(), work_dir.root());
    let Ok(overlay_layer) = graph_store.load_overlay(&plan, &stage) else {
        return GraphLoad::empty();
    };
    let Ok(graph) = graph_store.resolved(semantic_revision, Some((&plan, &stage))) else {
        return GraphLoad::empty();
    };

    let degraded = degraded_reason(semantic_revision, &graph);
    let working_tree_stale = working_tree(project_root)
        .ok()
        .and_then(|tree| match overlay_layer {
            Some(layer) if layer.generation != tree.generation => {
                Some("working tree changed since the source graph overlay was built".to_string())
            }
            None if tree.generation != clean_generation(&tree.head) => {
                Some("working tree has changes but no source graph overlay exists".to_string())
            }
            _ => None,
        });
    GraphLoad {
        graph: Some(graph),
        degraded,
        working_tree_stale,
    }
}

/// The A.11 degradation message, or `None` when this read is honestly
/// healthy.
///
/// An EMPTY `semantic_revision` means "never built" — `evaluate_state`
/// already reports that honestly as a `never_built` [`crate::context::schema::Freshness`]
/// elsewhere in the pack, so flagging it here too would print a
/// `DEGRADED:` banner on every unmapped checkout forever, for a condition
/// that is not a degradation at all; skip it.
///
/// Otherwise this is NOT simply `graph.base_revision.is_empty()` — that was
/// the bug. `GraphStore::resolved` returns `base ∪ overlay`, and an overlay
/// can still provide the requested checkout view if an older cache lacks the
/// matching base. Testing `base_revision` alone flagged every such checkout
/// as degraded forever, which is both a banner nobody can ever clear (a warning that is
/// always on is a warning nobody reads) and, far more importantly, a live
/// input to [`crate::commands::hook::reconcile_graph::spawn_if_needed`]: that
/// function fires a detached full-repository tree-sitter rebuild on `stale OR
/// degraded`, so this predicate does not merely choose a display string — it
/// decides whether every single prompt in every checkout with a dirty tree
/// (i.e. nearly all of them) starts an unbounded background rebuild, throttled
/// only by the reconcile debounce lock.
///
/// The honest test is two-part: a base was found (`base_revision` non-empty —
/// note this stays true even for a genuinely empty, zero-file base: a
/// published layer over a project with no matching source files is current,
/// not degraded), OR the resolved view has ANY content at all (`files`
/// non-empty — an overlay alone can supply this with no base present). Only
/// when NEITHER holds — no base was found for this revision AND nothing else
/// covered for it — is there truly nothing to answer a query with, which is
/// the one case worth surfacing.
fn degraded_reason(semantic_revision: &str, graph: &ResolvedGraph) -> Option<String> {
    if semantic_revision.is_empty() || !graph.base_revision.is_empty() || !graph.files.is_empty() {
        return None;
    }
    Some(format!(
        "source graph base {} missing and no overlay covers this checkout — no source graph content available",
        short_revision(semantic_revision)
    ))
}
