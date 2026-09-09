//! The tier-1/tier-2 twin rule: a summary whose whole content is a pointer to
//! a tier-2 topic must not spend budget alongside the topic itself.
//!
//! Two halves are tested separately. [`tier1_twin`] decides which pairs are
//! twins at all, and gets the shapes that must NOT collapse — a same-anchor
//! pair under a different stem, a deeper path, indexed prose, a source node.
//! The packer decides what to do about one, and gets both orders (detail
//! first, summary first) and the budget case where the summary is the only
//! thing that fits.

use super::source_fixtures::{full_node, graph_with_node, source_candidate};
use crate::context::pack::twins::tier1_twin;
use crate::context::pack::{pack, PackRequest};
use crate::context::rank::RankedCandidate;
use crate::context::schema::{
    Channel, ChunkId, Freshness, KnowledgeChunk, LifecycleState, RequiredRepresentation,
    SelectionReason, BRIEF_FRAME_TOKENS, BYTES_PER_TOKEN_ESTIMATE,
};
use std::path::PathBuf;

const TIER2: &str = "mistakes/sandbox-and-settings.md#sandbox-contradictory-path-rules#0";
const TIER1: &str = "mistakes.md#sandbox-contradictory-path-rules#0";

/// A chunk whose id is a real two-tier id, with `file`/`anchor` derived from it
/// the way the chunker derives them — the packer reads the pointer off the
/// chunk, so a fixture that faked them would hide a mismatch.
fn chunk(id: &str, tokens: usize) -> KnowledgeChunk {
    let (path, suffix) = id.split_once('#').expect("fixture ids carry an anchor");
    let anchor = suffix.split('#').next().unwrap_or_default();
    KnowledgeChunk {
        id: id.to_string(),
        file: PathBuf::from(path),
        anchor: anchor.to_string(),
        heading: anchor.to_string(),
        body: "x".repeat(tokens.saturating_mul(BYTES_PER_TOKEN_ESTIMATE)),
        content_hash: String::new(),
        estimated_tokens: tokens,
        aliases: Vec::new(),
        category: None,
        source_paths: Vec::new(),
        symbols: Vec::new(),
        links: Vec::new(),
        state: LifecycleState::Active,
    }
}

fn candidate(id: &str, score: f32, token_count: usize) -> RankedCandidate {
    RankedCandidate {
        id: ChunkId::from(id),
        channel: Channel::Knowledge,
        score,
        reasons: vec![SelectionReason::Lexical],
        token_count,
        matched_term_count: 1,
        confidence_ceiling: None,
    }
}

fn request(budget_tokens: usize) -> PackRequest {
    PackRequest {
        query: "query".into(),
        scope: vec![Channel::Knowledge],
        budget_tokens: BRIEF_FRAME_TOKENS + 70 + budget_tokens,
        structural_freshness: Freshness::default(),
        semantic_freshness: Freshness::default(),
        dropped_terms: Vec::new(),
        surviving_terms: vec!["query".to_string()],
        required_representation: RequiredRepresentation::Full,
        degraded: None,
    }
}

/// The ids a pack carried, in order.
fn packed_ids(
    budget_tokens: usize,
    ranked: &[RankedCandidate],
    chunks: &[KnowledgeChunk],
) -> Vec<String> {
    pack(&request(budget_tokens), ranked, chunks, None)
        .items
        .iter()
        .map(|item| item.id.as_str().to_string())
        .collect()
}

#[test]
fn a_tier2_topic_id_maps_to_the_tier1_summary_that_spilled_it() {
    assert_eq!(tier1_twin(TIER2).as_deref(), Some(TIER1));
}

/// A later occurrence of a repeated tier-2 heading still points at occurrence
/// `0` of the tier-1 heading: a tier-1 file states a topic once.
#[test]
fn a_repeated_tier2_occurrence_still_names_the_first_tier1_occurrence() {
    assert_eq!(
        tier1_twin("mistakes/sandbox-and-settings.md#retries#3").as_deref(),
        Some("mistakes.md#retries#0")
    );
}

/// The whole point of keying on the parent directory: two files can share an
/// anchor without one being a summary of the other.
#[test]
fn a_shared_anchor_under_a_different_stem_is_not_a_twin() {
    assert_eq!(
        tier1_twin("architecture/overview.md#overview#0").as_deref(),
        Some("architecture.md#overview#0"),
        "the twin is the file whose stem is the parent directory"
    );
    assert_ne!(
        tier1_twin("architecture/overview.md#overview#0").as_deref(),
        Some("conventions.md#overview#0")
    );
}

