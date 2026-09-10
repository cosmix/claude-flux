//! Cross-file symbol resolution and bounded reverse-impact traversal.
//!
//! Extraction is per file, so a call to a function defined elsewhere arrives
//! here as an [`EdgeProvenance::Inferred`] edge pointing at
//! [`UNRESOLVED_TARGET`](crate::context::source_graph::UNRESOLVED_TARGET): the
//! grammar that parsed the call site never saw the definition. This module is
//! the only place holding the whole graph at once, so it is the only place that
//! can turn some of those guesses into targets.
//!
//! **What it may claim.** An unresolved edge may be retargeted when the name it
//! recorded has exactly one definition in the entire graph, published at
//! [`UNIQUE_MATCH_CONFIDENCE`] — above the extraction-time ceiling, below
//! certainty. A call written with a path — `crate::a::b()`, `super::b()`,
//! `Widget::new()` — is matched on that path, which carries more evidence than
//! the bare name at its end: the qualified spelling is tried against every
//! node's scope, and failing that the qualifier is matched onto files and the
//! name is looked up only inside them. A qualifier naming nothing here is a call
//! into a dependency, so the edge stays a gap — resolving it against a namesake
//! elsewhere in the graph would be a fabrication. Imports are matched against
//! file paths by the rules in `paths`, and both only ever resolve when exactly
//! one candidate is left.
//!
//! **What it may never claim.**
//!
//! - *Never a complete call graph.* Resolution raises confidence where it can
//!   justify it and leaves the rest alone; [`ResolutionStats`] exists so a caller
//!   reports that residue instead of implying completeness.
//! - *Two candidates is ambiguity, not a coin flip.* Two definitions of one name
//!   leave the edge unresolved. Guessing would be indistinguishable from knowing,
//!   which is the failure this subsystem exists to avoid. The single exception is
//!   an `impl` block, which is indexed under its type's name without defining it
//!   (see `SymbolIndex::definitions`) — everything else contests.
//! - *A [`EdgeProvenance::Parser`] edge is never rewritten or downgraded, and
//!   nothing is ever promoted to `Parser`.* That provenance means one grammar saw
//!   both endpoints in one file; name matching cannot manufacture it.
//! - *A resolved edge stays `Inferred`.* Retargeting changes what an edge points
//!   at and how far to trust it, never what evidence produced it.
//!
//! [`impact`](fn@impact) walks the result backwards and reports, for every node
//! reached, the confidence of the *weakest* edge on the path taken — so a chain
//! passing through one guess is never presented as stronger than that guess. It
//! lives in the private `impact` submodule and is re-exported here, so every
//! caller keeps one path: `crate::context::resolve::impact`.

use crate::context::graph_store::ResolvedGraph;
use crate::context::source_graph::{EdgeProvenance, SourceEdge, SourceEdgeKind};

mod impact;
mod neighbors;
mod paths;
mod symbols;

pub use impact::{impact, impact_with, ImpactHit, ImpactOptions, ImpactResult};
pub use neighbors::{direct_callees, direct_callers, Neighbor};
pub(crate) use symbols::node_names;
pub use symbols::SymbolIndex;

use paths::{import_candidates, PathIndex};

/// Confidence for a cross-file name match that is unique in the whole graph.
///
/// Above [`MAX_INFERRED_CONFIDENCE`](crate::context::source_graph::MAX_INFERRED_CONFIDENCE)
/// because a definition unique across every indexed file is strictly stronger
/// evidence than the same-file guess that ceiling governs: the extractor could
/// see one file and had to hedge, while resolution has checked that no other
/// candidate exists anywhere.
///
/// Below 1.0 because a name match is still not a parse. It is wrong when a local
/// binding or a method on another type shadows the name at the call site; when
/// the real callee is chosen at run time through a trait object, virtual method,
/// or generic instantiation; when the name is re-exported and the definition
/// found is an alias rather than the body that runs; or when the true definition
/// lives in a dependency outside the graph and the in-graph match is a namesake.
pub const UNIQUE_MATCH_CONFIDENCE: f32 = 0.75;

/// What resolution did, so a view can report it instead of implying completeness.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResolutionStats {
    /// Unresolved edges retargeted onto a unique definition.
    pub retargeted: usize,
    /// Edges left unresolved because two or more definitions matched.
    pub ambiguous: usize,
    /// Edges still pointing at `UNRESOLVED_TARGET` after resolution.
    pub unresolved: usize,
}

/// What resolution did to one edge.
enum Outcome {
    Retargeted,
    Ambiguous,
    Unresolved,
}

/// Point `edge` at `target`, through the one primitive allowed to raise an
/// inferred edge above the extraction-time ceiling.
///
/// That ceiling governs a guess made from a single file; this match was checked
/// against the whole graph, which is why the raise is justified. Going through
/// `SourceEdge::resolve_to` rather than assigning the fields keeps the limit
/// structural: it clamps to `MAX_RESOLVED_INFERRED_CONFIDENCE`, keeps provenance
/// `Inferred` — a name match is not a parse — and refuses outright on a `Parser`
/// edge or an already-resolved one, so resolution can neither overwrite what the
/// grammar proved nor relitigate its own earlier decision.
///
/// Returns whether the edge was eligible; an ineligible edge is left untouched.
fn retarget(edge: &mut SourceEdge, target: &str) -> bool {
    edge.resolve_to(target, UNIQUE_MATCH_CONFIDENCE)
}

