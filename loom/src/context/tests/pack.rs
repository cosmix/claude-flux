//! General packing behavior for [`crate::context::pack::pack`]: candidate
//! lookup, budget fitting, omission accounting, excerpt windowing, and the
//! budget invariant itself. The `--require-id` reservation contract lives in
//! `pack_required.rs`; shared fixtures live in `pack_fixtures.rs`.

use super::pack_fixtures::{candidate, chunk, request_with_item_budget, request_with_raw_budget};
use crate::context::pack::*;
use crate::context::rank::*;
use crate::context::render::{rendered_chrome_tokens, rendered_item_tokens};
use crate::context::schema::*;
use std::path::PathBuf;

#[test]
fn rule_25_packer_looks_up_chunks_by_their_string_ids() {
    let chunks = vec![chunk("b", "body", 1), chunk("a", "body", 1)];
    let packed = pack(
        &request_with_item_budget(100),
        &[candidate("a", Channel::Knowledge, 1.0, 1)],
        &chunks,
        None,
    );
    assert_eq!(packed.items[0].pointer.path, PathBuf::from("a.md"));
}

#[test]
fn rule_26_missing_candidates_are_skipped_and_omitted() {
    let packed = pack(
        &request_with_item_budget(100),
        &[candidate("missing", Channel::Knowledge, 1.0, 4)],
        &[],
        None,
    );
    assert!(packed.items.is_empty());
    assert_eq!(packed.omitted.omitted, 1);
}

#[test]
fn rule_27_nonfitting_chunks_are_skipped_while_later_ones_can_fit() {
    let large = "large body ".repeat(100);
    let chunks = vec![chunk("large", &large, 300), chunk("small", "body", 3)];
    let packed = pack(
        &request_with_item_budget(40),
        &[
            candidate("large", Channel::Knowledge, 2.0, 8),
            candidate("small", Channel::Knowledge, 1.0, 3),
        ],
        &chunks,
        None,
    );
    assert_eq!(packed.items[0].id.as_str(), "small");
    assert_eq!(packed.omitted.omitted, 1);
}

#[test]
fn rule_29_knowledge_candidates_still_become_knowledge_chunk_items() {
    let mut source = chunk("a", &"é".repeat(121), 4);
    source.heading.clear();
    source.anchor = "where".into();
    source.file = PathBuf::from("doc/a.md");
    source.state = LifecycleState::Draft;
    let ranked = RankedCandidate {
        id: ChunkId::from("a"),
        channel: Channel::Knowledge,
        score: 2.5,
        reasons: vec![SelectionReason::ExactPath],
        token_count: 4,
        matched_term_count: 0,
        confidence_ceiling: None,
    };
    let packed = pack(&request_with_item_budget(200), &[ranked], &[source], None);
    let item = &packed.items[0];
    assert_eq!(item.id.as_str(), "a");
    assert_eq!(item.kind, ItemKind::KnowledgeChunk);
    assert_eq!(item.pointer.path, PathBuf::from("doc/a.md"));
    assert_eq!(item.pointer.anchor, "where");
    assert_eq!(item.pointer.line_start, None);
    assert_eq!(item.pointer.line_end, None);
    assert_eq!(item.summary.chars().count(), 120);
    assert_eq!(item.source, Channel::Knowledge);
    assert_eq!(item.token_count, rendered_item_tokens(item));
    assert!((item.score - 2.5).abs() < 1e-4, "got {}", item.score);
    assert_eq!(item.reasons, vec![SelectionReason::ExactPath]);
    assert_eq!(item.confidence, Confidence::High);
    assert_eq!(item.state, LifecycleState::Draft);
}

#[test]
fn rule_31_every_unincluded_ranked_candidate_is_counted() {
    let large_body = "too large ".repeat(100);
    let packed = pack(
        &request_with_item_budget(40),
        &[
            candidate("missing", Channel::Knowledge, 3.0, 1),
            candidate("large", Channel::Knowledge, 2.0, 2),
            candidate("fits", Channel::Knowledge, 1.0, 1),
        ],
        &[chunk("large", &large_body, 250), chunk("fits", "body", 1)],
        None,
    );
    assert_eq!(packed.items.len(), 1);
    assert_eq!(packed.omitted.omitted, 2);
}

#[test]
fn rule_32_weakest_included_score_is_the_minimum_or_zero() {
    let packed = pack(
        &request_with_item_budget(100),
        &[
            candidate("a", Channel::Knowledge, 0.9, 2),
            candidate("b", Channel::Knowledge, 0.4, 2),
        ],
        &[chunk("a", "body", 2), chunk("b", "body", 2)],
        None,
    );
    assert!((packed.omitted.weakest_included_score - 0.4).abs() < 1e-4);
    let empty = pack(&request_with_item_budget(0), &[], &[], None);
    assert_eq!(empty.omitted.weakest_included_score, 0.0);
}

