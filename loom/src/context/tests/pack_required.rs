//! The `--require-id` reservation contract: explicitly required candidates
//! are packed before anything optional, and one that cannot fit is reported
//! in `unmet_required` rather than silently dropped or substituted.

use super::pack_fixtures::{
    chunk, request_with_item_budget, request_with_raw_budget, required_candidate,
};
use super::source_fixtures::{full_node, graph};
use crate::context::graph_store::ResolvedGraph;
use crate::context::pack::pack;
use crate::context::rank::RankedCandidate;
use crate::context::render::rendered_chrome_tokens;
use crate::context::schema::{
    Channel, ChunkId, KnowledgeChunk, RequiredRepresentation, SelectionReason, SourceNode,
    BRIEF_FRAME_TOKENS,
};

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

    // Chrome-aware "available" is what's left for the second item's own
    // body once the chrome its inclusion would add — here, just the
    // `### Knowledge` heading the first item already required, shared by any
    // run of same-channel knowledge items — is paid for. Measured off the
    // pack's own first item via the real helper rather than hardcoded, so it
    // cannot drift from what `render` actually charges.
    let heading_tokens = rendered_chrome_tokens(std::iter::once(&packed.items[0]), &[]);
    assert_eq!(
        packed.unmet_required[0].available_tokens,
        SLACK - heading_tokens,
        "available must reflect the budget left AFTER the first item and its chrome were charged"
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

/// `reserve` must charge the per-path group prefix `render_source_group`
/// emits for each of several required source items at DISTINCT paths, not
/// just their bodies. `gamma`'s span is deliberately huge — a legitimate
/// value for a node deep in a large generated file — so its own admitted row
/// costs far more than reporting it `unmet` would (an unmet line never
/// quotes a node's span, only its id and a couple of small numbers): the
/// budget below is chosen so a `reserve` blind to header/prefix chrome would
/// still admit `gamma` on its own (inflated) body cost alone, overshooting
/// once the three group prefixes render, while a chrome-aware `reserve` sees
/// that overshoot coming and reports `gamma` unmet instead, comfortably
/// within budget since the tokens that saves dwarf the group prefix it
/// still owes for `alpha` and `beta`.
#[test]
fn several_required_source_items_at_distinct_paths_do_not_overshoot_the_finalized_budget() {
    let (source_graph, ranked) = three_required_source_items_corpus();

    // What each of `alpha`, `beta` and `gamma` costs alone once admitted,
    // chrome included — the same "measure it, don't hand-compute it" pattern
    // `required_item_cost` uses above. Room for all three (inflated) bodies
    // plus a few tokens of slack is not room for the header and three group
    // prefixes their three distinct paths add: a `reserve` that charges only
    // bodies sees exactly this much room and admits all three; the
    // finalized, chrome-charged pack does not fit it.
    let alpha_cost = required_source_item_cost(&ranked[..1], &source_graph);
    let beta_cost = required_source_item_cost(&ranked[1..2], &source_graph);
    let gamma_cost = required_source_item_cost(&ranked[2..], &source_graph);
    let budget = BRIEF_FRAME_TOKENS + alpha_cost + beta_cost + gamma_cost + 5;

    let packed = pack(
        &request_with_raw_budget(budget),
        &ranked,
        &[],
        Some(&source_graph),
    );

    assert!(
        packed.within_budget(),
        "pack claims {} tokens against its own budget of {}",
        packed.estimated_tokens,
        packed.budget_tokens
    );
}

/// The rendered cost of admitting one required source candidate alone, with
/// generous headroom — chrome included, the same "measure it, don't
/// hand-compute it" pattern `required_item_cost` uses for knowledge chunks.
fn required_source_item_cost(ranked: &[RankedCandidate], source_graph: &ResolvedGraph) -> usize {
    pack(
        &request_with_raw_budget(1_000_000),
        ranked,
        &[],
        Some(source_graph),
    )
    .items[0]
        .token_count
}

/// `gamma`'s node: a legitimate shape for a function deep in a large
/// generated file, its span deliberately huge so its admitted cost dwarfs
/// what reporting it `unmet` would cost — see
/// [`several_required_source_items_at_distinct_paths_do_not_overshoot_the_finalized_budget`].
fn huge_span_node(id: &str, path: &str, name: &str) -> SourceNode {
    let mut node = full_node(id, path, &[name], &format!("fn {name}()"));
    node.span.line_start = 123_456_789_012_345;
    node.span.line_end = 987_654_321_098_765;
    node
}

/// `gamma`'s candidate: every reason it could carry, firing at once — a rare
/// combination, but a legitimate one, and one an admitted row must render in
/// full while an `unmet` line never mentions reasons at all. Along with
/// `huge_span_node`, this is what makes admitting `gamma` cost more than
/// reporting it unmet.
fn candidate_with_every_reason(id: &str) -> RankedCandidate {
    let mut candidate = required_source_candidate(id);
    candidate.reasons = vec![
        SelectionReason::ExplicitId,
        SelectionReason::ExactPath,
        SelectionReason::ExactSymbol,
        SelectionReason::LinkedFrom,
        SelectionReason::StageDependency,
        SelectionReason::GraphNeighbor,
        SelectionReason::Lexical,
    ];
    candidate
}

/// Three required source nodes at three distinct paths. `alpha` and `beta`
/// are ordinary; `gamma` carries a huge span so its admitted cost dwarfs
/// what reporting it `unmet` would cost — see
/// [`several_required_source_items_at_distinct_paths_do_not_overshoot_the_finalized_budget`].
fn three_required_source_items_corpus() -> (ResolvedGraph, [RankedCandidate; 3]) {
    let gamma = huge_span_node("src/gamma.rs#function:gamma", "src/gamma.rs", "gamma");

    let source_graph = graph(vec![
        (
            "src/alpha.rs",
            vec![full_node(
                "src/alpha.rs#function:alpha",
                "src/alpha.rs",
                &["alpha"],
                "fn alpha()",
            )],
        ),
        (
            "src/beta.rs",
            vec![full_node(
                "src/beta.rs#function:beta",
                "src/beta.rs",
                &["beta"],
                "fn beta()",
            )],
        ),
        ("src/gamma.rs", vec![gamma]),
    ]);

    let ranked = [
        required_source_candidate("src/alpha.rs#function:alpha"),
        required_source_candidate("src/beta.rs#function:beta"),
        candidate_with_every_reason("src/gamma.rs#function:gamma"),
    ];
    (source_graph, ranked)
}

fn required_source_candidate(id: &str) -> RankedCandidate {
    RankedCandidate {
        id: ChunkId::from(id),
        channel: Channel::Source,
        score: 1.0,
        reasons: vec![SelectionReason::ExplicitId],
        token_count: 1,
        matched_term_count: 0,
        confidence_ceiling: None,
    }
}
