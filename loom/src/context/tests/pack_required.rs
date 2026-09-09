//! The `--require-id` reservation contract: explicitly required candidates
//! are packed before anything optional, and one that cannot fit is reported
//! in `unmet_required` rather than silently dropped or substituted.

use super::pack_fixtures::{
    chunk, request_with_item_budget, request_with_raw_budget, required_candidate,
};
use crate::context::pack::pack;
use crate::context::rank::RankedCandidate;
use crate::context::schema::{KnowledgeChunk, RequiredRepresentation, BRIEF_FRAME_TOKENS};

/// Headroom left after the first required item is charged, in
/// [`two_required_ids_cannot_be_satisfied_by_packing_only_one`] — enough that
/// the budget isn't zero, not enough for the second item.
const SLACK: usize = 50;

#[test]
fn a_required_full_item_larger_than_the_budget_is_reported_unmet_not_substituted() {
    let body = "required text\n".repeat(500);
    let packed = pack(
        &request_with_item_budget(20),
        &[required_candidate("required", 2_000)],
        &[chunk("required", &body, 2_000)],
        None,
    );
    assert!(packed.items.is_empty());
    assert_eq!(packed.unmet_required.len(), 1);
    assert_eq!(packed.unmet_required[0].id, "required");
    assert!(packed.unmet_required[0].needed_tokens > packed.unmet_required[0].available_tokens);
}

#[test]
fn two_required_ids_cannot_be_satisfied_by_packing_only_one() {
    let (chunks, ranked) = two_required_items_corpus();

    // Measure exactly what each item costs once rendered, so the budget
    // below is built from real numbers rather than guessed constants.
    let first_cost = required_item_cost(&ranked[..1], &chunks[..1]);
    let second_cost = required_item_cost(&ranked[1..], &chunks[1..]);

    // Enough headroom for the frame and the first item, plus a little slack —
    // not enough for the second, and not zero, so a bug that charges the
    // second against the FULL budget (rather than what the first left
    // behind) cannot pass by coincidence.
    let budget_tokens = BRIEF_FRAME_TOKENS + first_cost + SLACK;
    let packed = pack(
        &request_with_raw_budget(budget_tokens),
        &ranked,
        &chunks,
        None,
    );

    assert_eq!(
        packed
            .items
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>(),
        vec!["first"],
        "the second must not be substituted in place of itself going unmet"
    );
    assert_eq!(packed.unmet_required.len(), 1);
    assert_eq!(packed.unmet_required[0].id, "second");
    assert_eq!(
        packed.unmet_required[0].available_tokens, SLACK,
        "available must reflect the budget left AFTER the first item was charged"
    );
    assert!(packed.unmet_required[0].needed_tokens >= second_cost);
    assert!(
        SLACK < second_cost,
        "the test must leave genuinely insufficient room"
    );
}

/// Two required chunks sized to render to roughly 700 and 500 tokens
/// respectively — see `reserve` (`context/pack/required.rs`), which charges
/// each required candidate against what remains of the budget after the ones
/// before it, not against the full budget every time.
fn two_required_items_corpus() -> ([KnowledgeChunk; 2], [RankedCandidate; 2]) {
    let big_body = "required detail line for the first item\n".repeat(70);
    let small_body = "required detail line for the second item\n".repeat(50);
    let chunks = [
        chunk("first", &big_body, 1),
        chunk("second", &small_body, 1),
    ];
    let ranked = [
        required_candidate("first", 1),
        required_candidate("second", 1),
    ];
    (chunks, ranked)
}

/// Pack `ranked`/`chunks` alone with generous headroom and read back the
/// rendered token cost of the one item that must fit — the real, measured
/// cost rather than a guessed constant.
fn required_item_cost(ranked: &[RankedCandidate], chunks: &[KnowledgeChunk]) -> usize {
    pack(&request_with_item_budget(10_000), ranked, chunks, None).items[0].token_count
}

#[test]
fn a_required_compact_item_is_marked_truncated_when_cut() {
    let body = "long compact line\n".repeat(500);
    let mut pack_request = request_with_item_budget(1_000);
    pack_request.required_representation = RequiredRepresentation::Compact;
    let packed = pack(
        &pack_request,
        &[required_candidate("required", 2_000)],
        &[chunk("required", &body, 2_000)],
        None,
    );

    assert_eq!(packed.items.len(), 1);
    assert!(packed.items[0].truncated);
}