#[test]
fn rule_33_coverage_reports_all_and_included_candidate_tokens() {
    let large_body = "too large ".repeat(100);
    let packed = pack(
        &request_with_item_budget(100),
        &[
            candidate("a", Channel::Knowledge, 3.0, 5),
            candidate("b", Channel::Knowledge, 2.0, 7),
            candidate("c", Channel::Knowledge, 1.0, 3),
        ],
        &[
            chunk("a", "body", 5),
            chunk("b", &large_body, 7),
            chunk("c", "body", 3),
        ],
        None,
    );
    assert_eq!(packed.omitted.coverage.candidates, 3);
    assert_eq!(packed.omitted.coverage.included, 2);
    assert_eq!(packed.omitted.coverage.candidate_tokens, 15);
    assert_eq!(
        packed.omitted.coverage.included_tokens,
        packed
            .items
            .iter()
            .map(|item| item.token_count)
            .sum::<usize>()
    );
}

fn item_for_body(body: &str) -> ContextItem {
    let mut source = chunk("a", body, 1);
    source.content_hash = "sha256:deadbeef".to_string();
    let packed = pack(
        &request_with_item_budget(1_000),
        &[candidate("a", Channel::Knowledge, 1.0, 1)],
        &[source],
        None,
    );
    packed.items.into_iter().next().expect("chunk should fit")
}

fn quoted_prefix(excerpt: &str) -> &str {
    excerpt
        .strip_suffix(EXCERPT_TRUNCATION_MARKER)
        .expect("a truncated excerpt must announce itself with the marker")
        .strip_suffix('\n')
        .expect("the truncation marker must sit on its own line")
}

#[test]
fn packed_items_carry_the_backing_chunks_content_hash() {
    assert_eq!(item_for_body("body").content_hash, "sha256:deadbeef");
}

#[test]
fn a_body_within_the_excerpt_bound_is_quoted_unchanged() {
    let body = "## Heading\n\nA short section body.\n";
    let item = item_for_body(body);
    assert_eq!(item.excerpt.as_deref(), Some(body));
}

#[test]
fn a_body_over_the_excerpt_bound_is_cut_at_a_line_and_marked() {
    let line = "a line of prose that says something\n";
    let body = line.repeat(200);
    assert!(estimate_tokens(&body) > EXCERPT_MAX_TOKENS);

    let excerpt = item_for_body(&body).excerpt.expect("excerpt");
    assert!(excerpt.len() < body.len());
    let quoted = quoted_prefix(&excerpt);
    assert!(body.starts_with(quoted), "the excerpt must quote verbatim");
    assert!(quoted.ends_with("prose that says something"));
}

#[test]
fn an_excerpt_cut_landing_inside_a_multi_byte_character_does_not_panic() {
    let body = "→".repeat(700);
    let excerpt = item_for_body(&body).excerpt.expect("excerpt");
    let quoted = quoted_prefix(&excerpt);
    assert!(body.starts_with(quoted));
    assert_eq!(quoted.chars().count(), 533, "cut to the last whole '→'");

    let body = format!("a{}", "é".repeat(900));
    let excerpt = item_for_body(&body).excerpt.expect("excerpt");
    let quoted = quoted_prefix(&excerpt);
    assert!(body.starts_with(quoted));
    assert_eq!(quoted.chars().count(), 800, "'a' plus 799 whole 'é'");
}

#[test]
fn an_item_is_charged_its_rendered_cost_not_its_body_estimate() {
    let ranked = [candidate("small", Channel::Knowledge, 1.0, 10_000)];
    let packed = pack(
        &request_with_item_budget(100),
        &ranked,
        &[chunk("small", "body", 10_000)],
        None,
    );
    let item = &packed.items[0];

    assert_eq!(item.token_count, rendered_item_tokens(item));
    assert_ne!(item.token_count, ranked[0].token_count);
}

#[test]
fn an_excerpt_is_centred_on_the_matching_line_when_it_sits_past_the_window() {
    let body = format!(
        "## Heading\n{}needle appears near the end\n{}",
        "ordinary filler line\n".repeat(600),
        "tail line\n".repeat(20)
    );
    let mut pack_request = request_with_item_budget(1_000);
    pack_request.surviving_terms = vec!["needle".to_string()];
    let packed = pack(
        &pack_request,
        &[candidate("centred", Channel::Knowledge, 1.0, 3_000)],
        &[chunk("centred", &body, 3_000)],
        None,
    );
    let excerpt = packed.items[0].excerpt.as_deref().expect("excerpt");

    assert!(excerpt.starts_with("[… earlier lines omitted]"));
    assert!(excerpt.contains("needle appears near the end"));
}

#[test]
fn the_pack_estimate_includes_the_frame() {
    let packed = pack(&request_with_item_budget(0), &[], &[], None);
    assert_eq!(packed.estimated_tokens, BRIEF_FRAME_TOKENS);
    assert!(packed.within_budget());
}

#[test]
fn property_pack_never_exceeds_budget() {
    for iteration in 0..200_u32 {
        assert_budget_invariant_holds(iteration.wrapping_add(1));
    }
}

