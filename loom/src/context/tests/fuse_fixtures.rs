use crate::context::rank::*;
use crate::context::schema::*;

pub(super) fn candidate(
    id: &str,
    channel: Channel,
    score: f32,
    reasons: Vec<SelectionReason>,
    token_count: usize,
) -> RankedCandidate {
    RankedCandidate {
        id: ChunkId::from(id),
        channel,
        score,
        reasons,
        token_count,
        matched_term_count: 0,
        // Fusion must carry a ceiling through untouched, but none of these
        // fixtures exercise the demotion, so they claim no cap.
        confidence_ceiling: None,
    }
}

pub(super) fn find<'a>(fused: &'a [RankedCandidate], id: &str) -> &'a RankedCandidate {
    fused
        .iter()
        .find(|candidate| candidate.id.as_str() == id)
        .unwrap_or_else(|| panic!("no candidate with id {id:?} in {fused:?}"))
}

/// Fixture for `rule_25`: two channels, each with an exact-rung anchor ahead
/// of a lexical-only item. Once the anchors are pulled into tier 1 and
/// filtered out of tier 2's RRF numbering, both lexical items become their
/// channel's rank-1 survivor, so they tie at an identical RRF contribution of
/// `1 / (RRF_K + 1)` (~0.0163934) -- but their channels have different
/// overall maxima (1000.0 vs 50.0), so the within-channel normalized scores
/// that break the tie diverge: `5.0 / 1000.0 = 0.005` vs `5.0 / 50.0 = 0.1`.
pub(super) fn tied_tier2_fixture() -> Vec<Vec<RankedCandidate>> {
    vec![
        vec![
            candidate(
                "a-anchor",
                Channel::Knowledge,
                1000.0,
                vec![SelectionReason::ExplicitId],
                1,
            ),
            candidate(
                "a-lex",
                Channel::Knowledge,
                5.0,
                vec![SelectionReason::Lexical],
                1,
            ),
        ],
        vec![
            candidate(
                "b-anchor",
                Channel::Source,
                50.0,
                vec![SelectionReason::ExactSymbol],
                1,
            ),
            candidate(
                "b-lex",
                Channel::Source,
                5.0,
                vec![SelectionReason::Lexical],
                1,
            ),
        ],
    ]
}
