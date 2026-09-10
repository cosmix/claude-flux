//! Tests for [`super::format_knowledge_brief`] and its rendering helpers.

use super::*;
use crate::context::render::rendered_item_tokens;
use crate::context::schema::{
    estimate_tokens, Channel, ChunkId, Confidence, Coverage, ItemKind, LifecycleState,
    OmissionSummary, SelectionReason, SourcePointer, UnmetRequirement, BRIEF_FRAME_TOKENS,
};
use crate::orchestrator::signals::retrieval::STAGE_QUERY_INPUTS;
use std::path::PathBuf;

/// Charge an item what its own rendering costs, the way the packer does
/// (`context::pack::finalize_item`). A fixture carrying a constant instead
/// would let these budget assertions pass against renderings the real packer
/// could never have priced.
fn with_rendered_cost(mut item: ContextItem) -> ContextItem {
    item.token_count = rendered_item_tokens(&item);
    item
}

/// A knowledge-chunk item at a fixed anchor, optionally carrying an excerpt.
fn item(id: &str, excerpt: Option<&str>) -> ContextItem {
    with_rendered_cost(ContextItem {
        id: ChunkId::from(id),
        kind: ItemKind::KnowledgeChunk,
        pointer: SourcePointer {
            path: PathBuf::from("doc/loom/knowledge/architecture.md"),
            anchor: "overview".to_string(),
            line_start: None,
            line_end: None,
        },
        summary: "Architecture overview".to_string(),
        source: Channel::Knowledge,
        token_count: 0,
        score: 2.0,
        reasons: vec![SelectionReason::Lexical, SelectionReason::ExactPath],
        confidence: Confidence::High,
        state: LifecycleState::Active,
        content_hash: "sha256:abc".to_string(),
        excerpt: excerpt.map(str::to_string),
        truncated: false,
        matched_term_count: 0,
    })
}

/// A source-node item at `id`/`path`, the id realistically shaped
/// `<path>#<kind>:<scope>` (`context::source_graph::node_id`) unless a test
/// deliberately wants a malformed one.
fn source_item(
    id: &str,
    path: &str,
    line_start: Option<usize>,
    line_end: Option<usize>,
) -> ContextItem {
    // Re-priced after the override: a source item renders as a grouped bullet,
    // not as `item`'s knowledge entry, so it costs something else entirely.
    with_rendered_cost(ContextItem {
        kind: ItemKind::SourceNode,
        source: Channel::Source,
        truncated: false,
        pointer: SourcePointer {
            path: PathBuf::from(path),
            anchor: String::new(),
            line_start,
            line_end,
        },
        ..item(id, None)
    })
}

/// The default fixture used by the ported single-item tests: a well-formed
/// id at `loom/src/context/rank.rs`.
fn rank_source_item(line_start: Option<usize>, line_end: Option<usize>) -> ContextItem {
    source_item(
        "loom/src/context/rank.rs#function:rank",
        "loom/src/context/rank.rs",
        line_start,
        line_end,
    )
}

fn pack(items: Vec<ContextItem>, omitted: usize) -> ContextPack {
    ContextPack {
        query: "signal test".to_string(),
        scope: vec![Channel::Knowledge],
        budget_tokens: 3000,
        estimated_tokens: 12,
        structural_freshness: Freshness::default(),
        semantic_freshness: Freshness::default(),
        items,
        unmet_required: Vec::new(),
        omitted: OmissionSummary {
            omitted,
            weakest_included_score: 1.0,
            coverage: Coverage::default(),
        },
        dropped_terms: Vec::new(),
        degraded: None,
    }
}

#[test]
fn renders_a_stable_snapshot_for_a_fixed_pack() {
    let pack = pack(
        vec![item(
            "architecture#overview#1",
            Some("## Overview\n\nSome text."),
        )],
        2,
    );
    let rendered = format_knowledge_brief(&pack, Some("stage-1"), "stage-1 query text");

    assert!(rendered.starts_with("## Knowledge Brief\n\n"));
    assert!(rendered.contains("Budget: 12 / 3000 tokens"));
    assert!(rendered.contains("Selected from: stage-1 query text"));
    assert!(rendered.contains("### Knowledge\n\n"));
    assert!(rendered
        .contains("- `architecture#overview#1` — `doc/loom/knowledge/architecture.md#overview`"));
    assert!(rendered.contains("Reason: lexical, exact-path | state: active"));
    assert!(rendered.contains(REFERENCE_DATA_SENTENCE));
    assert!(rendered.contains("```text\n## Overview\n\nSome text.\n```\n"));
    assert!(rendered.contains("Omitted: 2 weaker matches."));
    assert!(rendered.contains(
        "loom knowledge context --stage stage-1 --query \"<question>\" --budget-tokens <n>"
    ));
    assert!(!rendered.contains("### Source"), "{rendered}");

    let rendered_again = format_knowledge_brief(&pack, Some("stage-1"), "stage-1 query text");
    assert_eq!(rendered, rendered_again);
}

