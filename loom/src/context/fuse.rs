//! Two-tier fusion of independently ranked channel lists.
//!
//! Plain reciprocal-rank fusion (RRF) reduces every candidate to its rank
//! *position*, so a knowledge chunk that scored 1080 on an explicit-id hit
//! and a source node that scored 0.3 on a weak lexical match both land at
//! rank 1 and tie exactly -- a tie that used to fall through to
//! `a.id.cmp(&b.id)`, i.e. alphabetical order. Two tiers fix that:
//!
//! - **Tier 1** holds every candidate (from either channel) whose merged
//!   [`SelectionReason`]s include at least one exact rung -- `ExplicitId`,
//!   `ExactPath`, `ExactSymbol`, `LinkedFrom`, or `StageDependency`. A
//!   candidate lexical-only in one channel but exact-rung in the other is
//!   still tier 1: classification runs on the *merged* reason set, so an id
//!   is excluded from every channel's tier-2 list, not just the one where it
//!   earned the rung. Tier 1 is ordered by raw channel score descending
//!   (the merged score is the maximum raw score seen across channels, not
//!   the sum -- summing would reward an id merely for appearing twice), id
//!   ascending on ties, and it precedes ALL of tier 2 in the output.
//! - **Tier 2** holds the remaining candidates. Each channel's survivors
//!   (tier-1 ids removed) keep their relative order and are renumbered from
//!   1, so RRF (`RRF_K = 60`, unchanged) runs over the *tier-2-only* ranked
//!   lists rather than the original ones -- a channel's best lexical match
//!   becomes tier 2's rank 1 for that channel even if an exact-rung hit
//!   outranked it in the original list. Ties in the resulting RRF score are
//!   broken by *within-channel normalized score* (`raw_score /
//!   max_raw_score_in_that_channel`, `0.0` when that channel's max is `0.0`
//!   or non-finite, computed over the channel's *original*, unfiltered
//!   candidates) descending, then id ascending. This is what separates two
//!   candidates that both rank 1 within their own channel's tier-2 list:
//!   RRF alone cannot, because identical rank positions produce identical
//!   contributions regardless of channel.
//!
//! Tier-1 raw scores are comparable *across* channels because both rankers
//! score the exact-match ladder from the same boost constants:
//! `rank_source.rs` imports `BOOST_EXACT_PATH`/`BOOST_EXACT_SYMBOL`/
//! `BOOST_EXPLICIT_ID` straight from [`crate::context::rank`]
//! (`rank_source.rs:24-27`), so a `100.0` exact-path hit means the same thing
//! whichever channel produced it. That shared scale is the assumption the
//! whole tier-1 ordering rests on.
//!
//! ## Invariant: `score` is not comparable across tiers
//!
//! A tier-1 item's output `score` is its raw exact-match-ladder score; a
//! tier-2 item's is its RRF score. These live on unrelated scales (tier 1:
//! tens to 1000+; tier 2: roughly `1 / RRF_K` and smaller) and are **not**
//! numerically comparable against each other -- the tier boundary carries the
//! ordering, `score` does not sort the whole output by magnitude. Downstream
//! code that reads `score` off the fused list --
//! `pack::build_omission_summary`'s `weakest_included_score` and
//! `commands/hook/user_prompt.rs::without_weakest` -- happens to behave
//! correctly today because the weakest-scoring item is always a tier-2 item,
//! but that is a consequence of the scale gap, not something either function
//! checks. A future reader must not assume `score` is a global ranking key.
//!
//! ## Cross-channel id collisions
//!
//! Both merge passes below key by [`ChunkId`] across every input list,
//! exactly as fusion did before the two-tier rework -- see the hazard note at
//! the top of [`mod@crate::context::rank_source`] (`rank_source.rs:10-20`):
//! knowledge and source ids live in disjoint path spaces today, so a
//! collision cannot occur, but if one ever did, a merged entry would adopt
//! one channel and [`crate::context::pack`]'s dispatch on `candidate.channel`
//! would silently consult the wrong map. That hazard is unchanged by tiering.

