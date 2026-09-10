//! Fusion tests: how an expanded graph neighbour's score interacts with
//! `fuse`'s exact-rung floor once a high-scoring seed pulls it in.
//!
//! Split out of `rank_source_expand.rs` (already near its own line budget)
//! into a sibling wired the same way `brief.rs` wires `brief_tests.rs`:
//! `#[path = "rank_source_expand_fusion.rs"] mod fusion_tests;`.

use super::{candidate, expand, full_node, function, graph_with_edges, NEIGHBOUR, SEED};
use crate::context::config::RetrievalConfig;
use crate::context::fuse::fuse;
use crate::context::graph_store::ResolvedGraph;
use crate::context::rank::{RankQuery, BOOST_EXACT_SYMBOL};
use crate::context::rank_source::rank_source;
use crate::context::schema::SelectionReason;
use crate::context::source_graph::{SourceEdge, SourceEdgeKind};

#[test]
fn a_graph_neighbour_fuses_in_tier_two_below_every_exact_rung_candidate() {
    let low_exact = "src/exact.rs#function:exact";
    let fixture = graph_with_edges(
        vec![
            function(SEED, "src/seed.rs"),
            function(low_exact, "src/exact.rs"),
            function(NEIGHBOUR, "src/neighbour.rs"),
        ],
        vec![SourceEdge::parser(
            SEED,
            NEIGHBOUR,
            SourceEdgeKind::Calls,
            "neighbour",
        )],
    );
    let ranked = vec![
        candidate(SEED, 100.0, SelectionReason::ExactSymbol),
        candidate(low_exact, 1.0, SelectionReason::ExactPath),
    ];

    let fused = fuse(&[expand(ranked, &fixture)]);
    let ids: Vec<&str> = fused.iter().map(|item| item.id.as_str()).collect();

    assert_eq!(ids, vec![SEED, low_exact, NEIGHBOUR]);
}

/// Fixture for `a_graph_neighbour_never_outranks_a_genuine_exact_symbol_hit_in_a_test_file`
/// (rank_source_expand_fusion.rs:44): a hub matched as a required id (so it
/// earns BOOST_EXPLICIT_ID = 1000.0) with one graph neighbour, alongside a
/// genuine exact-symbol hit under `tests/` — the weakest a direct hit can
/// score once `test_path_factor` damps it. Split out to keep the test itself
/// arrange-call-assert.
fn hub_neighbour_and_test_exact_fixture() -> (ResolvedGraph, RankQuery, &'static str) {
    let hub = "src/hub.rs#function:Hub";
    let test_exact = "tests/exact.rs#function:ExactThing";
    let fixture = graph_with_edges(
        vec![
            full_node(hub, "src/hub.rs", &["Hub"], "fn Hub()"),
            function(NEIGHBOUR, "src/neighbour.rs"),
            full_node(
                test_exact,
                "tests/exact.rs",
                &["ExactThing"],
                "fn ExactThing()",
            ),
        ],
        vec![SourceEdge::parser(
            hub,
            NEIGHBOUR,
            SourceEdgeKind::Calls,
            "neighbour",
        )],
    );
    let query = RankQuery {
        text: "inspect `ExactThing`".to_string(),
        required_ids: vec![hub.to_string()],
        ..RankQuery::default()
    };
    (fixture, query, test_exact)
}

#[test]
fn a_graph_neighbour_never_outranks_a_genuine_exact_symbol_hit_in_a_test_file() {
    // `hub` earns BOOST_EXPLICIT_ID (1000.0) as a required id, so its
    // uncapped neighbour would score `1000.0 * NEIGHBOR_SCORE_FACTOR ==
    // 200.0` - the scenario `expand.rs`'s cap exists to prevent. `test_exact`
    // earns only BOOST_EXACT_SYMBOL (80.0), damped by the default
    // `test_path_factor` (0.4) because its file lives under `tests/` - the
    // weakest a genuine exact rung can score, and the cap the neighbour must
    // never clear.
    let (fixture, query, test_exact) = hub_neighbour_and_test_exact_fixture();

    let ranked = rank_source(&query, &fixture, &RetrievalConfig::default());

    let exact_score = ranked
        .iter()
        .find(|item| item.id.as_str() == test_exact)
        .expect("direct exact-symbol hit must still be a candidate")
        .score;
    let neighbour_score = ranked
        .iter()
        .find(|item| item.id.as_str() == NEIGHBOUR)
        .expect("graph neighbour must still be a candidate")
        .score;

    assert!(
        neighbour_score <= exact_score,
        "graph neighbour ({neighbour_score}) outranked a direct exact-symbol \
         hit in a test file ({exact_score})"
    );
}

#[test]
fn a_neighbour_of_a_high_scoring_seed_never_reaches_the_weakest_exact_rung() {
    let fixture = graph_with_edges(
        vec![
            function(SEED, "src/seed.rs"),
            function(NEIGHBOUR, "src/neighbour.rs"),
        ],
        vec![SourceEdge::parser(
            SEED,
            NEIGHBOUR,
            SourceEdgeKind::Calls,
            "neighbour",
        )],
    );
    // Without a cap this seed's neighbour would score
    // `1000.0 * NEIGHBOR_SCORE_FACTOR == 200.0`, well above `BOOST_EXACT_SYMBOL`
    // (80.0) - a node the query never matched would then outrank one it
    // matched exactly.
    let expanded = expand(
        vec![candidate(SEED, 1000.0, SelectionReason::ExplicitId)],
        &fixture,
    );
    let neighbour = &expanded[1];

    assert!(neighbour.score < BOOST_EXACT_SYMBOL);
    assert!(neighbour.score > 0.0);
}

#[test]
fn a_neighbour_of_a_high_scoring_seed_still_fuses_below_a_weak_exact_rung_candidate() {
    let low_exact = "src/exact.rs#function:exact";
    let fixture = graph_with_edges(
        vec![
            function(SEED, "src/seed.rs"),
            function(low_exact, "src/exact.rs"),
            function(NEIGHBOUR, "src/neighbour.rs"),
        ],
        vec![SourceEdge::parser(
            SEED,
            NEIGHBOUR,
            SourceEdgeKind::Calls,
            "neighbour",
        )],
    );
    let ranked = vec![
        candidate(SEED, 1000.0, SelectionReason::ExplicitId),
        candidate(low_exact, BOOST_EXACT_SYMBOL, SelectionReason::ExactSymbol),
    ];

    let fused = fuse(&[expand(ranked, &fixture)]);
    let ids: Vec<&str> = fused.iter().map(|item| item.id.as_str()).collect();

    assert_eq!(ids, vec![SEED, low_exact, NEIGHBOUR]);
}
