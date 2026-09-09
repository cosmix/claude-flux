//! Bounded source-graph expansion from exact-rung ranking seeds.

use super::{estimate_node_tokens, paths::apply_test_path_factor};
use crate::context::config::RetrievalConfig;
use crate::context::graph_store::ResolvedGraph;
use crate::context::rank::{RankQuery, RankedCandidate};
use crate::context::schema::{
    Channel, ChunkId, Confidence, FileCoverage, SelectionReason, SourceNodeKind,
};
use crate::context::source_graph::{SourceEdge, SourceEdgeKind};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const MAX_EXPANSION_SEEDS: usize = 5;
pub(super) const MAX_NEIGHBORS_PER_SEED: usize = 4;
pub(super) const MAX_EXPANDED: usize = 12;
pub(super) const NEIGHBOR_SCORE_FACTOR: f32 = 0.2;
pub(super) const MIN_NEIGHBOR_EDGE_CONFIDENCE: f32 = 0.5;

type Adjacency<'a> = BTreeMap<&'a str, Vec<&'a SourceEdge>>;

/// Add graph neighbours of the strongest exact-rung candidates as tier-2 candidates.
/// `ranked` is the channel's scored list BEFORE truncation to `MAX_SOURCE_CANDIDATES`,
/// sorted strongest first. Returns the list with neighbours appended (unsorted).
pub(super) fn expand_from_seeds(
    mut ranked: Vec<RankedCandidate>,
    graph: &ResolvedGraph,
    _query: &RankQuery,
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

    let mut existing: BTreeSet<String> = ranked
        .iter()
        .map(|candidate| candidate.id.as_str().to_string())
        .collect();
    let (forward, reverse) = build_adjacencies(graph);
    let mut expanded = 0;
    for (seed_id, seed_score) in seeds {
        if expanded == MAX_EXPANDED {
            break;
        }
        let neighbours = neighbours_for_seed(&seed_id, &forward, &reverse);
        append_neighbours(
            neighbours,
            seed_score,
            graph,
            config,
            &mut ranked,
            &mut existing,
            &mut expanded,
        );
    }
    ranked
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

fn build_adjacencies(graph: &ResolvedGraph) -> (Adjacency<'_>, Adjacency<'_>) {
    let mut forward: Adjacency<'_> = BTreeMap::new();
    let mut reverse: Adjacency<'_> = BTreeMap::new();
    for edge in graph.edges() {
        forward.entry(edge.from.as_str()).or_default().push(edge);
        reverse.entry(edge.to.as_str()).or_default().push(edge);
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

#[allow(clippy::too_many_arguments)]
fn append_neighbours(
    neighbours: Vec<(&str, f32)>,
    seed_score: f32,
    graph: &ResolvedGraph,
    config: &RetrievalConfig,
    ranked: &mut Vec<RankedCandidate>,
    existing: &mut BTreeSet<String>,
    expanded: &mut usize,
) {
    let mut added_for_seed = 0;
    for (id, _) in neighbours {
        if added_for_seed == MAX_NEIGHBORS_PER_SEED || *expanded == MAX_EXPANDED {
            break;
        }
        let Some(node) = graph.node(id) else {
            continue;
        };
        if existing.contains(id)
            || matches!(node.kind, SourceNodeKind::File)
            || !matches!(node.coverage, FileCoverage::Full)
        {
            continue;
        }
        ranked.push(RankedCandidate {
            id: ChunkId::from(id),
            channel: Channel::Source,
            score: apply_test_path_factor(node, seed_score * NEIGHBOR_SCORE_FACTOR, config),
            reasons: vec![SelectionReason::GraphNeighbor],
            token_count: estimate_node_tokens(node),
            matched_term_count: 0,
            confidence_ceiling: Some(Confidence::Medium),
        });
        existing.insert(id.to_string());
        added_for_seed += 1;
        *expanded += 1;
    }
}
