//! The semantic (source-graph) half of [`super::refresh`], and the typed
//! answer to "which layer did this sync actually write?".
//!
//! Lives beside `source_graph` rather than inside it because this logic grows
//! with every new outcome the CLI must report, and `source_graph.rs` sits at
//! its file-size limit.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

use super::{
    ensure_snapshot, SnapshotAction, SnapshotOutcome, SnapshotPolicy, SourceGraphCounters,
};
use crate::context::graph_store::GraphStore;
use crate::context::schema::Freshness;
use crate::context::store::ContextStore;
use crate::fs::work_dir::WorkDir;

/// Prefix for every advisory line about the source graph, on any surface.
///
/// A NEW convention introduced here: this codebase has no shared advisory
/// marker (`advisory_codex_lane_preflight` prints bare text,
/// `check_for_uncommitted_changes` uses a red "x", `foreground.rs` uses a
/// literal "Warning: "). `loom knowledge sync` and
/// `commands::run::checks::advisory_source_graph_preflight` share THIS one so
/// the two surfaces agree.
pub const SOURCE_GRAPH_PREFIX: &str = "source graph: ";

/// What the semantic half of [`super::refresh`] actually did.
///
/// [`Freshness`] cannot answer this on its own: it carries only a revision, a
/// timestamp and a staleness reason, so it can say neither which layer was
/// written nor how big it is. Machine-readable state belongs in [`Self::layer`]
/// — never make a caller substring-match `freshness.detail` prose to learn
/// which layer it got.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticOutcome {
    /// Which layer this call ended up writing.
    pub layer: SemanticLayer,
    pub nodes: usize,
    pub edges: usize,
    /// Freshness of the semantic layer after this call.
    pub freshness: Freshness,
    pub counters: SourceGraphCounters,
    #[serde(skip)]
    snapshot: Option<SnapshotOutcome>,
}

/// Which layer the semantic reconcile ended up writing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SemanticLayer {
    /// Immutable base published for the current revision.
    Base { revision: String },
    /// The committed base plus the checkout's working-tree overlay.
    BaseAndLocalOverlay {
        revision: String,
        plan: String,
        stage: String,
    },
    /// Nothing ran: `--structural-only`, or an unresolvable project root.
    Skipped { reason: String },
}

impl SemanticOutcome {
    /// The semantic layer was not touched by this call. Carries the freshness
    /// the store already held through unchanged, with zero counts, because
    /// nothing was walked.
    pub fn skipped(freshness: Freshness, reason: impl Into<String>) -> Self {
        Self {
            layer: SemanticLayer::Skipped {
                reason: reason.into(),
            },
            nodes: 0,
            edges: 0,
            freshness,
            counters: SourceGraphCounters::default(),
            snapshot: None,
        }
    }

    fn from_snapshot(
        layer: SemanticLayer,
        freshness: Freshness,
        nodes: usize,
        edges: usize,
        snapshot: SnapshotOutcome,
    ) -> Self {
        Self {
            layer,
            nodes,
            edges,
            freshness,
            counters: snapshot.counters.clone(),
            snapshot: Some(snapshot),
        }
    }

