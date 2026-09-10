//! Machine-readable counterparts of the text map views.

use std::path::Path;

use serde_json::{json, Map, Value};

use crate::context::graph_store::ResolvedGraph;
use crate::context::resolve::{
    direct_callees, direct_callers, impact_with, ImpactOptions, Neighbor,
};
use crate::context::source_graph::{FileCoverage, SourceNode, SourceNodeKind};
use crate::context::{CoverageReport, ResolutionStats};

use super::{
    effective_impact_kinds, find_symbol_matches, project_relative, resolve_starts, ImpactArgs,
    FIND_ALL_CAP, IMPACT_MAX_STARTS,
};

pub fn outline_json(graph: &ResolvedGraph, project_root: &Path, arg: &str) -> Value {
    let rel = project_relative(project_root, arg).unwrap_or_else(|| arg.to_string());
    let path = safe(&rel);
    let Some(entry) = graph.files.get(&rel) else {
        return json!({"path": path, "error": "no indexed file"});
    };

    let mut symbols = entry
        .nodes
        .iter()
        .filter(|node| node.kind != SourceNodeKind::File)
        .collect::<Vec<_>>();
    symbols.sort_by_key(|node| node.span.start_byte);
    json!({
        "path": path,
        "coverage": coverage_json(&entry.coverage),
        "symbols": symbols.into_iter().map(symbol_json).collect::<Vec<_>>(),
    })
}

pub fn find_all_json(graph: &ResolvedGraph, symbol: &str) -> Value {
    let (mut matches, label) = find_symbol_matches(graph, symbol);
    matches.sort_by(|a, b| (&a.path, a.span.line_start).cmp(&(&b.path, b.span.line_start)));
    let suppressed = matches.len().saturating_sub(FIND_ALL_CAP);
    json!({
        "symbol": safe(symbol),
        "exact": label.is_empty(),
        "matches": matches.into_iter().take(FIND_ALL_CAP).map(find_match_json).collect::<Vec<_>>(),
        "suppressed": suppressed,
    })
}

pub fn impact_json(
    graph: &ResolvedGraph,
    project_root: &Path,
    arg: &str,
    _stats: &ResolutionStats,
    args: &ImpactArgs,
) -> Value {
    let resolved = resolve_starts(graph, project_root, arg);
    let shown_starts = resolved
        .starts
        .iter()
        .take(IMPACT_MAX_STARTS)
        .collect::<Vec<_>>();
    let mut suppressed = resolved.starts.len().saturating_sub(shown_starts.len());
    let mut hits = Vec::new();
    for start_id in &shown_starts {
        let result = impact_with(
            graph,
            start_id,
            &ImpactOptions {
                max_depth: args.depth,
                kinds: args.kinds.clone(),
                limit: args.limit,
                path_prefix: args.path_prefix.clone(),
                min_confidence: args.min_confidence,
            },
        );
        suppressed += result.suppressed;
        hits.extend(result.hits.iter().map(impact_hit_json));
    }
    json!({
        "start_ids": shown_starts.into_iter().map(|id| safe(id)).collect::<Vec<_>>(),
        "depth": args.depth,
        "kinds": effective_impact_kinds(&args.kinds).iter().map(|kind| kind.as_str()).collect::<Vec<_>>(),
        "hits": hits,
        "suppressed": suppressed,
    })
}

pub fn callers_json(graph: &ResolvedGraph, project_root: &Path, arg: &str, limit: usize) -> Value {
    neighbors_json(graph, project_root, arg, limit, direct_callers)
}

pub fn callees_json(graph: &ResolvedGraph, project_root: &Path, arg: &str, limit: usize) -> Value {
    neighbors_json(graph, project_root, arg, limit, direct_callees)
}

pub fn footer_json(graph: &ResolvedGraph, stats: &ResolutionStats) -> Value {
    let coverage = CoverageReport::of(graph);
    json!({
        "files": coverage.files,
        "files_by_status": coverage.files_by_status,
        "symbol_level_files": coverage.symbol_level_files,
        "nodes": coverage.nodes,
        "edges": coverage.edges,
        "edges_by_provenance": coverage.edges_by_provenance,
        "unresolved_edges": coverage.unresolved_edges,
        "base_revision": safe(&coverage.base_revision),
        "overlaid_files": coverage.overlaid_files,
        "resolution": {
            "retargeted": stats.retargeted,
            "ambiguous": stats.ambiguous,
            "unresolved": stats.unresolved,
        },
    })
}

fn neighbors_json(
    graph: &ResolvedGraph,
    project_root: &Path,
    arg: &str,
    limit: usize,
    query: fn(&ResolvedGraph, &str, usize) -> (Vec<Neighbor>, usize),
) -> Value {
    let resolved = resolve_starts(graph, project_root, arg);
    let shown_starts = resolved
        .starts
        .iter()
        .take(IMPACT_MAX_STARTS)
        .collect::<Vec<_>>();
    let mut suppressed = resolved.starts.len().saturating_sub(shown_starts.len());
    let mut neighbors = Vec::new();
    for start_id in &shown_starts {
        let (found, omitted) = query(graph, start_id, limit);
        suppressed += omitted;
        neighbors.extend(found.iter().map(neighbor_json));
    }
    json!({
        "start_ids": shown_starts.into_iter().map(|id| safe(id)).collect::<Vec<_>>(),
        "neighbors": neighbors,
        "suppressed": suppressed,
    })
}