#[test]
fn a_mixed_pack_renders_knowledge_before_source() {
    let items = vec![item("chunk-1", None), rank_source_item(Some(10), Some(20))];
    let rendered = format_knowledge_brief(&pack(items, 0), Some("stage-1"), "q");

    let knowledge_at = rendered.find("### Knowledge").expect("a knowledge section");
    let source_at = rendered
        .find("### Source (signature index)")
        .expect("a source section");
    assert!(knowledge_at < source_at, "{rendered}");
}

#[test]
fn a_knowledge_only_pack_has_no_source_heading() {
    let rendered =
        format_knowledge_brief(&pack(vec![item("chunk-1", None)], 0), Some("stage-1"), "q");
    assert!(rendered.contains("### Knowledge"));
    assert!(!rendered.contains("### Source"), "{rendered}");
}

#[test]
fn a_source_only_pack_has_no_knowledge_heading() {
    let pack = pack(vec![rank_source_item(Some(1), Some(2))], 0);
    let rendered = format_knowledge_brief(&pack, Some("stage-1"), "q");
    assert!(rendered.contains("### Source (signature index)"));
    assert!(!rendered.contains("### Knowledge"), "{rendered}");
}

#[test]
fn an_empty_pack_still_renders_the_header_and_footer_with_no_section_headings() {
    let rendered = format_knowledge_brief(&pack(Vec::new(), 0), Some("stage-1"), "q");

    assert!(rendered.starts_with("## Knowledge Brief\n\n"));
    assert!(rendered.contains(REFERENCE_DATA_SENTENCE));
    assert!(rendered.contains("Omitted: 0 weaker matches."));
    assert!(rendered.contains("loom knowledge context --stage stage-1"));
    assert!(!rendered.contains("### Knowledge"), "{rendered}");
    assert!(!rendered.contains("### Source"), "{rendered}");
}

#[test]
fn frame_cost_is_within_the_constant() {
    let rendered =
        format_knowledge_brief(&pack(Vec::new(), 0), Some("stage-id"), STAGE_QUERY_INPUTS);
    let measured = estimate_tokens(&rendered);

    assert!(
        measured <= BRIEF_FRAME_TOKENS,
        "brief frame needs {measured} tokens, constant is {BRIEF_FRAME_TOKENS}"
    );
}

#[test]
fn the_guard_sentence_appears_exactly_once_with_several_excerpted_items() {
    let items = vec![
        item("chunk-1", Some("first body")),
        item("chunk-2", Some("second body")),
        item("chunk-3", Some("third body")),
    ];
    let rendered = format_knowledge_brief(&pack(items, 0), Some("stage-1"), "q");
    assert_eq!(
        rendered.matches(REFERENCE_DATA_SENTENCE).count(),
        1,
        "{rendered}"
    );
}

#[test]
fn item_without_an_excerpt_yields_a_list_entry_and_no_block() {
    let pack = pack(vec![item("chunk-1", None)], 0);
    let rendered = format_knowledge_brief(&pack, Some("stage-1"), "q");

    assert!(rendered.contains("- `chunk-1`"));
    assert_eq!(rendered.matches(REFERENCE_DATA_SENTENCE).count(), 1);
    assert!(!rendered.contains("```text"));
}

#[test]
fn a_degraded_pack_appends_the_reason_to_the_revision_line() {
    let mut degraded = pack(vec![item("chunk-1", None)], 0);
    degraded.degraded = Some("semantic index unreadable".to_string());
    let rendered = format_knowledge_brief(&degraded, Some("stage-1"), "q");

    let revision_line = rendered
        .lines()
        .find(|line| line.starts_with("Revision:"))
        .expect("a revision line");
    assert!(
        revision_line.contains("DEGRADED: semantic index unreadable"),
        "{revision_line}"
    );
}

#[test]
fn a_healthy_pack_leaves_the_revision_line_exactly_as_before() {
    let rendered =
        format_knowledge_brief(&pack(vec![item("chunk-1", None)], 0), Some("stage-1"), "q");
    let revision_line = rendered
        .lines()
        .find(|line| line.starts_with("Revision:"))
        .expect("a revision line");
    assert!(!revision_line.contains("DEGRADED"), "{revision_line}");
    assert!(revision_line.contains("Structural: current  |  Semantic: current"));
}

