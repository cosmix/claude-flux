//! Bounded reverse traversal: who reaches a node, and how far to trust the claim.
//!
//! Edges point caller -> callee, so "what breaks if I change this?" is a walk
//! *backwards*: from a node to the `from` of every edge whose `to` is that node.
//! The walk is breadth-first and depth-bounded, and a node is emitted once.
//!
//! ## Trust is the minimum, never a product
//!
//! Each hit reports the confidence of the **weakest** edge on the path taken,
//! along with that edge's provenance and kind. A path is worth exactly its worst
//! step: one guess anywhere in a chain caps everything beyond it, and a caller
//! that sees `0.5 / inferred` knows which link to distrust. Multiplying
//! confidences would instead punish long chains for their length — five fully
//! parsed hops would decay below a single guess, which is precisely backwards.
//!
//! ## What a traversal may not claim
//!
//! Reachability here is reachability *in the derived graph*, which is not the
//! program. Edges the extractors could not resolve are dropped before the walk
//! rather than treated as edges to an "unresolved" hub, so an absent path means
//! "not found", never "does not exist".

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::context::graph_store::ResolvedGraph;
use crate::context::source_graph::{
    EdgeProvenance, SourceEdge, SourceEdgeKind, SourceNode, SourceNodeKind,
};

/// One node reached by a reverse traversal, with the trust of the weakest step.
#[derive(Debug, Clone, PartialEq)]
pub struct ImpactHit {
    pub id: String,
    pub kind: SourceNodeKind,
    pub path: PathBuf,
    /// Hops from the start node; 1 is a direct dependent.
    pub depth: usize,
    /// MINIMUM confidence along the path taken to reach this node.
    pub min_confidence: f32,
    /// Provenance of the weakest edge on that path.
    pub weakest_provenance: EdgeProvenance,
    /// Kind of the weakest edge on that path.
    pub weakest_kind: SourceEdgeKind,
}

/// Filters and bounds for one reverse-impact traversal.
#[derive(Debug, Clone, PartialEq)]
pub struct ImpactOptions {
    pub max_depth: usize,
    /// An empty list walks every semantic dependency kind, excluding file
    /// containment and imports. A non-empty list is an explicit allow-list.
    pub kinds: Vec<SourceEdgeKind>,
    /// Zero is unlimited; otherwise only the first `limit` sorted hits remain.
    pub limit: usize,
    pub path_prefix: Option<String>,
    pub min_confidence: f32,
}

impl Default for ImpactOptions {
    fn default() -> Self {
        Self {
            max_depth: 3,
            kinds: Vec::new(),
            limit: 0,
            path_prefix: None,
            min_confidence: 0.0,
        }
    }
}

/// Reverse-impact hits plus the number removed by [`ImpactOptions::limit`].
#[derive(Debug, Clone, PartialEq)]
pub struct ImpactResult {
    pub hits: Vec<ImpactHit>,
    pub suppressed: usize,
}

/// The weakest edge seen so far along one path.
#[derive(Debug, Clone, Copy)]
struct Trust {
    min_confidence: f32,
    provenance: EdgeProvenance,
    kind: SourceEdgeKind,
}

impl Trust {
    /// Extend a path by `edge`. `None` is the start node, which has no path yet.
    fn extend(current: Option<Trust>, edge: &SourceEdge) -> Trust {
        match current {
            Some(trust) if trust.min_confidence <= edge.confidence => trust,
            _ => Trust {
                min_confidence: edge.confidence,
                provenance: edge.provenance,
                kind: edge.kind,
            },
        }
    }
}

impl From<&ImpactHit> for Trust {
    fn from(hit: &ImpactHit) -> Self {
        Trust {
            min_confidence: hit.min_confidence,
            provenance: hit.weakest_provenance,
            kind: hit.weakest_kind,
        }
    }
}

type Reverse<'a> = BTreeMap<&'a str, Vec<&'a SourceEdge>>;
type Nodes<'a> = BTreeMap<&'a str, &'a SourceNode>;
type Frontier<'a> = Vec<(&'a str, Option<Trust>)>;

/// Incoming edges keyed by target, so a level step does not rescan the graph.
/// Unresolved edges are dropped: they name no target worth walking back from.
fn reverse_adjacency(graph: &ResolvedGraph) -> Reverse<'_> {
    let mut reverse: Reverse<'_> = BTreeMap::new();
    for edge in graph.edges().filter(|edge| !edge.is_unresolved()) {
        reverse.entry(edge.to.as_str()).or_default().push(edge);
    }
    reverse
}

/// Breadth-first state for one [`impact`] query.
struct Walk<'a> {
    start: &'a str,
    hits: Vec<ImpactHit>,
    seen: BTreeMap<&'a str, usize>,
}