#[test]
fn ids_that_name_no_tier1_file_have_no_twin() {
    for id in [
        // Nested deeper than one directory: no tier-1 file is its parent.
        "doc/plans/notes.md#anchor#0",
        // A tier-1 id itself.
        TIER1,
        // Indexed project prose, which has no curated tier-1 counterpart.
        "prose:mistakes/sandbox-and-settings.md#sandbox-contradictory-path-rules#0",
        // A source node: one `#`, and a scope where an occurrence would be.
        "loom/src/context/pack.rs#function:pack",
        // A file's preamble chunk, not a spilled topic.
        "mistakes/sandbox-and-settings.md##0",
        // Not markdown.
        "mistakes/sandbox-and-settings.txt#anchor#0",
    ] {
        assert_eq!(tier1_twin(id), None, "{id} must have no tier-1 twin");
    }
}

#[test]
fn the_summary_is_dropped_when_its_detail_is_packed() {
    let ranked = vec![candidate(TIER2, 2.0, 4), candidate(TIER1, 1.0, 2)];
    let packed = pack(
        &request(100),
        &ranked,
        &[chunk(TIER2, 4), chunk(TIER1, 2)],
        None,
    );

    assert_eq!(
        packed
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec![TIER2]
    );
    assert_eq!(
        packed.omitted.omitted, 1,
        "the summary is reported, not lost"
    );
    assert_eq!(
        packed.estimated_tokens,
        BRIEF_FRAME_TOKENS + packed.items[0].token_count,
        "the summary's tokens are not charged to the budget"
    );
}

/// The fallback. A detail too large for the budget leaves the summary as the
/// only thing that can tell the reader the topic exists.
#[test]
fn the_summary_is_packed_when_its_detail_does_not_fit() {
    let ranked = vec![candidate(TIER2, 2.0, 40), candidate(TIER1, 1.0, 2)];
    let packed = pack(
        &request(5),
        &ranked,
        &[chunk(TIER2, 40), chunk(TIER1, 2)],
        None,
    );

    assert_eq!(
        packed
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec![TIER1]
    );
    assert_eq!(packed.omitted.omitted, 1);
}

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

/// The channel gate. A source node whose id happens to be punctuated like a
/// tier-2 chunk is not anybody's detail: id spaces are disjoint today, and the
/// packer decides what an id means from its channel, never from its shape.
#[test]
fn a_source_node_shaped_like_a_tier2_id_suppresses_nothing() {
    let graph = graph_with_node(full_node(
        TIER2,
        "mistakes/sandbox-and-settings.md",
        &["topic"],
        "fn topic()",
    ));
    let ranked = vec![source_candidate(TIER2, 2.0, 4), candidate(TIER1, 1.0, 2)];
    let mut request = request(100);
    request.scope = vec![Channel::Knowledge, Channel::Source];

    let packed = pack(&request, &ranked, &[chunk(TIER1, 2)], Some(&graph));

    assert_eq!(
        packed
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec![TIER2, TIER1]
    );
}

/// A caller who names an id gets that id. `--require-id` is the only thing
/// that awards `ExplicitId`, and answering it with a different chunk would
/// break the one promise retrieval makes literally.
#[test]
fn a_summary_the_caller_required_survives_its_detail() {
    let mut summary = candidate(TIER1, 1.0, 2);
    summary.reasons.push(SelectionReason::ExplicitId);
    let ranked = vec![candidate(TIER2, 2.0, 4), summary];
    let chunks = [chunk(TIER2, 4), chunk(TIER1, 2)];

    assert_eq!(
        packed_ids(100, &ranked, &chunks),
        vec![TIER1.to_string(), TIER2.to_string()]
    );
}

/// The other half of the same guard: a required summary is boosted far above
/// its detail, so promoting the detail into its slot would pull a weak chunk
/// to the head of the pack. Order stands instead.
#[test]
fn a_required_summary_does_not_promote_its_detail() {
    let mut summary = candidate(TIER1, 1000.0, 2);
    summary.reasons.push(SelectionReason::ExplicitId);
    let ranked = vec![
        summary,
        candidate("conventions.md#unrelated#0", 1.5, 2),
        candidate(TIER2, 0.1, 4),
    ];
    let chunks = [
        chunk(TIER1, 2),
        chunk("conventions.md#unrelated#0", 2),
        chunk(TIER2, 4),
    ];

    assert_eq!(
        packed_ids(100, &ranked, &chunks),
        vec![
            TIER1.to_string(),
            "conventions.md#unrelated#0".to_string(),
            TIER2.to_string()
        ],
        "an explicitly required summary leaves the fused order alone"
    );
}

/// Indexed prose is not curated knowledge and has no tier-1 summary, so a
/// prose id shaped like a tier-2 topic must suppress nothing.
#[test]
fn a_prose_id_suppresses_no_tier1_summary() {
    let prose = "prose:mistakes/sandbox-and-settings.md#sandbox-contradictory-path-rules#0";
    let ranked = vec![candidate(prose, 2.0, 4), candidate(TIER1, 1.0, 2)];
    let chunks = [chunk(prose, 4), chunk(TIER1, 2)];

    assert_eq!(
        packed_ids(100, &ranked, &chunks),
        vec![prose.to_string(), TIER1.to_string()]
    );
}
