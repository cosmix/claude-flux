//! The one retrieval entry point.
//!
//! Every caller that wants context for a stage — the `loom knowledge context`
//! command, signal generation, the prompt hook — goes through
//! [`retrieve_for_stage`]. Having a single pipeline is what makes a brief
//! rendered at spawn time and a brief pulled by hand comparable: same catalog,
//! same freshness evaluation, same ranking, same packer.
//!
//! Retrieval is deterministic and offline. There is no model call, no network
//! access and no randomness here; a [`ContextPack`] is a pure function of the
//! bytes on disk and the [`StageQuery`].

mod channels;
mod graph;

use crate::context::config::RetrievalConfig;
use crate::context::fuse::fuse;
use crate::context::graph_store::ResolvedGraph;
use crate::context::ingest::ingest;
use crate::context::local_overlay::OverlayScope;
use crate::context::pack::{pack, PackRequest};
use crate::context::refresh::{evaluate, refresh};
use crate::context::schema::{
    Channel, ContextPack, Freshness, LifecyclePolicy, RequiredRepresentation,
};
use crate::context::store::{ContextStore, StoreState};
use crate::fs::knowledge::catalog::Catalog;
use crate::fs::knowledge::KnowledgeDir;
use crate::fs::work_dir::WorkDir;
use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Everything a stage-scoped retrieval needs to be reproducible.
#[derive(Debug, Clone)]
pub struct StageQuery {
    /// Directory to start the `.loom/work/` search from ([`WorkDir::new`] semantics).
    pub work_dir_hint: PathBuf,
    /// Free text the ranker scores against.
    pub text: String,
    /// Chunk ids the caller demands verbatim.
    pub required_ids: Vec<String>,
    /// Lifecycle states this query may retrieve.
    pub lifecycle: LifecyclePolicy,
    /// Representation used for required ids.
    pub required_representation: RequiredRepresentation,
    /// Chunk ids referenced by stages this query's stage depends on.
    pub stage_dependency_ids: Vec<String>,
    /// Project-relative paths owned by the stages this query depends on.
    ///
    /// Only the stage spawn brief fills this; the hook and CLI leave it empty.
    /// `rank_source` boosts nodes whose file is named here (A.23).
    pub dependency_paths: Vec<String>,
    /// Channels to rank. Use `Channel::all().to_vec()` unless narrowing.
    pub scope: Vec<Channel>,
    /// Which source-graph overlay to read on top of the base layer.
    ///
    /// Defaults to [`OverlayScope::Local`] in [`StageQuery::new`], not to a
    /// base-only read: a query that names no stage means "the tree in front of
    /// me", so a dirty checkout needs its working-tree overlay applied to the
    /// committed base.
    pub overlay: OverlayScope,
}

impl StageQuery {
    /// A whole-scope query rooted at `work_dir_hint`.
    pub fn new(work_dir_hint: impl Into<PathBuf>, text: impl Into<String>) -> Self {
        StageQuery {
            work_dir_hint: work_dir_hint.into(),
            text: text.into(),
            required_ids: Vec::new(),
            lifecycle: LifecyclePolicy::default(),
            required_representation: RequiredRepresentation::default(),
            stage_dependency_ids: Vec::new(),
            dependency_paths: Vec::new(),
            scope: Channel::all().to_vec(),
            overlay: OverlayScope::Local,
        }
    }
}

/// Message for a caller that cannot proceed without a knowledge tree.
const NO_KNOWLEDGE_DIR: &str = "Knowledge directory not found. Run 'loom init' to create it.";

/// Structural freshness detail for a project that has no knowledge tree.
///
/// Never built, and therefore stale — not "current" over an empty catalog. A
/// derived layer that reports itself fresh when nothing was ever derived is the
/// failure `doc/loom/knowledge/mistakes/store-without-consumer.md` records: the
/// pack looks authoritative and its emptiness reads as "there is nothing to
/// say" rather than "nothing was ever built".
const NO_KNOWLEDGE_TREE_DETAIL: &str =
    "no knowledge directory; the knowledge channel is unavailable here";

/// Semantic freshness detail when no source graph has ever been built here.
const NO_SOURCE_GRAPH_DETAIL: &str = "source graph not built; run 'loom map' to build one";

/// Everything one retrieval resolves out of a `work_dir_hint` before it can
/// read anything: which knowledge tree, which cache, and which project root
/// owns the configuration for both.
pub(crate) struct ResolvedRoots {
    /// Knowledge tree root, `None` when `doc/loom/knowledge/` does not exist.
    pub(crate) knowledge_root: Option<PathBuf>,
    /// Canonical MAIN project root — the one [`ContextStore`] resolves its
    /// cache under, and therefore the one `.loom/config.toml` lives at.
    ///
    /// Not the worktree's own root: in a linked worktree `.loom/work` is a symlink
    /// into the main repository, and `.loom/` is not a tracked directory, so a
    /// stage resolving its config against its worktree would find nothing and
    /// silently run on defaults while the host ran on the operator's tunables.
    pub(crate) main_project_root: PathBuf,
    /// Cache of derived retrieval artifacts.
    pub(crate) store: ContextStore,
}

