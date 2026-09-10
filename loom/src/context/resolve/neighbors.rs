//! Direct resolved symbol neighbors for callers and callees views.

use std::collections::BTreeMap;

use crate::context::graph_store::ResolvedGraph;
use crate::context::source_graph::{EdgeProvenance, SourceEdgeKind, SourceNode, SourceNodeKind};

/// One symbol joined to the query node by one resolved semantic edge.
#[derive(Debug, Clone, PartialEq)]
pub struct Neighbor {
    pub id: String,
    pub kind: SourceNodeKind,
    pub path: String,
    pub edge_kind: SourceEdgeKind,
    pub confidence: f32,
    pub provenance: EdgeProvenance,
    pub line_start: Option<usize>,
}

/// Symbol nodes with a resolved Calls/References/Implements/Extends edge into `node_id`.
pub fn direct_callers(
    graph: &ResolvedGraph,
    node_id: &str,
    limit: usize,
) -> (Vec<Neighbor>, usize) {
    direct_neighbors(graph, node_id, limit, Direction::Incoming)
}

/// Symbol nodes such an edge out of `node_id` reaches.
pub fn direct_callees(
    graph: &ResolvedGraph,
    node_id: &str,
    limit: usize,
) -> (Vec<Neighbor>, usize) {
    direct_neighbors(graph, node_id, limit, Direction::Outgoing)
}

#[derive(Clone, Copy)]
enum Direction {
    Incoming,
    Outgoing,
}

fn direct_neighbors(
    graph: &ResolvedGraph,
    node_id: &str,
    limit: usize,
    direction: Direction,
) -> (Vec<Neighbor>, usize) {
    let nodes = graph
        .nodes()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let Some(start) = nodes.get(node_id).copied().filter(|node| is_symbol(node)) else {
        return (Vec::new(), 0);
    };

    let mut neighbors = collect_neighbors(graph, &nodes, start, direction);
    neighbors.sort_by(|a, b| {
        b.confidence
            .total_cmp(&a.confidence)
            .then_with(|| a.id.cmp(&b.id))
    });
    let suppressed = suppress_beyond_limit(&mut neighbors, limit);
    (neighbors, suppressed)
}

fn collect_neighbors(
    graph: &ResolvedGraph,
    nodes: &BTreeMap<&str, &SourceNode>,
    start: &SourceNode,
    direction: Direction,
) -> Vec<Neighbor> {
    graph
        .edges()
        .filter(|edge| !edge.is_unresolved() && is_neighbor_kind(edge.kind))
        .filter_map(|edge| {
            let endpoint = match direction {
                Direction::Incoming if edge.to == start.id => {
                    nodes.get(edge.from.as_str()).copied()
                }
                Direction::Outgoing if edge.from == start.id => {
                    nodes.get(edge.to.as_str()).copied()
                }
                _ => None,
            }?;
            is_symbol(endpoint).then(|| Neighbor {
                id: endpoint.id.clone(),
                kind: endpoint.kind,
                path: endpoint.path.to_string_lossy().into_owned(),
                edge_kind: edge.kind,
                confidence: edge.confidence,
                provenance: edge.provenance,
                line_start: Some(endpoint.span.line_start),
            })
        })
        .collect()
}

/// Drop everything past `limit`, reporting how many were dropped. `limit ==
/// 0` means unlimited.
fn suppress_beyond_limit(neighbors: &mut Vec<Neighbor>, limit: usize) -> usize {
    if limit > 0 && neighbors.len() > limit {
        let suppressed = neighbors.len() - limit;
        neighbors.truncate(limit);
        suppressed
    } else {
        0
    }
}

fn is_neighbor_kind(kind: SourceEdgeKind) -> bool {
    matches!(
        kind,
        SourceEdgeKind::Calls
            | SourceEdgeKind::References
            | SourceEdgeKind::Implements
            | SourceEdgeKind::Extends
    )
}

fn is_symbol(node: &SourceNode) -> bool {
    node.kind != SourceNodeKind::File
}

#[cfg(test)]
#[path = "tests_neighbors.rs"]
mod tests;