impl<'a> Walk<'a> {
    /// Step one level outward, returning the next frontier with each newly
    /// reached node's best-known trust.
    fn expand(
        &mut self,
        frontier: &Frontier<'a>,
        depth: usize,
        reverse: &Reverse<'a>,
        nodes: &Nodes<'a>,
        options: &ImpactOptions,
    ) -> Frontier<'a> {
        let mut discovered: Vec<&'a str> = Vec::new();
        for (id, carried) in frontier {
            for &edge in reverse.get(*id).into_iter().flatten() {
                if !edge_is_allowed(edge, options) {
                    continue;
                }
                let from = edge.from.as_str();
                // The start node is the query's subject, never a result; skipping
                // it is also what makes a cycle back to it terminate.
                if from == self.start {
                    continue;
                }
                let Some(&node) = nodes.get(from) else {
                    continue;
                };
                if self.record(node, depth, Trust::extend(*carried, edge)) {
                    discovered.push(from);
                }
            }
        }
        // Read after the level is complete, so each node propagates its settled
        // best trust rather than whichever path happened to reach it first.
        let mut next: Frontier<'a> = Vec::new();
        for id in discovered {
            if let Some(hit) = self.hit(id) {
                next.push((id, Some(Trust::from(hit))));
            }
        }
        next
    }

    fn hit(&self, id: &str) -> Option<&ImpactHit> {
        self.seen.get(id).and_then(|index| self.hits.get(*index))
    }

    /// Emit `node` at `depth`, or improve the trust already recorded for it.
    /// Returns true only the first time the walk reaches it.
    fn record(&mut self, node: &'a SourceNode, depth: usize, trust: Trust) -> bool {
        if let Some(&index) = self.seen.get(node.id.as_str()) {
            if let Some(hit) = self.hits.get_mut(index) {
                // Reaching a node again at the same depth by a more trusted path
                // improves what we report but never re-expands it — that is what
                // bounds a cyclic graph.
                if hit.depth == depth && trust.min_confidence > hit.min_confidence {
                    hit.min_confidence = trust.min_confidence;
                    hit.weakest_provenance = trust.provenance;
                    hit.weakest_kind = trust.kind;
                }
            }
            return false;
        }
        self.seen.insert(node.id.as_str(), self.hits.len());
        self.hits.push(ImpactHit {
            id: node.id.clone(),
            kind: node.kind,
            path: node.path.clone(),
            depth,
            min_confidence: trust.min_confidence,
            weakest_provenance: trust.provenance,
            weakest_kind: trust.kind,
        });
        true
    }

    /// Nearest first, then most trusted, then by id, so output is stable.
    fn finish(mut self) -> Vec<ImpactHit> {
        self.hits.sort_by(|a, b| {
            a.depth
                .cmp(&b.depth)
                .then_with(|| b.min_confidence.total_cmp(&a.min_confidence))
                .then_with(|| a.id.cmp(&b.id))
        });
        self.hits
    }
}

/// Apply edge filters while expanding the graph. Filtering here, rather than
/// retaining hits afterward, prevents a rejected edge from becoming a bridge
/// to otherwise-matching nodes farther away.
fn edge_is_allowed(edge: &SourceEdge, options: &ImpactOptions) -> bool {
    let kind_allowed = if options.kinds.is_empty() {
        !matches!(
            edge.kind,
            SourceEdgeKind::Contains | SourceEdgeKind::Imports
        )
    } else {
        options.kinds.contains(&edge.kind)
    };
    kind_allowed && edge.confidence >= options.min_confidence
}

/// Breadth-first reverse traversal with edge, path, confidence, and result bounds.
pub fn impact_with(graph: &ResolvedGraph, start_id: &str, options: &ImpactOptions) -> ImpactResult {
    if options.max_depth == 0 {
        return ImpactResult {
            hits: Vec::new(),
            suppressed: 0,
        };
    }
    let nodes: Nodes<'_> = graph.nodes().map(|node| (node.id.as_str(), node)).collect();
    let reverse = reverse_adjacency(graph);
    let mut walk = Walk {
        start: start_id,
        hits: Vec::new(),
        seen: BTreeMap::new(),
    };

    let mut frontier: Frontier<'_> = vec![(start_id, None)];
    for depth in 1..=options.max_depth {
        let next = walk.expand(&frontier, depth, &reverse, &nodes, options);
        if next.is_empty() {
            break;
        }
        frontier = next;
    }

    let mut hits = walk.finish();
    if let Some(prefix) = &options.path_prefix {
        hits.retain(|hit| hit.path.to_string_lossy().starts_with(prefix));
    }
    let suppressed = if options.limit > 0 && hits.len() > options.limit {
        let suppressed = hits.len() - options.limit;
        hits.truncate(options.limit);
        suppressed
    } else {
        0
    };
    ImpactResult { hits, suppressed }
}

/// Compatibility traversal retaining the original all-edge-kinds semantics.
pub fn impact(graph: &ResolvedGraph, start_id: &str, max_depth: usize) -> Vec<ImpactHit> {
    let options = ImpactOptions {
        max_depth,
        kinds: vec![
            SourceEdgeKind::Contains,
            SourceEdgeKind::Imports,
            SourceEdgeKind::Calls,
            SourceEdgeKind::References,
            SourceEdgeKind::Implements,
            SourceEdgeKind::Extends,
        ],
        ..ImpactOptions::default()
    };
    impact_with(graph, start_id, &options).hits
}

#[cfg(test)]
#[path = "tests_impact.rs"]
mod tests;