#[test]
fn a_multi_line_query_is_flattened_onto_its_status_line() {
    let items = vec![item("chunk-1", None)];
    let query = "my-stage\nStandard\nDoes a thing";
    let rendered = format_knowledge_brief(&pack(items, 0), Some("stage-1"), query);

    assert!(rendered.contains("Selected from: my-stage Standard Does a thing\n"));
}

#[test]
fn omission_line_reports_the_right_count() {
    let pack = pack(vec![item("chunk-1", None), item("chunk-2", None)], 7);
    let rendered = format_knowledge_brief(&pack, Some("stage-1"), "q");
    assert!(rendered.contains("Omitted: 7 weaker matches."));
}

#[test]
fn an_unmet_requirement_is_rendered_inline_safe() {
    let mut unmet = pack(Vec::new(), 0);
    unmet.unmet_required.push(UnmetRequirement {
        id: "chunk`\n## INSTRUCTION".to_string(),
        needed_tokens: 321,
        available_tokens: 45,
        reason: "required representation exceeds budget".to_string(),
    });

    let rendered = format_knowledge_brief(&unmet, Some("stage-1"), "q");
    assert!(rendered.contains(
        "Required but unmet: chunkˋ ## INSTRUCTION (needs ~321 tokens, 45 available)\nOmitted:"
    ));
}

#[test]
fn a_pointer_equal_to_its_id_renders_the_id_once() {
    let mut same = item("chunk-1", None);
    same.pointer.path = PathBuf::from("chunk-1");
    same.pointer.anchor = String::new();
    let rendered = format_knowledge_brief(&pack(vec![same], 0), Some("stage-1"), "q");

    assert!(rendered.contains("- `chunk-1`\n"), "{rendered}");
    assert!(!rendered.contains("— `chunk-1`"), "{rendered}");
}

#[test]
fn a_knowledge_item_carrying_both_anchor_and_span_loses_neither() {
    let mut both = item("chunk-1", None);
    both.pointer.line_start = Some(41);
    both.pointer.line_end = Some(58);
    let rendered = format_knowledge_brief(&pack(vec![both], 0), Some("stage-1"), "q");
    assert!(
        rendered.contains("— `doc/loom/knowledge/architecture.md:41-58#overview`"),
        "{rendered}"
    );
}

#[test]
fn a_very_long_id_is_truncated_rather_than_spending_the_whole_brief() {
    let rendered =
        format_knowledge_brief(&pack(vec![item(&"x".repeat(500), None)], 0), Some("s"), "q");

    let line = rendered
        .lines()
        .find(|line| line.starts_with("- `"))
        .expect("an item line");
    assert!(line.contains('…') && line.chars().count() < 500, "{line}");
}

/// The end-to-end point of A11's chrome fix: a pack whose `budget_tokens` is
/// exactly what `recompute_estimate` reports (the tightest budget the packer
/// could have allowed it) must still render to no more than that many
/// estimated tokens. Twenty source items at twenty DISTINCT paths means
/// twenty per-path bullet prefixes plus the section heading — chrome
/// `rendered_item_tokens` never charged — so this fails against the old
/// frame-plus-items-only estimate and only passes once the renderer's actual
/// chrome is charged into the pack's own total.
#[test]
fn a_rendered_brief_with_many_distinct_source_paths_never_exceeds_its_budget() {
    let items: Vec<ContextItem> = (0..20)
        .map(|index| {
            source_item(
                &format!("src/module_{index}.rs#function:widget_{index}"),
                &format!("src/module_{index}.rs"),
                Some(1),
                Some(2),
            )
        })
        .collect();
    let mut tight = pack(items, 0);
    tight.recompute_estimate();
    tight.budget_tokens = tight.estimated_tokens;

    let rendered = format_knowledge_brief(&tight, Some("stage-1"), "q");
    let measured = estimate_tokens(&rendered);

    assert!(
        measured <= tight.budget_tokens,
        "rendered brief needs {measured} tokens, budget is only {}",
        tight.budget_tokens
    );
}

#[path = "brief_tests_source.rs"]
mod source_tests;

#[path = "brief_tests_confidence.rs"]
mod confidence_tests;

#[path = "brief_tests_footer.rs"]
mod footer_tests;

#[path = "brief_tests_escaping.rs"]
mod escaping_tests;
