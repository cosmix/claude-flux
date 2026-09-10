//! Promotion tests: when the detail takes the summary's ranked slot, whether
//! that costs it anything, and what a cheaper candidate ranked between them
//! can lose to it.
//!
//! Split out of `pack_twins.rs` (which stayed near its own line budget) into
//! a sibling wired the same way `brief.rs` wires `brief_tests.rs`:
//! `#[path = "pack_twins_promotion.rs"] mod promotion_tests;`.

use super::{candidate, chunk, packed_ids, TIER1, TIER2};

/// Look-ahead: the summary outranking its own detail must not turn the pack
/// into the pointer without the text.
#[test]
fn a_summary_ranked_above_its_detail_yields_its_slot_to_the_detail() {
    let ranked = vec![
        candidate(TIER1, 2.0, 2),
        candidate("conventions.md#unrelated#0", 1.5, 2),
        candidate(TIER2, 1.0, 4),
    ];
    let chunks = [
        chunk(TIER1, 2),
        chunk("conventions.md#unrelated#0", 2),
        chunk(TIER2, 4),
    ];

    assert_eq!(
        packed_ids(100, &ranked, &chunks),
        vec![TIER2.to_string(), "conventions.md#unrelated#0".to_string()],
        "the detail takes the summary's position, the rest keep their order"
    );
}

/// Promotion must not cost the reader the fallback: a detail that cannot fit,
/// pulled up to a summary that can, still leaves the summary behind it.
#[test]
fn a_promoted_detail_that_does_not_fit_leaves_the_summary_behind_it() {
    let ranked = vec![candidate(TIER1, 2.0, 2), candidate(TIER2, 1.0, 40)];
    let chunks = [chunk(TIER1, 2), chunk(TIER2, 40)];

    assert_eq!(packed_ids(5, &ranked, &chunks), vec![TIER1.to_string()]);
}

#[test]
fn an_unrelated_pair_sharing_an_anchor_is_packed_whole() {
    let ranked = vec![
        candidate("architecture/overview.md#overview#0", 2.0, 4),
        candidate("conventions.md#overview#0", 1.0, 2),
    ];
    let chunks = [
        chunk("architecture/overview.md#overview#0", 4),
        chunk("conventions.md#overview#0", 2),
    ];

    assert_eq!(
        packed_ids(100, &ranked, &chunks),
        vec![
            "architecture/overview.md#overview#0".to_string(),
            "conventions.md#overview#0".to_string()
        ]
    );
}

/// Promotion is not free. The detail takes the summary's slot, so under a
/// tight budget it can cost a cheaper candidate that ranked between them the
/// room it would otherwise have had. Pinned rather than fixed: the reader
/// asked about this topic and the detail IS the topic, but a future change
/// must not flip the trade-off without saying so.
#[test]
fn a_promoted_detail_can_cost_a_cheaper_later_candidate_its_slot() {
    let ranked = vec![
        candidate(TIER1, 2.0, 2),
        candidate("conventions.md#unrelated#0", 1.5, 2),
        candidate(TIER2, 1.0, 4),
    ];
    let chunks = [
        chunk(TIER1, 2),
        chunk("conventions.md#unrelated#0", 2),
        chunk(TIER2, 4),
    ];

    assert_eq!(
        packed_ids(5, &ranked, &chunks),
        vec![TIER2.to_string()],
        "the detail fits where the summary and the unrelated chunk together did"
    );
}

/// Two tier-2 files can repeat one heading, and both then compute the same
/// tier-1 twin. The highest-ranked of them is the one promoted into the
/// summary's slot, and neither is deduplicated against the other — they are
/// different topics that happen to share a title.
#[test]
fn the_highest_ranked_of_two_details_sharing_a_heading_is_the_one_promoted() {
    let alpha = "mistakes/alpha.md#shared-heading#0";
    let beta = "mistakes/beta.md#shared-heading#0";
    let summary = "mistakes.md#shared-heading#0";
    let ranked = vec![
        candidate(summary, 3.0, 2),
        candidate(alpha, 2.0, 2),
        candidate(beta, 1.0, 2),
    ];
    let chunks = [chunk(summary, 2), chunk(alpha, 2), chunk(beta, 2)];

    assert_eq!(
        packed_ids(100, &ranked, &chunks),
        vec![alpha.to_string(), beta.to_string()],
        "alpha outranks beta, so alpha takes the summary's slot"
    );
}
