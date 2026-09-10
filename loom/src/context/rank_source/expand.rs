//! Bounded source-graph expansion from exact-rung ranking seeds.

use super::{estimate_node_tokens, paths::apply_test_path_factor};
use crate::context::config::RetrievalConfig;
use crate::context::graph_store::ResolvedGraph;
use crate::context::rank::{RankedCandidate, BOOST_EXACT_SYMBOL};
use crate::context::schema::{
    Channel, ChunkId, Confidence, FileCoverage, SelectionReason, SourceNode, SourceNodeKind,
};
use crate::context::source_graph::{SourceEdge, SourceEdgeKind};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const MAX_EXPANSION_SEEDS: usize = 5;
pub(super) const MAX_NEIGHBORS_PER_SEED: usize = 4;
pub(super) const MAX_EXPANDED: usize = 12;
pub(super) const NEIGHBOR_SCORE_FACTOR: f32 = 0.2;
pub(super) const MIN_NEIGHBOR_EDGE_CONFIDENCE: f32 = 0.5;

/// Neighbours examined per seed before the loop moves on, independent of how
/// many of them are actually accepted.
///
/// `MAX_NEIGHBORS_PER_SEED` bounds acceptances, but a seed's neighbour list can
/// be mostly rejections — already a candidate, a `File` node, partial coverage
/// — and none of those rejections used to stop the scan. This bounds the scan
/// itself, on the sorted-by-confidence list `neighbours_for_seed` already
/// produces, so truncating it still keeps the strongest neighbours.
pub(super) const MAX_EXAMINED_NEIGHBORS_PER_SEED: usize = 32;

/// Ceiling on a neighbour's score: at most the weakest a genuine exact rung
/// can ever score, so a node that merely neighbours a seed can never outrank
/// a node the query matched directly.
///
/// [`BOOST_EXACT_SYMBOL`] (80.0) is not itself that floor. `score_node`
/// (`rank_source.rs:281`) applies `config.test_path_factor` to a node's WHOLE
/// score as its last step, so a genuine `ExactSymbol` hit in a test file
/// scores as low as `BOOST_EXACT_SYMBOL * config.test_path_factor` (32.0 at
/// the 0.4 default) — well below 80. A flat cap near 80 let a neighbour of an
/// `ExplicitId` or `ExactPath` seed (`BOOST_EXPLICIT_ID` = 1000,
/// `BOOST_EXACT_PATH` = 100) outrank exactly that kind of direct hit.
/// Deriving the cap from the same factor keeps the invariant true regardless
/// of which file the direct match lives in.
fn max_neighbor_score(config: &RetrievalConfig) -> f32 {
    BOOST_EXACT_SYMBOL * config.test_path_factor
}

type Adjacency<'a> = BTreeMap<&'a str, Vec<&'a SourceEdge>>;

/// Each seed's score paired with the neighbours it reached, in the order the
/// accept walk examines them. Named because the pair of that list and the id
/// set it covers is `resolve_seed_neighbours`'s whole output.
type SeedNeighbours<'a> = Vec<(f32, Vec<(&'a str, f32)>)>;

/// Mutable state threaded through one expansion pass: the growing candidate
/// list, the id set it already carries, how many neighbours have been
/// accepted overall, and an id index over the graph so accepting a neighbour
/// costs a map probe rather than a scan of every node.
struct Expansion<'a> {
    ranked: Vec<RankedCandidate>,
    existing: BTreeSet<String>,
    expanded: usize,
    nodes_by_id: BTreeMap<&'a str, &'a SourceNode>,
}

/// Add graph neighbours of the strongest exact-rung candidates as tier-2 candidates.
/// `ranked` is the channel's scored list BEFORE truncation to `MAX_SOURCE_CANDIDATES`,
/// sorted strongest first. Returns the list with neighbours appended (unsorted).
pub(super) fn expand_from_seeds(
    ranked: Vec<RankedCandidate>,
    graph: &ResolvedGraph,
    config: &RetrievalConfig,
) -> Vec<RankedCandidate> {
    let seeds: Vec<(String, f32)> = ranked
        .iter()
        .filter(|candidate| has_seed_reason(&candidate.reasons))
        .take(MAX_EXPANSION_SEEDS)
        .map(|candidate| (candidate.id.as_str().to_string(), candidate.score))
        .collect();
    if seeds.is_empty() {
        return ranked;
    }

    let existing: BTreeSet<String> = ranked
        .iter()
        .map(|candidate| candidate.id.as_str().to_string())
        .collect();

    let seed_ids: BTreeSet<&str> = seeds.iter().map(|(id, _)| id.as_str()).collect();
    let (forward, reverse) = build_seed_adjacencies(graph, &seed_ids);
    let (per_seed, wanted_ids) = resolve_seed_neighbours(&seeds, &forward, &reverse);
    let nodes_by_id = index_wanted_nodes(graph, &wanted_ids);

    let mut state = Expansion {
        ranked,
        existing,
        expanded: 0,
        nodes_by_id,
    };
    for (seed_score, neighbours) in per_seed {
        if state.expanded == MAX_EXPANDED {
            break;
        }
        append_neighbours(neighbours, seed_score, config, &mut state);
    }
    state.ranked
}

