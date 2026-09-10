//! Deterministic ranking of derived source-graph nodes for
//! [`Channel::Source`].
//!
//! The knowledge ranker in [`crate::context::rank`] scores curated prose; this
//! one scores the symbols extracted into the source graph, reusing the same
//! exact-match rungs, the same BM25 statistics and the same candidate type so
//! the two lists are directly comparable when [`crate::context::fuse`] merges
//! them by reciprocal rank.
//!
//! What it does NOT share with the knowledge ranker is who may become a
//! candidate. A curated chunk is prose and answers a question asked in prose; a
//! source node is a pointer into the code and answers a question that named the
//! code. `candidacy` is that difference written down — see that module's doc
//! for the three measured collisions that made it necessary.
//!
//! ## Note for whoever next touches `fuse`
//!
//! Fusion keys its accumulator by [`ChunkId`] *across* channels. Knowledge ids
//! are `<path>#<heading>#<occurrence>` while source ids are
//! `<path>#<kind>:<scope>`, over disjoint path spaces, so a collision is not
//! reachable today — but if one ever occurred, `fuse` would merge the two
//! entries and adopt the
//! better-ranked channel, after which [`crate::context::pack`]'s
//! channel-dispatch would consult the wrong map and silently drop the item. Do
//! not "simplify" that dispatch into trying both maps: it would hide the
//! collision instead of surfacing it.

mod candidacy;
mod expand;
mod paths;

pub use paths::normalize_dependency_path;

use crate::context::config::RetrievalConfig;
use crate::context::graph_store::ResolvedGraph;
use crate::context::lexical::{ExactGate, WEIGHT_BODY, WEIGHT_SYMBOLS};
use crate::context::lexical_index::LexicalCache;
use crate::context::rank::{
    prepare_lexical_cached, tokenize, ChannelRanking, LexicalCorpus, RankQuery, RankedCandidate,
    RungScore, BOOST_EXACT_PATH, BOOST_EXACT_SYMBOL, BOOST_EXPLICIT_ID, BOOST_STAGE_DEPENDENCY,
};
use crate::context::schema::{
    estimate_tokens, Channel, ChunkId, FileCoverage, SelectionReason, SourceNode, SourceNodeKind,
};
use candidacy::admits_lexical_evidence;
use expand::expand_from_seeds;
use paths::{apply_test_path_factor, matches_path, names_dependency_path, PathMatch};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Most source candidates one ranking pass hands to fusion.
///
/// [`crate::context::fuse`] is pure reciprocal-rank fusion, so a rank-1 source
/// node contributes exactly what a rank-1 knowledge chunk contributes, and
/// there is no per-channel budget anywhere in the pipeline. Packing then walks
/// the fused list in order taking whole units until the budget is spent (see
/// [`crate::context::pack`]). A source item costs ~20-30 tokens against a knowledge
/// chunk's few hundred, so an unbounded source list does not crowd out prose by
/// token volume — it crowds it out by *slot*, one alternating rank at a time.
/// 60 keeps the source channel able to answer a symbol query fully while
/// leaving the fused head dominated by prose: source facts are a pointer into
/// the code, prose is the thing that cannot be re-derived from the code.
const MAX_SOURCE_CANDIDATES: usize = 60;

/// A scored node paired with the positional key its ties break on.
struct ScoredNode<'a> {
    candidate: RankedCandidate,
    order: (&'a Path, usize),
}

/// Score every symbol node of `graph` against the query for
/// [`Channel::Source`], reporting the corpus diagnostics alongside the
/// candidates.
///
/// Results descend by score and break ties by `(path, line_start)` so two runs
/// over identical bytes produce an identical order. This is the form
/// [`crate::context::retrieve`] calls, because the pack has to report the
/// dropped terms; [`rank_source`] is the same pass without them.
///
/// Scores over a freshly tokenized corpus — the scan path, and the oracle the
/// persistent index is checked against. [`rank_source_channel_cached`] is the
/// same pass with an index behind it.
pub fn rank_source_channel(
    query: &RankQuery,
    graph: &ResolvedGraph,
    config: &RetrievalConfig,
) -> ChannelRanking {
    rank_source_channel_cached(query, graph, config, None)
}

