//! Tests for the bounded reverse traversal and its minimum-confidence rule.

use super::*;
use crate::context::resolve::fixtures::*;
use crate::context::resolve::UNIQUE_MATCH_CONFIDENCE;
use crate::context::source_graph::UNRESOLVED_TARGET;

/// A fully-parsed call edge, the trustworthy baseline for traversal tests.
fn call(from: &str, to: &str) -> Vec<SourceEdge> {
    vec![edge_at(
        from,
        to,
        SourceEdgeKind::Calls,
        EdgeProvenance::Parser,
        1.0,
    )]
}

fn ids(hits: &[ImpactHit]) -> Vec<&str> {
    hits.iter().map(|hit| hit.id.as_str()).collect()
}

/// `a -> b -> c -> d` as parser call edges, each stored with its caller. Every
/// file holds one function named after it, so `b` lives in `src/b.rs`.
fn chain() -> ResolvedGraph {
    let link = |from: &str, to: &str| {
        call(
            &func_id(&format!("src/{from}.rs"), from),
            &func_id(&format!("src/{to}.rs"), to),
        )
    };
    graph_from(vec![
        ("src/a.rs", &["a"], link("a", "b")),
        ("src/b.rs", &["b"], link("b", "c")),
        ("src/c.rs", &["c"], link("c", "d")),
        ("src/d.rs", &["d"], vec![]),
    ])
}

#[test]
fn impact_stops_at_max_depth() {
    let graph = chain();
    let hits = impact(&graph, &func_id("src/d.rs", "d"), 2);

    assert_eq!(
        ids(&hits),
        vec![
            func_id("src/c.rs", "c").as_str(),
            func_id("src/b.rs", "b").as_str()
        ]
    );
    assert_eq!(hits[0].depth, 1);
    assert_eq!(hits[1].depth, 2);
    assert_eq!(hits[0].kind, SourceNodeKind::Function);
    assert_eq!(hits[0].path, PathBuf::from("src/c.rs"));
}

#[test]
fn impact_of_depth_zero_reaches_nothing() {
    let graph = chain();
    assert!(impact(&graph, &func_id("src/d.rs", "d"), 0).is_empty());
}

/// `w -> x -> y -> z`, with the three hops at descending confidence
/// 0.75, 0.5, 1.0 — deliberately NOT monotonic, so a path-confidence rule that
/// took the last hop or multiplied the chain would give a different answer from
/// the minimum at every node.
fn descending_confidence_chain() -> crate::context::graph_store::ResolvedGraph {
    let hop =
        |from: &str,
         to: &str,
         kind: SourceEdgeKind,
         provenance: EdgeProvenance,
         confidence: f32| { vec![edge_at(from, to, kind, provenance, confidence)] };
    graph_from(vec![
        (
            "src/w.rs",
            &["w"],
            hop(
                "src/w.rs#function:w",
                "src/x.rs#function:x",
                SourceEdgeKind::References,
                EdgeProvenance::Inferred,
                UNIQUE_MATCH_CONFIDENCE,
            ),
        ),
        (
            "src/x.rs",
            &["x"],
            hop(
                "src/x.rs#function:x",
                "src/y.rs#function:y",
                SourceEdgeKind::Imports,
                EdgeProvenance::Inferred,
                0.5,
            ),
        ),
        (
            "src/y.rs",
            &["y"],
            hop(
                "src/y.rs#function:y",
                "src/z.rs#function:z",
                SourceEdgeKind::Calls,
                EdgeProvenance::Parser,
                1.0,
            ),
        ),
        ("src/z.rs", &["z"], vec![]),
    ])
}

#[test]
fn path_confidence_is_the_minimum_not_the_product() {
    let graph = descending_confidence_chain();

    let hits = impact(&graph, "src/z.rs#function:z", 3);

    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0].id, "src/y.rs#function:y");
    assert_eq!(hits[0].min_confidence, 1.0);
    assert_eq!(hits[0].weakest_provenance, EdgeProvenance::Parser);

    assert_eq!(hits[1].id, "src/x.rs#function:x");
    assert_eq!(hits[1].min_confidence, 0.5);

    let farthest = &hits[2];
    assert_eq!(farthest.id, "src/w.rs#function:w");
    assert_eq!(
        farthest.min_confidence, 0.5,
        "0.5 is the weakest step; 0.375 would be a product and 0.75 the last hop"
    );
    assert_eq!(farthest.weakest_provenance, EdgeProvenance::Inferred);
    assert_eq!(farthest.weakest_kind, SourceEdgeKind::Imports);
}

#[test]
fn a_cycle_terminates_and_reports_each_node_once() {
    let graph = graph_from(vec![
        ("src/a.rs", &[], call("src/a.rs", "src/b.rs")),
        ("src/b.rs", &[], call("src/b.rs", "src/c.rs")),
        ("src/c.rs", &[], call("src/c.rs", "src/a.rs")),
    ]);

    let hits = impact(&graph, "src/a.rs", 10);

    assert_eq!(
        ids(&hits),
        vec!["src/c.rs", "src/b.rs"],
        "the start node is the subject of the query, never a result"
    );
}