use crate::context::rank::RankedCandidate;
use crate::context::schema::{Channel, ChunkId, SelectionReason};
use std::collections::{BTreeMap, BTreeSet};

/// Rank offset used by reciprocal-rank fusion.
pub const RRF_K: f32 = 60.0;

/// True when `reasons` includes one of the explicitly enumerated exact-match
/// rungs that qualify a candidate for tier 1.
fn has_exact_rung(reasons: &[SelectionReason]) -> bool {
    reasons.iter().any(|reason| {
        matches!(
            reason,
            SelectionReason::ExplicitId
                | SelectionReason::ExactPath
                | SelectionReason::ExactSymbol
                | SelectionReason::LinkedFrom
                | SelectionReason::StageDependency
        )
    })
}

/// Every id that carries an exact rung in ANY input list.
///
/// Computed once, before tier 2's per-channel lists are filtered, so an id
/// that is exact-rung in one channel and lexical-only in another is removed
/// from every channel's tier-2 numbering -- not just the channel where it
/// earned the rung.
fn tier1_ids(lists: &[Vec<RankedCandidate>]) -> BTreeSet<ChunkId> {
    lists
        .iter()
        .flatten()
        .filter(|candidate| has_exact_rung(&candidate.reasons))
        .map(|candidate| candidate.id.clone())
        .collect()
}

/// Each channel's maximum raw score across every input candidate (both
/// tiers), computed once up front -- tier 2's normalization divisor. A
/// channel with no candidates yields `f32::NEG_INFINITY`, which
/// [`normalized_score`] treats as non-finite and maps to `0.0`.
fn channel_maxima(lists: &[Vec<RankedCandidate>]) -> Vec<(Channel, f32)> {
    Channel::all()
        .iter()
        .map(|&channel| {
            let max = lists
                .iter()
                .flatten()
                .filter(|candidate| candidate.channel == channel)
                .map(|candidate| candidate.score)
                .fold(f32::NEG_INFINITY, f32::max);
            (channel, max)
        })
        .collect()
}

/// Look up one channel's precomputed maximum from [`channel_maxima`].
fn max_for(maxima: &[(Channel, f32)], channel: Channel) -> f32 {
    maxima
        .iter()
        .find(|(candidate_channel, _)| *candidate_channel == channel)
        .map(|(_, max)| *max)
        .unwrap_or(f32::NEG_INFINITY)
}

/// `raw_score / channel_max`, or `0.0` when `channel_max` is `0.0` or
/// non-finite -- guards both division-by-zero and a channel with no
/// candidates (see [`channel_maxima`]).
fn normalized_score(raw_score: f32, channel_max: f32) -> f32 {
    if !channel_max.is_finite() || channel_max == 0.0 {
        0.0
    } else {
        raw_score / channel_max
    }
}

/// One tier-1 id's merged state: metadata from the channel that produced its
/// highest raw score, reasons unioned across every channel it appeared in.
struct Tier1Accumulator {
    candidate: RankedCandidate,
    raw_score_max: f32,
}

/// Merge every tier-1 occurrence by id, across channels.
///
/// A tier-1 id can have a lexical-only occurrence in one channel and an
/// exact-rung occurrence in another (see the module doc); both are folded in
/// here so the reason union and the raw-score maximum see every occurrence,
/// not just the one that earned the rung.
fn merge_tier1(
    lists: &[Vec<RankedCandidate>],
    tier1_ids: &BTreeSet<ChunkId>,
) -> BTreeMap<ChunkId, Tier1Accumulator> {
    let mut merged: BTreeMap<ChunkId, Tier1Accumulator> = BTreeMap::new();
    for candidate in lists.iter().flatten() {
        if !tier1_ids.contains(&candidate.id) {
            continue;
        }
        match merged.get_mut(&candidate.id) {
            Some(entry) => {
                let mut reasons = entry.candidate.reasons.clone();
                for reason in &candidate.reasons {
                    if !reasons.contains(reason) {
                        reasons.push(*reason);
                    }
                }
                if candidate.score > entry.raw_score_max {
                    entry.raw_score_max = candidate.score;
                    entry.candidate = candidate.clone();
                }
                entry.candidate.reasons = reasons;
            }
            None => {
                merged.insert(
                    candidate.id.clone(),
                    Tier1Accumulator {
                        candidate: candidate.clone(),
                        raw_score_max: candidate.score,
                    },
                );
            }
        }
    }
    merged
}