    pub fn action(&self) -> Option<SnapshotAction> {
        self.snapshot.as_ref().map(|snapshot| snapshot.action)
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.snapshot
            .as_ref()
            .map(|snapshot| u64::try_from(snapshot.elapsed.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0)
    }

    pub fn describe(&self) -> String {
        match &self.snapshot {
            Some(snapshot) => snapshot.describe(),
            None => match &self.layer {
                SemanticLayer::Skipped { reason } => {
                    format!("{SOURCE_GRAPH_PREFIX}skipped ({reason})")
                }
                _ => format!("{SOURCE_GRAPH_PREFIX}unavailable (snapshot outcome missing)"),
            },
        }
    }
}

/// Best-effort semantic reconciliation for `project_root`: any failure below
/// degrades to a stale [`Freshness`] naming it — this never returns an `Err`,
/// by design, so a caller on a hot or fire-and-forget path never has to
/// decide what to do with one.
///
/// `pub(crate)`, not `pub(super)`: reachable from
/// `commands::hook::reconcile_graph` (A.12/A.22's checkout-scope background
/// reconcile), which needs exactly this "base, plus `_local` when dirty"
/// policy and must not re-derive it — a second derivation
/// of one rule is the drift risk `architecture/context-retrieval.md`'s
/// `plan_key` reasoning already warns about (`orchestrator/signals/retrieval.rs`
/// routes through one helper for the same reason). No other visibility in
/// this module was widened for that call site.
///
/// Takes `project_root` directly rather than a knowledge root — the only
/// thing [`try_reconcile_semantic`] actually needs, and what a caller with no
/// knowledge tree (a source-graph-only checkout, see `retrieve.rs`'s
/// `resolve_roots_optional` doc comment) has on hand. [`refresh`] itself only
/// ever has a knowledge root, so [`reconcile_semantic_best_effort_from_knowledge_root`]
/// derives one and calls through.
pub(crate) fn reconcile_semantic_best_effort(
    store: &ContextStore,
    project_root: &Path,
    current: Freshness,
) -> SemanticOutcome {
    match try_reconcile_semantic(store, project_root) {
        Ok(outcome) => outcome,
        Err(error) => {
            let reason = format!("semantic reconciliation skipped: {error}");
            let freshness = Freshness {
                stale: true,
                detail: Some(reason.clone()),
                ..current
            };
            SemanticOutcome::skipped(freshness, reason)
        }
    }
}

/// [`reconcile_semantic_best_effort`] for a caller that only has a knowledge
/// root, not the project root itself — derives it via [`derive_project_root`],
/// degrading when the layout does not match. [`super::refresh`] is the only
/// production caller; it always has a knowledge root by its own contract, so
/// this wrapper stays `pub(super)` rather than widening further.
pub(super) fn reconcile_semantic_best_effort_from_knowledge_root(
    store: &ContextStore,
    knowledge_root: &Path,
    current: Freshness,
) -> SemanticOutcome {
    let Some(project_root) = derive_project_root(knowledge_root) else {
        return SemanticOutcome::skipped(
            current,
            "project root could not be derived from the knowledge root",
        );
    };
    reconcile_semantic_best_effort(store, project_root, current)
}

/// Derive the project root from `knowledge_root` (`<root>/doc/loom/knowledge`),
/// refusing to guess when the layout does not match - an unvalidated ancestor
/// would point `WorkDir`, `GraphStore` and `rev-parse` at the wrong tree.
///
/// `pub(crate)` rather than private: `refresh::evaluate` also needs it, to
/// resolve the project root a stored semantic revision should be checked
/// against `git rev-parse HEAD` for (`refresh.rs`'s
/// `semantic_freshness_against_head`).
pub(crate) fn derive_project_root(knowledge_root: &Path) -> Option<&Path> {
    let candidate = knowledge_root.ancestors().nth(3)?;
    let derived = candidate.join("doc/loom/knowledge");
    let matches = match (derived.canonicalize(), knowledge_root.canonicalize()) {
        (Ok(derived), Ok(actual)) => derived == actual,
        _ => derived == knowledge_root,
    };
    matches.then_some(candidate)
}

/// The fallible half of [`reconcile_semantic_best_effort`]; any error becomes a
/// named staleness reason.
///
/// A base always describes committed `HEAD`. When the checkout differs from
/// that clean generation, the same probe also drives a `_local` overlay.
fn try_reconcile_semantic(store: &ContextStore, project_root: &Path) -> Result<SemanticOutcome> {
    let work_dir = WorkDir::new(project_root)?;
    let graph_store = GraphStore::new(store.root(), work_dir.root());
    let snapshot = ensure_snapshot(
        store,
        &graph_store,
        project_root,
        SnapshotPolicy::LocalCurrent,
    )?;
    let freshness = store.load_state()?.semantic;
    let layer = semantic_layer(&snapshot);
    let (nodes, edges) = resolved_counts(&graph_store, &snapshot)?;
    Ok(SemanticOutcome::from_snapshot(
        layer, freshness, nodes, edges, snapshot,
    ))
}

fn semantic_layer(snapshot: &SnapshotOutcome) -> SemanticLayer {
    if snapshot.action == SnapshotAction::Unavailable {
        return SemanticLayer::Skipped {
            reason: snapshot.reason.clone(),
        };
    }
    match &snapshot.overlay {
        Some((plan, stage)) => SemanticLayer::BaseAndLocalOverlay {
            revision: snapshot.revision.clone(),
            plan: plan.clone(),
            stage: stage.clone(),
        },
        None => SemanticLayer::Base {
            revision: snapshot.revision.clone(),
        },
    }
}

fn resolved_counts(graph_store: &GraphStore, snapshot: &SnapshotOutcome) -> Result<(usize, usize)> {
    if snapshot.revision.is_empty() {
        return Ok((0, 0));
    }
    let overlay = snapshot
        .overlay
        .as_ref()
        .map(|(plan, stage)| (plan.as_str(), stage.as_str()));
    let graph = graph_store.resolved(&snapshot.revision, overlay)?;
    Ok((graph.node_count(), graph.edge_count()))
}