/// Targets `edge` could name, and whether its name was contested by two or more
/// candidates *before* `impl` blocks were filtered out — so that a name genuinely
/// fought over by several definitions still reports as ambiguous even when the
/// filter leaves nothing behind.
///
/// `from` is the path of the file the edge was extracted from, which is what a
/// relative path (`self::`, `super::`) is written against.
fn candidates_for(
    edge: &SourceEdge,
    from: &str,
    symbols: &SymbolIndex,
    paths: &PathIndex,
) -> (Vec<String>, bool) {
    match edge.kind {
        SourceEdgeKind::Calls | SourceEdgeKind::References if !edge.symbol.is_empty() => {
            call_candidates(&edge.symbol, from, symbols, paths)
        }
        SourceEdgeKind::Imports if !edge.symbol.is_empty() => {
            (import_candidates(&edge.symbol, from, paths), false)
        }
        // Containment, implementation and inheritance edges are emitted with both
        // endpoints inside one file. An unresolved one is a fact about the
        // extractor, not something a name match is entitled to repair.
        _ => (Vec::new(), false),
    }
}

/// Targets a call could name, strongest evidence first.
///
/// 1. The qualified spelling, longest first: `Widget::new` matches the `new`
///    inside `impl Widget` and no other, which is the whole reason the extractor
///    keeps the path.
/// 2. The qualifier as a module path, which scopes the search for the bare name
///    to the files it named: `crate::codex::run` is the `run` in `codex.rs`,
///    however many other `run`s the project has. A qualifier the graph cannot
///    place — `String::from` names a type this project never defined — leaves
///    the edge unresolved, because every `from` in the graph is then a namesake
///    rather than a candidate.
///
/// One qualifier is deliberately left unresolved: `Self::`. Nothing is indexed
/// under it, and the type it stands for is known only to the `impl` block the
/// call sits in — a fact the extractor drops. Those calls keep their edge and
/// their symbol, and stay part of the reported residue.
fn call_candidates(
    symbol: &str,
    from: &str,
    symbols: &SymbolIndex,
    paths: &PathIndex,
) -> (Vec<String>, bool) {
    let segments: Vec<&str> = symbol.split("::").collect();
    let Some((name, qualifier)) = segments.split_last().filter(|(_, rest)| !rest.is_empty()) else {
        return by_name(symbols, symbol);
    };

    for start in 0..qualifier.len() {
        let spelling = segments[start..].join("::");
        if !symbols.lookup(&spelling).is_empty() {
            return by_name(symbols, &spelling);
        }
    }

    // The files the qualifier names decide. A name none of them defines is
    // somewhere the path did not point — an external crate, or an item
    // re-exported from further down — so the definitions elsewhere in the graph
    // are namesakes rather than rival candidates, and the edge stays a gap.
    let module = import_candidates(&qualifier.join("::"), from, paths);
    let inside = symbols.definitions_in(name, &module);
    let contested = inside.len() >= 2;
    (inside, contested)
}

/// Definitions of one written name, with its contest status.
fn by_name(symbols: &SymbolIndex, name: &str) -> (Vec<String>, bool) {
    (symbols.definitions(name), symbols.contested(name))
}

/// Resolve one unresolved, inferred edge against the whole-graph indexes.
fn resolve_edge(
    edge: &mut SourceEdge,
    from: &str,
    symbols: &SymbolIndex,
    paths: &PathIndex,
) -> Outcome {
    let (candidates, contested) = candidates_for(edge, from, symbols, paths);
    match candidates.as_slice() {
        // `retarget` refusing means the edge was not the unresolved inferred
        // edge this pass is meant to act on. Report what actually happened
        // rather than claiming a retarget that did not occur.
        [only] if *only != edge.from => {
            if retarget(edge, only) {
                Outcome::Retargeted
            } else {
                Outcome::Unresolved
            }
        }
        // A lone candidate that is the edge's own origin resolves nothing, and
        // neither does an empty set — but either can still be a contested name.
        [_] | [] if !contested => Outcome::Unresolved,
        _ => Outcome::Ambiguous,
    }
}

/// Rewrite the unresolved edges of `graph` in place where the evidence justifies it.
pub fn resolve_graph(graph: &mut ResolvedGraph) -> ResolutionStats {
    let symbols = SymbolIndex::build(graph);
    let paths = PathIndex::build(graph);
    let mut stats = ResolutionStats::default();

    for (path, entry) in graph.files.iter_mut() {
        for edge in entry.edges.iter_mut() {
            // `Parser` — and the reserved `Lsp` — mean something stronger than a
            // name match already resolved both endpoints. Never rewritten, and
            // never counted: they are not part of the residue.
            if edge.provenance != EdgeProvenance::Inferred || !edge.is_unresolved() {
                continue;
            }
            match resolve_edge(edge, path, &symbols, &paths) {
                Outcome::Retargeted => stats.retargeted += 1,
                Outcome::Ambiguous => {
                    stats.ambiguous += 1;
                    stats.unresolved += 1;
                }
                Outcome::Unresolved => stats.unresolved += 1,
            }
        }
    }

    stats
}

#[cfg(test)]
pub(crate) mod fixtures;

#[cfg(test)]
#[path = "resolve/tests_resolve.rs"]
mod tests;

#[cfg(test)]
#[path = "resolve/tests_qualified.rs"]
mod tests_qualified;