/// Finish tier 1: raw score descending, id ascending on ties.
fn finish_tier1(merged: BTreeMap<ChunkId, Tier1Accumulator>) -> Vec<RankedCandidate> {
    let mut tier1: Vec<RankedCandidate> = merged
        .into_values()
        .map(|entry| RankedCandidate {
            score: entry.raw_score_max,
            ..entry.candidate
        })
        .collect();
    tier1.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
    tier1
}

/// One tier-2 id's merged state across every channel it survived in.
struct Tier2Accumulator {
    candidate: RankedCandidate,
    best_rank: usize,
    rrf_score: f32,
    raw_score_max: f32,
}

/// Merge every tier-2 occurrence by id, running RRF over each channel's
/// *survivor* list -- tier-1 ids filtered out, remaining candidates
/// renumbered from 1 in their existing relative order.
fn merge_tier2(
    lists: &[Vec<RankedCandidate>],
    tier1_ids: &BTreeSet<ChunkId>,
) -> BTreeMap<ChunkId, Tier2Accumulator> {
    let mut fused: BTreeMap<ChunkId, Tier2Accumulator> = BTreeMap::new();
    for list in lists {
        let survivors = list
            .iter()
            .filter(|candidate| !tier1_ids.contains(&candidate.id));
        for (position, candidate) in survivors.enumerate() {
            let rank = position + 1;
            let contribution = 1.0 / (RRF_K + rank as f32);
            match fused.get_mut(&candidate.id) {
                Some(entry) => {
                    entry.rrf_score += contribution;
                    entry.raw_score_max = entry.raw_score_max.max(candidate.score);
                    for reason in &candidate.reasons {
                        if !entry.candidate.reasons.contains(reason) {
                            entry.candidate.reasons.push(*reason);
                        }
                    }
                    if rank < entry.best_rank {
                        entry.best_rank = rank;
                        entry.candidate.channel = candidate.channel;
                    }
                }
                None => {
                    fused.insert(
                        candidate.id.clone(),
                        Tier2Accumulator {
                            candidate: candidate.clone(),
                            best_rank: rank,
                            rrf_score: contribution,
                            raw_score_max: candidate.score,
                        },
                    );
                }
            }
        }
    }
    fused
}

/// Finish tier 2: RRF score descending, then within-channel normalized score
/// descending, then id ascending.
fn finish_tier2(
    merged: BTreeMap<ChunkId, Tier2Accumulator>,
    maxima: &[(Channel, f32)],
) -> Vec<RankedCandidate> {
    let mut tier2: Vec<(RankedCandidate, f32)> = merged
        .into_values()
        .map(|entry| {
            let channel_max = max_for(maxima, entry.candidate.channel);
            let normalized = normalized_score(entry.raw_score_max, channel_max);
            let candidate = RankedCandidate {
                score: entry.rrf_score,
                ..entry.candidate
            };
            (candidate, normalized)
        })
        .collect();
    tier2.sort_by(|(a, a_norm), (b, b_norm)| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| b_norm.total_cmp(a_norm))
            .then_with(|| a.id.cmp(&b.id))
    });
    tier2.into_iter().map(|(candidate, _)| candidate).collect()
}

/// Two-tier fusion over per-channel ranked lists. See the module doc for the
/// tier contract and the score-comparability invariant.
pub fn fuse(lists: &[Vec<RankedCandidate>]) -> Vec<RankedCandidate> {
    let maxima = channel_maxima(lists);
    let tier1_ids = tier1_ids(lists);
    let mut output = finish_tier1(merge_tier1(lists, &tier1_ids));
    output.extend(finish_tier2(merge_tier2(lists, &tier1_ids), &maxima));
    output
}
