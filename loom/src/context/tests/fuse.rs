use super::fuse_fixtures::{candidate, find, tied_tier2_fixture};
use crate::context::fuse::*;
use crate::context::schema::*;

#[test]
fn rule_20_rrf_uses_one_based_positions_within_each_list() {
    let fused = fuse(&[vec![
        candidate("first", Channel::Knowledge, 99.0, vec![], 1),
        candidate("second", Channel::Knowledge, 1.0, vec![], 1),
    ]]);
    let second = find(&fused, "second");
    assert!(
        (second.score - 0.016_129).abs() < 1e-4,
        "got {}",
        second.score
    );
}

#[test]
fn rule_21_rrf_sums_contributions_across_lists() {
    let fused = fuse(&[
        vec![candidate("shared", Channel::Knowledge, 1.0, vec![], 1)],
        vec![
            candidate("other", Channel::Source, 100.0, vec![], 1),
            candidate("shared", Channel::Source, 0.0, vec![], 1),
        ],
    ]);
    assert!(
        (fused[0].score - 0.032_522).abs() < 1e-4,
        "got {}",
        fused[0].score
    );
}

#[test]
fn rule_22_fusion_keeps_best_channel_and_first_metadata() {
    // Both occurrences are lexical-only, so "shared" stays tier 2 and this
    // exercises the same cross-channel merge mechanics fusion has always had:
    // channel tracks the best RRF rank, but other metadata (token_count)
    // comes from whichever list inserted the id first.
    let fused = fuse(&[
        vec![
            candidate(
                "other",
                Channel::Knowledge,
                2.0,
                vec![SelectionReason::Lexical],
                1,
            ),
            candidate(
                "shared",
                Channel::Knowledge,
                1.0,
                vec![SelectionReason::Lexical],
                5,
            ),
        ],
        vec![candidate(
            "shared",
            Channel::Source,
            1.0,
            vec![SelectionReason::Lexical],
            9,
        )],
    ]);
    let shared = find(&fused, "shared");
    assert!(
        (shared.score - 0.032_522).abs() < 1e-4,
        "got {}",
        shared.score
    );
    assert_eq!(shared.channel, Channel::Source);
    assert_eq!(shared.token_count, 5);
    assert_eq!(shared.reasons, vec![SelectionReason::Lexical]);
}

/// Regression case from the retrieval-precision proposal: a knowledge chunk
/// with an exact rung must precede a purely lexical source node regardless of
/// which id sorts alphabetically first. The chunk id is chosen so plain
/// alphabetical order would put it SECOND.
#[test]
fn rule_23_tier1_precedes_tier2_regardless_of_alphabetical_id_order() {
    let fused = fuse(&[
        vec![candidate(
            "zzz-chunk",
            Channel::Knowledge,
            1080.0,
            vec![SelectionReason::ExactPath, SelectionReason::Lexical],
            1,
        )],
        vec![candidate(
            "aaa-source",
            Channel::Source,
            0.3,
            vec![SelectionReason::Lexical],
            1,
        )],
    ]);
    assert_eq!(
        fused
            .iter()
            .map(|candidate| candidate.id.as_str())
            .collect::<Vec<_>>(),
        vec!["zzz-chunk", "aaa-source"]
    );
    assert_eq!(fused[0].score, 1080.0, "tier 1 carries its raw score");
    assert!(
        (fused[1].score - 0.016_393).abs() < 1e-4,
        "tier 2 carries its RRF score, got {}",
        fused[1].score
    );
}

#[test]
fn graph_neighbor_stays_in_tier2_below_a_lower_scoring_exact_symbol() {
    let fused = fuse(&[vec![
        candidate(
            "graph-neighbor",
            Channel::Source,
            100.0,
            vec![SelectionReason::GraphNeighbor],
            1,
        ),
        candidate(
            "exact-symbol",
            Channel::Source,
            1.0,
            vec![SelectionReason::ExactSymbol],
            1,
        ),
    ]]);

    assert_eq!(
        fused
            .iter()
            .map(|candidate| candidate.id.as_str())
            .collect::<Vec<_>>(),
        vec!["exact-symbol", "graph-neighbor"]
    );
}

#[test]
fn rule_24_empty_input_and_empty_lists_produce_no_candidates() {
    assert!(fuse(&[]).is_empty());
    assert!(fuse(&[Vec::new(), Vec::new()]).is_empty());
}

/// The alphabetical-bias regression case for tier 2: two candidates that both
/// land at rank 1 within their own channel's tier-2 list (after that
/// channel's exact-rung anchor is pulled into tier 1) tie on RRF score, and
/// the one with the higher within-channel normalized score must win --
/// regardless of id order.
#[test]
fn rule_25_tier2_ties_break_by_within_channel_normalized_score() {
    let fused = fuse(&tied_tier2_fixture());
    let a_lex = find(&fused, "a-lex");
    let b_lex = find(&fused, "b-lex");
    assert!(
        (a_lex.score - b_lex.score).abs() < 1e-6,
        "RRF scores should tie: a={} b={}",
        a_lex.score,
        b_lex.score
    );
    let ids: Vec<&str> = fused
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect();
    let a_pos = ids.iter().position(|id| *id == "a-lex").unwrap();
    let b_pos = ids.iter().position(|id| *id == "b-lex").unwrap();
    assert!(
        b_pos < a_pos,
        "b-lex has the higher normalized score (0.1 vs 0.005) and must sort first: {ids:?}"
    );
}