/// Resolve the knowledge root and the derived-artifact store together, so no two
/// callers can disagree about which tree or which cache they are working on.
///
/// The knowledge root is `None` when `doc/loom/knowledge/` does not exist. That
/// is a degraded retrieval, not a failure: the source channel ranks over the
/// resolved graph, which lives in the context cache and needs no catalog at
/// all, so a checkout with a mapped source graph and no knowledge tree still
/// has something to answer with. Callers that genuinely require a tree use
/// [`resolve_roots`].
pub(crate) fn resolve_roots_optional(work_dir_hint: &Path) -> Result<ResolvedRoots> {
    let work_dir = WorkDir::new(work_dir_hint)?;
    let project_root = work_dir
        .project_root()
        .context("Could not determine project root")?;
    let knowledge = KnowledgeDir::new(project_root);
    let knowledge_root = knowledge.exists().then(|| knowledge.root().to_path_buf());
    let main_project_root = work_dir
        .main_project_root()
        .context("Could not determine the canonical main project root")?;

    let store = ContextStore::open(&work_dir)?;
    Ok(ResolvedRoots {
        knowledge_root,
        main_project_root,
        store,
    })
}

/// [`resolve_roots_optional`] for a caller that cannot work without a knowledge
/// tree — the `loom knowledge` commands, which read and write that tree.
pub(crate) fn resolve_roots(work_dir_hint: &Path) -> Result<(PathBuf, ContextStore)> {
    let roots = resolve_roots_optional(work_dir_hint)?;
    let knowledge_root = roots
        .knowledge_root
        .ok_or_else(|| anyhow!(NO_KNOWLEDGE_DIR))?;
    Ok((knowledge_root, roots.store))
}

/// Bring the catalog current and return it. A read-only query must never die
/// because the cache is unwritable: a `refresh` failure is downgraded to a
/// warning and the catalog is built in memory instead.
///
/// With no knowledge tree there is nothing to ingest, so the catalog is empty
/// and the knowledge channel contributes no candidates. [`evaluate_state`] is
/// what keeps that honest.
fn resolve_catalog(
    store: &ContextStore,
    knowledge_root: Option<&Path>,
) -> Result<(Catalog, StoreState)> {
    let Some(knowledge_root) = knowledge_root else {
        let catalog = Catalog {
            revision: String::new(),
            chunks: Vec::new(),
            issues: Vec::new(),
        };
        return Ok((catalog, evaluate_state(store, None)?));
    };

    match refresh(store, knowledge_root, true) {
        Ok(outcome) => {
            let catalog = match store.load_catalog()? {
                Some(catalog) => catalog,
                None => ingest(knowledge_root)?.0,
            };
            let state = StoreState {
                structural: outcome.structural,
                semantic: outcome.semantic.freshness,
                catalog_revision: catalog.revision.clone(),
            };
            Ok((catalog, state))
        }
        Err(error) => {
            eprintln!(
                "warning: failed to refresh the context cache ({error}); using an in-memory catalog for this query"
            );
            let (catalog, _report) = ingest(knowledge_root)?;
            let state = evaluate_state(store, Some(knowledge_root))?;
            Ok((catalog, state))
        }
    }
}

/// Freshness of both derived layers for this query.
///
/// With a knowledge tree this is [`evaluate`] verbatim. Without one there is
/// nothing to fingerprint, so the structural layer reports itself never built
/// while the semantic layer keeps whatever the store records — the source graph
/// is not derived from the knowledge tree, and its revision is what
/// [`graph::load_resolved_graph`] reads the base layer by.
fn evaluate_state(store: &ContextStore, knowledge_root: Option<&Path>) -> Result<StoreState> {
    #[cfg(test)]
    EVALUATE_STATE_CALLS.with(|count| count.set(count.get() + 1));

    let Some(knowledge_root) = knowledge_root else {
        let stored = store.load_state()?;
        let semantic = if stored.semantic.revision.is_empty() {
            Freshness::never_built(NO_SOURCE_GRAPH_DETAIL)
        } else {
            stored.semantic
        };
        return Ok(StoreState {
            structural: Freshness::never_built(NO_KNOWLEDGE_TREE_DETAIL),
            semantic,
            catalog_revision: stored.catalog_revision,
        });
    };

    evaluate(store, knowledge_root)
}

