use super::*;
use crate::context::resolve::fixtures::*;
use crate::context::source_graph::{SourceEdge, UNRESOLVED_TARGET};

fn ids(neighbors: &[Neighbor]) -> Vec<&str> {
    neighbors
        .iter()
        .map(|neighbor| neighbor.id.as_str())
        .collect()
}

#[test]
fn callers_and_callees_follow_opposite_directions() {
    let caller = func_id("src/caller.rs", "caller");
    let callee = func_id("src/callee.rs", "callee");
    let graph = graph_from(vec![
        (
            "src/caller.rs",
            &["caller"],
            vec![SourceEdge::parser(
                &caller,
                &callee,
                SourceEdgeKind::Calls,
                "callee",
            )],
        ),
        ("src/callee.rs", &["callee"], vec![]),
    ]);

    let (callers, _) = direct_callers(&graph, &callee, 0);
    let (callees, _) = direct_callees(&graph, &caller, 0);

    assert_eq!(ids(&callers), vec![caller.as_str()]);
    assert_eq!(ids(&callees), vec![callee.as_str()]);
}

#[test]
fn unresolved_edges_and_file_endpoints_are_excluded() {
    let source = func_id("src/source.rs", "source");
    let target = func_id("src/target.rs", "target");
    let graph = graph_from(vec![
        (
            "src/source.rs",
            &["source"],
            vec![
                SourceEdge::parser(&source, &target, SourceEdgeKind::Calls, "target"),
                SourceEdge::unresolved(&source, SourceEdgeKind::Calls, "missing"),
                SourceEdge::parser(
                    "src/source.rs",
                    &target,
                    SourceEdgeKind::References,
                    "target",
                ),
                SourceEdge::parser(
                    &source,
                    "src/target.rs",
                    SourceEdgeKind::References,
                    "target.rs",
                ),
            ],
        ),
        ("src/target.rs", &["target"], vec![]),
    ]);

    let (callers, _) = direct_callers(&graph, &target, 0);
    let (callees, _) = direct_callees(&graph, &source, 0);

    assert_eq!(ids(&callers), vec![source.as_str()]);
    assert_eq!(ids(&callees), vec![target.as_str()]);
    assert!(direct_callers(&graph, "src/target.rs", 0).0.is_empty());
    assert!(direct_callees(&graph, UNRESOLVED_TARGET, 0).0.is_empty());
}

#[test]
fn neighbors_are_ordered_by_confidence_then_id() {
    let target = func_id("src/target.rs", "target");
    let graph = graph_from(vec![
        (
            "src/a.rs",
            &["a"],
            vec![edge_at(
                &func_id("src/a.rs", "a"),
                &target,
                SourceEdgeKind::Calls,
                EdgeProvenance::Parser,
                1.0,
            )],
        ),
        (
            "src/b.rs",
            &["b"],
            vec![edge_at(
                &func_id("src/b.rs", "b"),
                &target,
                SourceEdgeKind::References,
                EdgeProvenance::Parser,
                1.0,
            )],
        ),
        (
            "src/c.rs",
            &["c"],
            vec![edge_at(
                &func_id("src/c.rs", "c"),
                &target,
                SourceEdgeKind::Calls,
                EdgeProvenance::Inferred,
                0.5,
            )],
        ),
        ("src/target.rs", &["target"], vec![]),
    ]);

    let (neighbors, _) = direct_callers(&graph, &target, 0);

    assert_eq!(
        ids(&neighbors),
        vec![
            "src/a.rs#function:a",
            "src/b.rs#function:b",
            "src/c.rs#function:c"
        ]
    );
}

#[test]
fn a_neighbor_limit_reports_suppressed_count() {
    let target = func_id("src/target.rs", "target");
    let graph = graph_from(vec![
        (
            "src/a.rs",
            &["a"],
            vec![SourceEdge::parser(
                func_id("src/a.rs", "a"),
                &target,
                SourceEdgeKind::Calls,
                "target",
            )],
        ),
        (
            "src/b.rs",
            &["b"],
            vec![SourceEdge::parser(
                func_id("src/b.rs", "b"),
                &target,
                SourceEdgeKind::Calls,
                "target",
            )],
        ),
        ("src/target.rs", &["target"], vec![]),
    ]);

    let (neighbors, suppressed) = direct_callers(&graph, &target, 1);

    assert_eq!(ids(&neighbors), vec!["src/a.rs#function:a"]);
    assert_eq!(suppressed, 1);
}