/// [`rank_source_channel`], reading and maintaining the persistent lexical
/// index (A.13) when `cache` is given.
///
/// The `Option` is what keeps the scan reachable: a caller with no context
/// cache root — and every existing test — passes `None` and gets the full
/// scan, so the fallback path stays exercised rather than becoming code that
/// only runs after a cache is deleted.
pub fn rank_source_channel_cached(
    query: &RankQuery,
    graph: &ResolvedGraph,
    config: &RetrievalConfig,
    cache: Option<&LexicalCache>,
) -> ChannelRanking {
    // Whole-file nodes carry no signature and no scope, so they can only ever
    // match on their path — one per file, crowding out the symbols inside it.
    let nodes: Vec<&SourceNode> = graph
        .nodes()
        .filter(|node| !matches!(node.kind, SourceNodeKind::File))
        .collect();
    if nodes.is_empty() {
        return ChannelRanking::default();
    }

    let query_terms = tokenize(&query.text);
    // Node ids in corpus order: the index is keyed by the resolved layer, and
    // these prove the file it hands back describes exactly these nodes in
    // exactly this order. Building them is a pointer copy per node; building
    // the documents below is the ~7,900-node tokenization a warm index exists
    // to skip, which is why it is a closure and not a value.
    let doc_ids: Vec<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
    // Order is load-bearing: the gate's `rare` test reads these document
    // frequencies, so the corpus has to exist before any rung is scored.
    let corpus = prepare_lexical_cached(
        &query_terms,
        &doc_ids,
        || nodes.iter().copied().map(node_document).collect(),
        &query.text,
        config,
        cache,
    );
    let gate = ExactGate::new(
        &query.text,
        &corpus.document_frequencies,
        config.df_ident_max,
    );

    let scored = score_nodes(query, &nodes, &corpus, &gate, config);
    let ranked = sorted_candidates(scored);
    let ranked = expand_from_seeds(ranked, graph, config);
    rank_order(ranked, graph, corpus.dropped_terms)
}

/// Put ordinary scored nodes in seed-strength order before graph expansion.
fn sorted_candidates(mut scored: Vec<ScoredNode<'_>>) -> Vec<RankedCandidate> {
    scored.sort_by(|a, b| {
        b.candidate
            .score
            .partial_cmp(&a.candidate.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.order.cmp(&b.order))
    });
    scored.into_iter().map(|scored| scored.candidate).collect()
}

/// Put ordinary and expanded candidates in the one deterministic order and
/// cut them to the channel's ceiling.
fn rank_order(
    mut ranked: Vec<RankedCandidate>,
    graph: &ResolvedGraph,
    dropped_terms: Vec<String>,
) -> ChannelRanking {
    let order_by_id: BTreeMap<&str, (&Path, usize)> = graph
        .nodes()
        .map(|node| {
            (
                node.id.as_str(),
                (node.path.as_path(), node.span.line_start),
            )
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                match (
                    order_by_id.get(a.id.as_str()),
                    order_by_id.get(b.id.as_str()),
                ) {
                    (Some(a_order), Some(b_order)) => a_order.cmp(b_order),
                    _ => a.id.cmp(&b.id),
                }
            })
    });
    ranked.truncate(MAX_SOURCE_CANDIDATES);
    ChannelRanking {
        candidates: ranked,
        dropped_terms,
    }
}

/// Score every symbol node of `graph` against the query for
/// [`Channel::Source`].
///
/// [`rank_source_channel`] without the corpus diagnostics, for callers that
/// only want the ordering.
pub fn rank_source(
    query: &RankQuery,
    graph: &ResolvedGraph,
    config: &RetrievalConfig,
) -> Vec<RankedCandidate> {
    rank_source_channel(query, graph, config).candidates
}

/// Score every node against the query, keeping each surviving candidate beside
/// the `(path, line_start)` key its ties break on.
fn score_nodes<'a>(
    query: &RankQuery,
    nodes: &[&'a SourceNode],
    corpus: &LexicalCorpus,
    gate: &ExactGate<'_>,
    config: &RetrievalConfig,
) -> Vec<ScoredNode<'a>> {
    let corpus_size = nodes.len() as f32;
    // Collected once per pass, not once per node: the candidacy test below asks
    // this set a question per name part, across every node in the graph.
    let surviving: BTreeSet<&str> = corpus
        .surviving_terms()
        .iter()
        .map(String::as_str)
        .collect();
    nodes
        .iter()
        .copied()
        .enumerate()
        .filter_map(|(index, node)| {
            score_node(
                query,
                node,
                corpus,
                gate,
                config,
                &surviving,
                corpus_size,
                index,
            )
            .map(|candidate| ScoredNode {
                candidate,
                order: (node.path.as_path(), node.span.line_start),
            })
        })
        .collect()
}