/// One fuzz iteration of the budget invariant: build a random corpus from
/// `seed`, pack it, and assert every property that must hold no matter what
/// the corpus turned out to be.
fn assert_budget_invariant_holds(mut seed: u32) {
    let (budget, chunks, ranked) = random_pack_corpus(&mut seed);
    // Unpadded: `budget` ranges over 0..=1000 and is used exactly as the real
    // budget, including every value at or below `BRIEF_FRAME_TOKENS` that the
    // padded helper could never generate.
    let packed = pack(&request_with_raw_budget(budget), &ranked, &chunks, None);

    assert_estimated_tokens_match_rendering(&packed);
    assert_every_item_renders_its_token_count(&packed);
    assert_coverage_partitions_all_candidates(&packed);
    assert_pack_never_overshoots_budget(&packed, budget);
}

/// `estimated_tokens` must equal what actually renders: the frame plus every
/// admitted item's own count plus whatever chrome the unmet-required lines
/// cost. Split out of `assert_budget_invariant_holds` (pack.rs:263).
fn assert_estimated_tokens_match_rendering(packed: &ContextPack) {
    assert_eq!(
        packed.estimated_tokens,
        BRIEF_FRAME_TOKENS
            + packed
                .items
                .iter()
                .map(|item| item.token_count)
                .sum::<usize>()
            + rendered_chrome_tokens(packed.items.iter(), &packed.unmet_required)
    );
}

/// Every admitted item's stored `token_count` must match what it actually
/// renders to. Split out of `assert_budget_invariant_holds` (pack.rs:263).
fn assert_every_item_renders_its_token_count(packed: &ContextPack) {
    assert!(packed
        .items
        .iter()
        .all(|item| item.token_count == rendered_item_tokens(item)));
}

/// Every ranked candidate is accounted for exactly once: included or
/// omitted, never both, never neither. Split out of
/// `assert_budget_invariant_holds` (pack.rs:263).
fn assert_coverage_partitions_all_candidates(packed: &ContextPack) {
    assert_eq!(
        packed.omitted.coverage.included + packed.omitted.omitted,
        packed.omitted.coverage.candidates
    );
}

/// The core budget invariant: the pack never reports itself within budget
/// while actually overshooting it, in either of the two regimes a budget can
/// fall into relative to the frame's own cost. Split out of
/// `assert_budget_invariant_holds` (pack.rs:263).
fn assert_pack_never_overshoots_budget(packed: &ContextPack, budget: usize) {
    if budget >= BRIEF_FRAME_TOKENS {
        // The frame alone fits, and every item taken was checked against what
        // remained of the budget once the unmet lines reporting the required
        // ids it could not fit were paid for, so the pack can never overshoot.
        //
        // The one exception is a budget those lines alone cannot fit: they
        // report required ids the caller named and the brief must carry them
        // whatever they cost, so the pack reports itself over budget rather
        // than dropping them — with nothing admitted alongside them, since
        // `reserve_within_budget` reaches that state only by holding the whole
        // budget back.
        let unmet_floor =
            BRIEF_FRAME_TOKENS + rendered_chrome_tokens(std::iter::empty(), &packed.unmet_required);
        if unmet_floor <= budget {
            assert!(packed.within_budget());
        } else {
            assert!(packed.items.is_empty(), "{:?}", packed.items);
            assert_eq!(packed.estimated_tokens, unmet_floor);
        }
    } else {
        // Below the frame's own cost, the packer cannot lie its way into
        // `within_budget()`: it charges the frame regardless, so this pack is
        // honestly reported as over budget rather than silently treated as
        // fitting — and with no room left even for the frame, no item can
        // fit either.
        assert!(!packed.within_budget());
        assert!(packed.items.is_empty());
    }
}

/// A pseudo-random `(budget, chunks, candidates)` corpus derived from `seed`,
/// advancing it in place with the same LCG the property test has always used.
///
/// Roughly one candidate in four is explicitly REQUIRED. Without them the fuzz
/// never reaches `pack::required::reserve`, whose unmet lines are the only
/// chrome a pack can grow after an item was already admitted — the one way the
/// budget invariant above can be broken — so a corpus of purely optional
/// candidates leaves exactly the code this property is here to police
/// untested.
fn random_pack_corpus(seed: &mut u32) -> (usize, Vec<KnowledgeChunk>, Vec<RankedCandidate>) {
    fn next(seed: &mut u32) -> u32 {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *seed
    }

    let count = (next(seed) % 40) as usize;
    let budget = (next(seed) % 1_001) as usize;
    let mut chunks = Vec::new();
    let mut ranked = Vec::new();
    for index in 0..count {
        let tokens = (next(seed) % 501) as usize;
        let id = format!("chunk-{index}");
        let body = "x".repeat(tokens.saturating_mul(BYTES_PER_TOKEN_ESTIMATE));
        chunks.push(chunk(&id, &body, tokens));
        let mut fuzzed = candidate(&id, Channel::Knowledge, next(seed) as f32, tokens);
        if next(seed).is_multiple_of(4) {
            fuzzed.reasons = vec![SelectionReason::ExplicitId];
        }
        ranked.push(fuzzed);
    }
    (budget, chunks, ranked)
}