/// Resolve every seed's neighbour ids before touching `graph.nodes()`, so the
/// id index `index_wanted_nodes` builds only has to cover ids the traversal
/// can actually reach — at most `MAX_EXPANSION_SEEDS *
/// MAX_EXAMINED_NEIGHBORS_PER_SEED` — instead of every node in the graph.
/// Split out of `expand_from_seeds` (expand.rs:62) to keep it under the
/// function line limit.
fn resolve_seed_neighbours<'a>(
    seeds: &[(String, f32)],
    forward: &Adjacency<'a>,
    reverse: &Adjacency<'a>,
) -> (SeedNeighbours<'a>, BTreeSet<&'a str>) {
    let mut per_seed: SeedNeighbours = Vec::with_capacity(seeds.len());
    let mut wanted_ids: BTreeSet<&str> = BTreeSet::new();
    for (seed_id, seed_score) in seeds {
        let neighbours: Vec<(&str, f32)> = neighbours_for_seed(seed_id, forward, reverse)
            .into_iter()
            .take(MAX_EXAMINED_NEIGHBORS_PER_SEED)
            .collect();
        wanted_ids.extend(neighbours.iter().map(|(id, _)| *id));
        per_seed.push((*seed_score, neighbours));
    }
    (per_seed, wanted_ids)
}

/// Index just the nodes `resolve_seed_neighbours` found reachable, rather
/// than every node the graph holds. Split out of `expand_from_seeds`
/// (expand.rs:62) to keep it under the function line limit.
fn index_wanted_nodes<'a>(
    graph: &'a ResolvedGraph,
    wanted_ids: &BTreeSet<&str>,
) -> BTreeMap<&'a str, &'a SourceNode> {
    graph
        .nodes()
        .filter(|node| wanted_ids.contains(node.id.as_str()))
        .map(|node| (node.id.as_str(), node))
        .collect()
}

fn has_seed_reason(reasons: &[SelectionReason]) -> bool {
    reasons.iter().any(|reason| {
        matches!(
            reason,
            SelectionReason::ExplicitId
                | SelectionReason::ExactPath
                | SelectionReason::ExactSymbol
                | SelectionReason::StageDependency
        )
    })
}

/// Forward/reverse adjacency restricted to edges that touch a seed.
///
/// A retrieval pass has at most [`MAX_EXPANSION_SEEDS`] seeds, so scanning
/// every node and every edge in the graph to answer at most 5 lookups wastes
/// work on the prompt-hook path the lexical cache exists to keep cheap. One
/// pass over `graph.edges()`, keeping only the edges whose `from` or `to` is a
/// seed, does the same job without a `Vec` entry for every node the graph
/// actually has.
fn build_seed_adjacencies<'a>(
    graph: &'a ResolvedGraph,
    seed_ids: &BTreeSet<&str>,
) -> (Adjacency<'a>, Adjacency<'a>) {
    let mut forward: Adjacency<'a> = BTreeMap::new();
    let mut reverse: Adjacency<'a> = BTreeMap::new();
    for edge in graph.edges() {
        if seed_ids.contains(edge.from.as_str()) {
            forward.entry(edge.from.as_str()).or_default().push(edge);
        }
        if seed_ids.contains(edge.to.as_str()) {
            reverse.entry(edge.to.as_str()).or_default().push(edge);
        }
    }
    (forward, reverse)
}

fn neighbours_for_seed<'a>(
    seed_id: &str,
    forward: &Adjacency<'a>,
    reverse: &Adjacency<'a>,
) -> Vec<(&'a str, f32)> {
    let mut neighbours = Vec::new();
    for edge in forward.get(seed_id).into_iter().flatten() {
        if eligible_edge(edge) {
            neighbours.push((edge.to.as_str(), edge.confidence));
        }
    }
    for edge in reverse.get(seed_id).into_iter().flatten() {
        if eligible_edge(edge) {
            neighbours.push((edge.from.as_str(), edge.confidence));
        }
    }
    neighbours.sort_by(|(a_id, a_confidence), (b_id, b_confidence)| {
        b_confidence
            .total_cmp(a_confidence)
            .then_with(|| a_id.cmp(b_id))
    });
    neighbours
}

fn eligible_edge(edge: &SourceEdge) -> bool {
    matches!(
        edge.kind,
        SourceEdgeKind::Calls
            | SourceEdgeKind::Implements
            | SourceEdgeKind::Extends
            | SourceEdgeKind::References
    ) && !edge.is_unresolved()
        && edge.confidence >= MIN_NEIGHBOR_EDGE_CONFIDENCE
}

fn append_neighbours(
    neighbours: Vec<(&str, f32)>,
    seed_score: f32,
    config: &RetrievalConfig,
    state: &mut Expansion<'_>,
) {
    let mut added_for_seed = 0;
    for (id, _) in neighbours {
        if added_for_seed == MAX_NEIGHBORS_PER_SEED || state.expanded == MAX_EXPANDED {
            break;
        }
        // Cheapest rejection first: a set probe, before the node lookup.
        if state.existing.contains(id) {
            continue;
        }
        let Some(node) = state.nodes_by_id.get(id).copied() else {
            continue;
        };
        if matches!(node.kind, SourceNodeKind::File) || !matches!(node.coverage, FileCoverage::Full)
        {
            continue;
        }
        let score = apply_test_path_factor(
            node,
            (seed_score * NEIGHBOR_SCORE_FACTOR).min(max_neighbor_score(config)),
            config,
        );
        state.ranked.push(RankedCandidate {
            id: ChunkId::from(id),
            channel: Channel::Source,
            score,
            reasons: vec![SelectionReason::GraphNeighbor],
            token_count: estimate_node_tokens(node),
            matched_term_count: 0,
            confidence_ceiling: Some(Confidence::Medium),
        });
        state.existing.insert(id.to_string());
        added_for_seed += 1;
        state.expanded += 1;
    }
}