#[test]
fn rule_26_all_lexical_input_degenerates_to_pure_tier2() {
    let fused = fuse(&[
        vec![
            candidate(
                "first",
                Channel::Knowledge,
                9.0,
                vec![SelectionReason::Lexical],
                1,
            ),
            candidate(
                "second",
                Channel::Knowledge,
                3.0,
                vec![SelectionReason::Lexical],
                1,
            ),
        ],
        vec![candidate(
            "third",
            Channel::Source,
            1.0,
            vec![SelectionReason::Lexical],
            1,
        )],
    ]);
    // "first" (Knowledge rank 1) and "third" (Source rank 1) both contribute
    // 1/61 -- identical rank-1 RRF scores across channels, which is inherent
    // to position-based RRF, not something tiering changes -- and both
    // normalize to 1.0 within their own channel (each is that channel's only
    // candidate), so the tie falls through to id ascending: "first" precedes
    // "third", exactly as plain id-ascending tie-break would have ordered
    // them before this change. "second" (Knowledge rank 2, 1/62) is strictly
    // behind both.
    assert_eq!(
        fused
            .iter()
            .map(|candidate| candidate.id.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "third", "second"],
        "today's RRF order, id-ascending tie-break unchanged for this case"
    );
}

/// Cross-channel merge: an id lexical-only in one channel and exact-rung in
/// the other must merge into a single tier-1 output item, with reasons
/// unioned (no duplicates) and channel/score taken from the higher-scoring,
/// better-ranked occurrence.
#[test]
fn rule_27_cross_channel_merge_promotes_lexical_only_occurrence_to_tier1() {
    let fused = fuse(&[
        vec![
            candidate(
                "k-anchor",
                Channel::Knowledge,
                5.0,
                vec![SelectionReason::Lexical],
                1,
            ),
            candidate(
                "dual",
                Channel::Knowledge,
                0.5,
                vec![SelectionReason::Lexical],
                1,
            ),
        ],
        vec![candidate(
            "dual",
            Channel::Source,
            100.0,
            vec![SelectionReason::ExactPath, SelectionReason::Lexical],
            1,
        )],
    ]);
    let dual = find(&fused, "dual");
    assert_eq!(
        dual.score, 100.0,
        "raw score maximum across channels, not the sum"
    );
    assert_eq!(dual.channel, Channel::Source);
    assert_eq!(
        dual.reasons,
        vec![SelectionReason::Lexical, SelectionReason::ExactPath],
        "reasons unioned without duplicates"
    );
    // Tier 1 precedes tier 2, so "dual" comes before the lexical-only anchor.
    assert_eq!(fused[0].id.as_str(), "dual");
}

#[test]
fn rule_28_zero_score_channel_normalizes_to_zero_without_nan() {
    let fused = fuse(&[
        vec![candidate(
            "a",
            Channel::Knowledge,
            0.0,
            vec![SelectionReason::Lexical],
            1,
        )],
        vec![candidate(
            "b",
            Channel::Source,
            0.0,
            vec![SelectionReason::Lexical],
            1,
        )],
    ]);
    assert_eq!(fused.len(), 2);
    for item in &fused {
        assert!(!item.score.is_nan(), "score must not be NaN: {item:?}");
    }
    // Both channels' max score is 0.0, so normalization yields 0.0 for both
    // and the RRF-score tie falls through to id ascending.
    assert_eq!(
        fused
            .iter()
            .map(|candidate| candidate.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
}

#[test]
fn rule_29_fusion_is_deterministic_across_repeated_calls() {
    let lists = vec![
        vec![
            candidate(
                "k1",
                Channel::Knowledge,
                40.0,
                vec![SelectionReason::LinkedFrom],
                1,
            ),
            candidate(
                "k2",
                Channel::Knowledge,
                2.0,
                vec![SelectionReason::Lexical],
                1,
            ),
        ],
        vec![candidate(
            "s1",
            Channel::Source,
            0.5,
            vec![SelectionReason::Lexical],
            1,
        )],
    ];
    assert_eq!(fuse(&lists), fuse(&lists));
}

#[test]
fn rule_30_one_empty_list_among_two_behaves() {
    let fused = fuse(&[
        vec![candidate(
            "only",
            Channel::Knowledge,
            4.0,
            vec![SelectionReason::Lexical],
            1,
        )],
        Vec::new(),
    ]);
    assert_eq!(fused.len(), 1);
    assert_eq!(fused[0].id.as_str(), "only");
    assert!(
        (fused[0].score - 0.016_393).abs() < 1e-4,
        "got {}",
        fused[0].score
    );
}
