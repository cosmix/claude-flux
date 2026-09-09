//! Confidence-label tests: what a demoted item renders, and what a High one
//! deliberately does not.
//!
//! Split out of `brief_tests.rs` (already near its own line budget) into a
//! sibling wired the same way `brief_tests_source.rs` is:
//! `#[path = "brief_tests_confidence.rs"] mod confidence_tests;`.

use super::super::format_knowledge_brief;
use super::{item, pack, rank_source_item};
use crate::context::schema::{Confidence, ContextItem};

/// The fixture item, demoted to `confidence` without touching its reasons —
/// exactly the state the packer publishes when the rung ceiling is weaker than
/// the reasons imply.
fn demoted(confidence: Confidence) -> ContextItem {
    ContextItem {
        confidence,
        truncated: false,
        ..item("chunk-1", None)
    }
}

#[test]
fn a_medium_knowledge_item_says_so_after_its_reasons() {
    let rendered =
        format_knowledge_brief(&pack(vec![demoted(Confidence::Medium)], 0), Some("s"), "q");

    assert!(
        rendered.contains("Reason: lexical, exact-path; medium | state: active"),
        "{rendered}"
    );
}

#[test]
fn a_low_knowledge_item_says_so_after_its_reasons() {
    let rendered = format_knowledge_brief(&pack(vec![demoted(Confidence::Low)], 0), Some("s"), "q");

    assert!(
        rendered.contains("Reason: lexical, exact-path; low | state: active"),
        "{rendered}"
    );
}

/// The common case must stay byte-identical to what it rendered before the
/// label existed: a High item that spent tokens announcing its own confidence
/// would spend them on every item of every brief.
#[test]
fn a_high_knowledge_item_renders_no_confidence_at_all() {
    let rendered =
        format_knowledge_brief(&pack(vec![demoted(Confidence::High)], 0), Some("s"), "q");

    assert!(
        rendered.contains("Reason: lexical, exact-path | state: active"),
        "{rendered}"
    );
    assert!(!rendered.contains("high"), "{rendered}");
    assert!(!rendered.contains("medium"), "{rendered}");
    assert!(!rendered.contains("; low"), "{rendered}");
}

#[test]
fn a_demoted_source_item_carries_its_label_inside_the_reason_parentheses() {
    let demoted = ContextItem {
        confidence: Confidence::Medium,
        truncated: false,
        ..rank_source_item(Some(23), Some(28))
    };
    let rendered = format_knowledge_brief(&pack(vec![demoted], 0), Some("s"), "q");

    assert!(
        rendered.contains("— `rank` function :23-28 (lexical, exact-path; medium)"),
        "a coincidental symbol hit must not read like a certain one: {rendered}"
    );
}

#[test]
fn a_high_source_item_renders_its_reasons_alone() {
    let rendered = format_knowledge_brief(
        &pack(vec![rank_source_item(Some(1), None)], 0),
        Some("s"),
        "q",
    );

    assert!(
        rendered.contains("— `rank` function :1 (lexical, exact-path)"),
        "{rendered}"
    );
}