fn symbol_json(node: &SourceNode) -> Value {
    json!({
        "id": safe(&node.id),
        "kind": node.kind.as_str(),
        "scope": node.scope.iter().map(|segment| safe(segment)).collect::<Vec<_>>(),
        "line_start": node.span.line_start,
        "line_end": node.span.line_end,
        "signature": safe(&node.signature),
    })
}

fn find_match_json(node: &SourceNode) -> Value {
    json!({
        "id": safe(&node.id),
        "kind": node.kind.as_str(),
        "path": safe(&node.path.to_string_lossy()),
        "line_start": node.span.line_start,
        "coverage_status": node.coverage.status(),
    })
}

fn impact_hit_json(hit: &crate::context::resolve::ImpactHit) -> Value {
    json!({
        "id": safe(&hit.id),
        "kind": hit.kind.as_str(),
        "path": safe(&hit.path.to_string_lossy()),
        "depth": hit.depth,
        "min_confidence": hit.min_confidence,
        "weakest_provenance": hit.weakest_provenance.as_str(),
        "weakest_kind": hit.weakest_kind.as_str(),
    })
}

fn neighbor_json(neighbor: &Neighbor) -> Value {
    json!({
        "id": safe(&neighbor.id),
        "kind": neighbor.kind.as_str(),
        "path": safe(&neighbor.path),
        "edge_kind": neighbor.edge_kind.as_str(),
        "confidence": neighbor.confidence,
        "provenance": neighbor.provenance.as_str(),
        "line_start": neighbor.line_start,
    })
}

fn coverage_json(coverage: &FileCoverage) -> Value {
    let mut value = Map::new();
    value.insert("status".to_string(), json!(coverage.status()));
    let detail = match coverage {
        FileCoverage::Partial { detail }
        | FileCoverage::LexicalOnly { detail }
        | FileCoverage::ParseError { detail, .. } => Some(safe(detail)),
        FileCoverage::Oversized { bytes, limit } => Some(format!("{bytes} bytes (limit {limit})")),
        FileCoverage::Deleted | FileCoverage::Full => None,
    };
    if let Some(detail) = detail {
        value.insert("detail".to_string(), json!(detail));
    }
    Value::Object(value)
}

fn safe(value: &str) -> String {
    crate::context::untrusted::inline_safe(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::views::tests::{impact_args, impact_chain_graph};
    use std::collections::BTreeSet;
    use tempfile::TempDir;

    fn round_trip(value: Value) -> Value {
        serde_json::from_str(&serde_json::to_string(&value).unwrap()).unwrap()
    }

    fn assert_keys(value: &Value, expected: &[&str]) {
        let actual = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected.iter().copied().collect::<BTreeSet<_>>());
    }

    #[test]
    fn outline_and_find_all_key_sets_are_stable() {
        let graph = impact_chain_graph();
        let root = TempDir::new().unwrap();
        let outline = round_trip(outline_json(&graph, root.path(), "src/a.rs"));
        let missing = round_trip(outline_json(&graph, root.path(), "src/missing.rs"));
        let find_all = round_trip(find_all_json(&graph, "foo"));

        assert_keys(&outline, &["path", "coverage", "symbols"]);
        assert_keys(&outline["coverage"], &["status"]);
        assert_keys(
            &outline["symbols"][0],
            &["id", "kind", "scope", "line_start", "line_end", "signature"],
        );
        assert_keys(&missing, &["path", "error"]);
        assert_keys(&find_all, &["symbol", "exact", "matches", "suppressed"]);
        assert_keys(
            &find_all["matches"][0],
            &["id", "kind", "path", "line_start", "coverage_status"],
        );
    }

    #[test]
    fn impact_and_neighbor_key_sets_are_stable() {
        let graph = impact_chain_graph();
        let root = TempDir::new().unwrap();
        let stats = ResolutionStats::default();
        let args = impact_args(Vec::new());
        let impact = round_trip(impact_json(&graph, root.path(), "foo", &stats, &args));
        let callers = round_trip(callers_json(&graph, root.path(), "foo", 0));
        let callees = round_trip(callees_json(&graph, root.path(), "bar", 0));

        assert_keys(
            &impact,
            &["start_ids", "depth", "kinds", "hits", "suppressed"],
        );
        assert_keys(
            &impact["hits"][0],
            &[
                "id",
                "kind",
                "path",
                "depth",
                "min_confidence",
                "weakest_provenance",
                "weakest_kind",
            ],
        );
        for value in [&callers, &callees] {
            assert_keys(value, &["start_ids", "neighbors", "suppressed"]);
            assert_keys(
                &value["neighbors"][0],
                &[
                    "id",
                    "kind",
                    "path",
                    "edge_kind",
                    "confidence",
                    "provenance",
                    "line_start",
                ],
            );
        }
    }

    #[test]
    fn footer_key_set_is_stable() {
        let graph = impact_chain_graph();
        let footer = round_trip(footer_json(&graph, &ResolutionStats::default()));

        assert_keys(
            &footer,
            &[
                "files",
                "files_by_status",
                "symbol_level_files",
                "nodes",
                "edges",
                "edges_by_provenance",
                "unresolved_edges",
                "base_revision",
                "overlaid_files",
                "resolution",
            ],
        );
        assert_keys(
            &footer["resolution"],
            &["retargeted", "ambiguous", "unresolved"],
        );
    }
}
