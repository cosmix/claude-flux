//! Fusion tests: how an expanded graph neighbour's score interacts with
//! `fuse`'s exact-rung floor once a high-scoring seed pulls it in.
//!
//! Split out of `rank_source_expand.rs` (already near its own line budget)
//! into a sibling wired the same way `brief.rs` wires `brief_tests.rs`:
//! `#[path = "rank_source_expand_fusion.rs"] mod fusion_tests;`.

use super::{candidate, expand, function, graph_with_edges, NEIGHBOUR, SEED};
use crate::context::fuse::fuse;
use crate::context::rank::BOOST_EXACT_SYMBOL;
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