#[test]
fn impact_ignores_unresolved_edges_and_absent_origins() {
    let graph = graph_from(vec![(
        "src/app.rs",
        &[],
        vec![
            SourceEdge::unresolved("src/app.rs", SourceEdgeKind::Calls, "mystery"),
            edge_at(
                "src/deleted.rs",
                "src/app.rs",
                SourceEdgeKind::Calls,
                EdgeProvenance::Parser,
                1.0,
            ),
        ],
    )]);

    assert!(
        impact(&graph, UNRESOLVED_TARGET, 3).is_empty(),
        "the unresolved placeholder must not become a hub joining every guess"
    );
    assert!(
        impact(&graph, "src/app.rs", 3).is_empty(),
        "an edge from a node the graph does not contain reaches nothing"
    );
}

#[test]
fn default_options_skip_contains_and_imports_edges() {
    let target = func_id("src/target.rs", "target");
    let called = func_id("src/called.rs", "called");
    let contained = func_id("src/contained.rs", "contained");
    let imported = func_id("src/imported.rs", "imported");
    let graph = graph_from(vec![
        (
            "src/called.rs",
            &["called"],
            vec![edge_at(
                &called,
                &target,
                SourceEdgeKind::Calls,
                EdgeProvenance::Parser,
                1.0,
            )],
        ),
        (
            "src/contained.rs",
            &["contained"],
            vec![edge_at(
                &contained,
                &target,
                SourceEdgeKind::Contains,
                EdgeProvenance::Parser,
                1.0,
            )],
        ),
        (
            "src/imported.rs",
            &["imported"],
            vec![edge_at(
                &imported,
                &target,
                SourceEdgeKind::Imports,
                EdgeProvenance::Parser,
                1.0,
            )],
        ),
        ("src/target.rs", &["target"], vec![]),
    ]);

    let result = impact_with(&graph, &target, &ImpactOptions::default());

    assert_eq!(ids(&result.hits), vec![called.as_str()]);
}

#[test]
fn a_kind_filter_is_applied_while_traversing_not_after() {
    let target = func_id("src/target.rs", "target");
    let bridge = func_id("src/bridge.rs", "bridge");
    let far = func_id("src/far.rs", "far");
    let graph = graph_from(vec![
        (
            "src/far.rs",
            &["far"],
            vec![edge_at(
                &far,
                &bridge,
                SourceEdgeKind::Calls,
                EdgeProvenance::Inferred,
                0.5,
            )],
        ),
        (
            "src/bridge.rs",
            &["bridge"],
            vec![edge_at(
                &bridge,
                &target,
                SourceEdgeKind::References,
                EdgeProvenance::Parser,
                1.0,
            )],
        ),
        ("src/target.rs", &["target"], vec![]),
    ]);
    let options = ImpactOptions {
        kinds: vec![SourceEdgeKind::Calls],
        ..ImpactOptions::default()
    };

    let result = impact_with(&graph, &target, &options);

    assert!(result.hits.is_empty());
}

#[test]
fn a_limit_reports_how_many_hits_it_suppressed() {
    let target = func_id("src/target.rs", "target");
    let graph = graph_from(vec![
        ("src/a.rs", &["a"], call(&func_id("src/a.rs", "a"), &target)),
        ("src/b.rs", &["b"], call(&func_id("src/b.rs", "b"), &target)),
        ("src/c.rs", &["c"], call(&func_id("src/c.rs", "c"), &target)),
        ("src/target.rs", &["target"], vec![]),
    ]);
    let options = ImpactOptions {
        limit: 2,
        ..ImpactOptions::default()
    };

    let result = impact_with(&graph, &target, &options);

    assert_eq!(
        ids(&result.hits),
        vec!["src/a.rs#function:a", "src/b.rs#function:b"]
    );
    assert_eq!(result.suppressed, 1);
}

#[test]
fn a_path_prefix_keeps_only_matching_hits() {
    let target = func_id("src/target.rs", "target");
    let graph = graph_from(vec![
        ("src/a.rs", &["a"], call(&func_id("src/a.rs", "a"), &target)),
        (
            "tests/b.rs",
            &["b"],
            call(&func_id("tests/b.rs", "b"), &target),
        ),
        ("src/target.rs", &["target"], vec![]),
    ]);
    let options = ImpactOptions {
        path_prefix: Some("tests/".to_string()),
        ..ImpactOptions::default()
    };

    let result = impact_with(&graph, &target, &options);

    assert_eq!(ids(&result.hits), vec!["tests/b.rs#function:b"]);
    assert_eq!(result.suppressed, 0);
}

#[test]
fn a_min_confidence_stops_traversal_through_weak_edges() {
    let target = func_id("src/target.rs", "target");
    let bridge = func_id("src/bridge.rs", "bridge");
    let far = func_id("src/far.rs", "far");
    let graph = graph_from(vec![
        ("src/far.rs", &["far"], call(&far, &bridge)),
        (
            "src/bridge.rs",
            &["bridge"],
            vec![edge_at(
                &bridge,
                &target,
                SourceEdgeKind::Calls,
                EdgeProvenance::Inferred,
                0.4,
            )],
        ),
        ("src/target.rs", &["target"], vec![]),
    ]);
    let options = ImpactOptions {
        min_confidence: 0.5,
        ..ImpactOptions::default()
    };

    let result = impact_with(&graph, &target, &options);

    assert!(result.hits.is_empty());
}