/// Score one node, returning `None` when no reason fired — a node nothing in
/// the query pointed at is not a candidate at all.
#[allow(clippy::too_many_arguments)]
fn score_node(
    query: &RankQuery,
    node: &SourceNode,
    corpus: &LexicalCorpus,
    gate: &ExactGate<'_>,
    config: &RetrievalConfig,
    surviving: &BTreeSet<&str>,
    corpus_size: f32,
    index: usize,
) -> Option<RankedCandidate> {
    let mut rungs = withhold_partial_coverage(node, score_exact_rungs(query, node, gate));
    // Deliberately after the coverage guard: partial extraction says nothing
    // about whether this file belongs to a stage we depend on, and the guard is
    // about high-confidence rungs, which this medium-tier one is not.
    if names_dependency_path(query, node) {
        rungs.award(BOOST_STAGE_DEPENDENCY, SelectionReason::StageDependency);
    }
    let (lexical_score, matched_term_count) = corpus.score(corpus_size, index);
    // A rung already established that the query pointed at this node, so its
    // lexical score rides along unexamined; without one, `candidacy` decides
    // whether ordinary words matching a ~10-token document mean anything.
    let lexical_admitted = matched_term_count > 0
        && (!rungs.is_empty() || admits_lexical_evidence(query, node, surviving, gate));
    if lexical_admitted {
        rungs.reasons.push(SelectionReason::Lexical);
        rungs.score += lexical_score;
    }
    if rungs.is_empty() {
        return None;
    }
    // Both read before `reasons` is moved out of `rungs` below.
    let confidence_ceiling = rungs.confidence_ceiling();
    let score = apply_test_path_factor(node, rungs.score, config);
    Some(RankedCandidate {
        id: ChunkId::from(node.id.as_str()),
        channel: Channel::Source,
        score,
        reasons: rungs.reasons,
        token_count: estimate_node_tokens(node),
        matched_term_count,
        confidence_ceiling,
    })
}

/// Exact-match rungs for one node: explicit id, exact path, exact symbol.
///
/// The rungs search the *raw* query text for the candidate's own string rather
/// than comparing tokens. [`tokenize`] splits on every non-identifier
/// character, so `Foo::Bar` becomes `foo, bar` and `src/context/pack.rs`
/// becomes `src, context, pack, rs` — token equality would match neither a
/// CamelCase symbol nor a whole path. Tokens feed BM25 and nothing else.
///
/// Finding the string is necessary but not sufficient: `gate` additionally
/// requires the occurrence in the prompt to LOOK like a code reference, which
/// is what stops "the point is" from claiming `type Point` and "write … in
/// /home/…" from claiming every `write` and `home` in the tree.
fn score_exact_rungs(query: &RankQuery, node: &SourceNode, gate: &ExactGate<'_>) -> RungScore {
    let mut rungs = RungScore::default();
    if query.required_ids.iter().any(|id| id == &node.id) {
        rungs.award(BOOST_EXPLICIT_ID, SelectionReason::ExplicitId);
    }
    match matches_path(&query.text, node, gate) {
        Some(PathMatch::FullPath) => rungs.award(BOOST_EXACT_PATH, SelectionReason::ExactPath),
        Some(PathMatch::Stem(evidence)) => {
            rungs.award_matched(BOOST_EXACT_PATH, SelectionReason::ExactPath, &evidence)
        }
        None => {}
    }
    if let Some(evidence) = node.scope.last().and_then(|terminal| gate.admits(terminal)) {
        rungs.award_matched(BOOST_EXACT_SYMBOL, SelectionReason::ExactSymbol, &evidence);
    }
    rungs
}

/// Withhold every high-confidence rung from a node whose file was not fully
/// extracted, leaving it to stand on whatever lexical score it earns.
///
/// [`crate::context::schema::Confidence::from_reasons`] classifies `High` from
/// *any* of `ExplicitId`, `ExactPath` or `ExactSymbol` — one `matches!` over
/// all three — so the guard has to drop all three together. Withholding only
/// the two match rungs while still emitting `ExplicitId` would leave the
/// invariant violated by the single most likely query shape: a caller passing
/// `--require-id` for a node in a partially-extracted file. A required id that
/// loses its rung is still ranked and can still appear; it simply cannot claim
/// that the extraction behind it was complete.
fn withhold_partial_coverage(node: &SourceNode, scored: RungScore) -> RungScore {
    if matches!(node.coverage, FileCoverage::Full) {
        return scored;
    }
    // Every rung score_exact_rungs awards is high-tier, so dropping them all
    // leaves no boost and no reason behind.
    RungScore::default()
}

/// Build one node's BM25 document from the text that identifies it: its scope
/// segments carry the symbol name and its owners, the signature the types and
/// parameters around them.
fn node_document(node: &SourceNode) -> Vec<(String, f32)> {
    let mut terms: Vec<(String, f32)> = node
        .scope
        .iter()
        .flat_map(|segment| tokenize(segment))
        .map(|term| (term, WEIGHT_SYMBOLS))
        .collect();
    terms.extend(
        tokenize(&node.signature)
            .into_iter()
            .map(|term| (term, WEIGHT_BODY)),
    );
    terms
}

/// Estimated token cost of rendering one source node.
///
/// A source item is a signature and a pointer, a few lines at most. The
/// constant covers the pointer: under-estimating here overfills the pack, so
/// the estimate deliberately rounds up.
pub(super) fn estimate_node_tokens(node: &SourceNode) -> usize {
    estimate_tokens(&node.signature) + 16
}

#[cfg(test)]
pub(super) fn expand_from_seeds_for_test(
    ranked: Vec<RankedCandidate>,
    graph: &ResolvedGraph,
    config: &RetrievalConfig,
) -> Vec<RankedCandidate> {
    expand_from_seeds(ranked, graph, config)
}