/// Fail loudly when a caller demands a chunk or source-node id absent from
/// this query's scope.
///
/// Without this check the demand silently does nothing on a typo: the id
/// never matches, ranking proceeds as if it were never passed, and the caller
/// gets a successful pack with no signal that the requested id was never
/// included.
///
/// `graph` is consulted only when the caller passes `Some` — which
/// `retrieve_for_stage` does only when `Channel::Source` is in the query's
/// scope. A source-node id accepted while the source channel is out of scope
/// would pass this check and then never be ranked (`rank_source` is the only
/// reader of `graph`, and it never runs for a channel outside `scope`): the
/// exact silent no-op this function exists to prevent, just moved one step
/// later.
pub(crate) fn reject_unknown_require_ids(
    catalog: &Catalog,
    graph: Option<&ResolvedGraph>,
    require_id: &[String],
) -> Result<()> {
    if require_id.is_empty() {
        return Ok(());
    }
    let mut known_ids: BTreeSet<&str> = catalog
        .chunks
        .iter()
        .map(|chunk| chunk.id.as_str())
        .collect();
    if let Some(graph) = graph {
        known_ids.extend(graph.nodes().map(|node| node.id.as_str()));
    }
    let unknown: Vec<&str> = require_id
        .iter()
        .map(String::as_str)
        .filter(|id| !known_ids.contains(id))
        .collect();
    if !unknown.is_empty() {
        let ids = unknown.join(", ");
        bail!(
            "Unknown --require-id value(s): {ids}. No chunk or source node with that id \
             exists in scope; run 'loom knowledge context' without --require-id to see \
             available ids."
        );
    }
    Ok(())
}

/// Retrieve a token-budgeted pack for `query`.
///
/// Deterministic and offline: no model call, no network access, no randomness.
/// The result is a pure function of the bytes on disk and `query`.
///
/// Returns `Err` when the catalog cannot be built or `query.required_ids` names
/// an id the catalog does not hold. An ABSENT knowledge directory is not an
/// error: retrieval runs source-only, over the resolved graph alone, and the
/// pack says the structural layer was never built. Callers on a hot path —
/// signal generation, the prompt hook — must degrade rather than propagate: log
/// at `tracing::debug` and carry on with no pack.
pub fn retrieve_for_stage(query: &StageQuery, budget_tokens: usize) -> Result<ContextPack> {
    let roots = resolve_roots_optional(&query.work_dir_hint)?;
    let knowledge_root = roots.knowledge_root.as_deref();

    let config = RetrievalConfig::load(&roots.main_project_root);

    let (catalog, mut state) = resolve_catalog(&roots.store, knowledge_root)?;

    let graph_load = graph::load_resolved_graph(
        &query.work_dir_hint,
        &roots.store,
        &state.semantic.revision,
        &query.overlay,
    );
    if let Some(detail) = graph_load.working_tree_stale {
        state.semantic.stale = true;
        state.semantic.detail = Some(detail);
    }
    let graph = graph_load.graph;

    check_require_ids(query, &catalog, graph.as_ref())?;

    let chunks_by_id: BTreeMap<&str, &_> = catalog
        .chunks
        .iter()
        .map(|chunk| (chunk.id.as_str(), chunk))
        .collect();
    let ranked = channels::rank_channels_cached(
        &query.scope,
        &channels::build_rank_query(query),
        &catalog,
        &chunks_by_id,
        graph.as_ref(),
        query.lifecycle,
        &config,
        Some(roots.store.root()),
    );
    let fused = fuse(&ranked.lists);

    let request = build_pack_request(
        query,
        budget_tokens,
        state,
        ranked.dropped_terms,
        ranked.surviving_terms,
        graph_load.degraded,
    );
    Ok(pack(&request, &fused, &catalog.chunks, graph.as_ref()))
}

/// Reject a `--require-id` this query's scope cannot possibly hold.
///
/// `graph` is offered to the check only when the source channel is actually in
/// scope: see [`reject_unknown_require_ids`]'s doc comment for why accepting a
/// source-node id for a query that will never rank source nodes reintroduces
/// the exact silent no-op that function exists to prevent.
fn check_require_ids(
    query: &StageQuery,
    catalog: &Catalog,
    graph: Option<&ResolvedGraph>,
) -> Result<()> {
    let source_graph = query
        .scope
        .contains(&Channel::Source)
        .then_some(graph)
        .flatten();
    reject_unknown_require_ids(catalog, source_graph, &query.required_ids)
}

fn build_pack_request(
    query: &StageQuery,
    budget_tokens: usize,
    state: StoreState,
    dropped_terms: Vec<String>,
    surviving_terms: Vec<String>,
    degraded: Option<String>,
) -> PackRequest {
    PackRequest {
        query: query.text.clone(),
        scope: query.scope.clone(),
        budget_tokens,
        structural_freshness: state.structural,
        semantic_freshness: state.semantic,
        dropped_terms,
        surviving_terms,
        required_representation: query.required_representation,
        degraded,
    }
}

#[cfg(test)]
thread_local! {
    static EVALUATE_STATE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn take_evaluate_state_call_count() -> usize {
    EVALUATE_STATE_CALLS.with(|count| count.replace(0))
}

/// Identity of the derived-data generation a pack was built from.
///
/// Two packs share an epoch exactly when both derived layers were built from
/// the same bytes, which is what makes "already delivered" a safe thing to say.
/// Rebuilding either layer changes the epoch and re-opens delivery. A layer that
/// was never built contributes its empty revision string; the epoch stays well
/// defined.
pub fn context_epoch(pack: &ContextPack) -> String {
    let mut hasher = Sha256::new();
    hasher.update(pack.structural_freshness.revision.as_bytes());
    hasher.update(b"\n");
    hasher.update(pack.semantic_freshness.revision.as_bytes());
    let result = hasher.finalize();
    hex::encode(&result[..8])
}
