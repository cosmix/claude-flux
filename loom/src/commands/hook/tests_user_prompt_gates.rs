//! Tests for the emit floor and the dedupe-omitted count.
//!
//! Split out of `tests_user_prompt.rs` itself so that file stays under the
//! maintainability line limit — same idiom `tests_user_prompt.rs` uses for its
//! own `tests_user_prompt_e2e.rs` child: no repeated `#[cfg(test)]`, since this
//! whole file is already gated by it.

use super::super::compose::compose;
use super::ContextItem;
use super::{default_config, delivered, item, pack_of};
use crate::context::schema::{ItemKind, SelectionReason};
use std::collections::BTreeSet;

/// A source-graph unit scored only by loose lexical overlap: no exact-rung
/// reason, and `matched_term_count` left at zero so it clears no term-count
/// floor either. Callers that want the floor case bump the count back up —
/// this is the shape the emit floor exists to silence.
fn weak_source_item(id: &str) -> ContextItem {
    let mut unit = item(id, "sha256:aa", Some("some excerpt"));
    unit.kind = ItemKind::SourceNode;
    unit.reasons = vec![SelectionReason::Lexical];
    unit.matched_term_count = 0;
    unit
}

#[test]
fn a_weak_lexical_source_item_alone_clears_no_floor() {
    let pack = pack_of(vec![weak_source_item("src#zorble#0")]);
    assert!(
        compose(Some("stage-a"), &pack, &BTreeSet::new(), &default_config()).is_none(),
        "one weak lexical match on a source node says nothing worth surfacing"
    );
}

/// The regression case, and the reason for this change: a source-graph
/// checkout with no curated knowledge tree has no `ItemKind::KnowledgeChunk`
/// items at all, so before the fix a pack of nothing but `SourceNode` items
/// could clear the floor only via an exact rung — withholding every
/// conceptual prompt, which is most of them, from the only channel that
/// checkout has. A pack of only source nodes, none with an exact rung, where
/// one clears the term floor on lexical overlap alone, must now be emitted.
#[test]
fn a_pack_of_only_source_nodes_is_emitted_once_one_clears_the_term_floor() {
    let config = default_config();
    let mut strong = weak_source_item("src#zorble#0");
    strong.matched_term_count = config.min_knowledge_terms;
    let pack = pack_of(vec![weak_source_item("src#other#0"), strong]);

    assert!(
        compose(Some("stage-a"), &pack, &BTreeSet::new(), &config).is_some(),
        "a source node clearing the term floor must be emitted like a knowledge chunk would be"
    );
}

/// The floor must still be a floor after widening it to source nodes: a pack
/// of only source nodes, every one short of the threshold, stays silent —
/// this change must not become "always emit".
#[test]
fn a_pack_of_only_source_nodes_all_below_the_term_floor_is_still_silent() {
    let config = default_config();
    let mut unit = weak_source_item("src#zorble#0");
    unit.matched_term_count = config.min_knowledge_terms - 1;
    let pack = pack_of(vec![weak_source_item("src#other#0"), unit]);

    assert!(
        compose(Some("stage-a"), &pack, &BTreeSet::new(), &config).is_none(),
        "one term short of the floor is exactly the case it exists to silence, source node or not"
    );
}

/// The boundary, mirrored for a source node: exactly `min_knowledge_terms`
/// clears the floor, matching `a_knowledge_item_at_exactly_the_term_floor_is_emitted`.
#[test]
fn a_source_item_exactly_at_the_term_floor_is_emitted() {
    let config = default_config();
    let mut unit = weak_source_item("src#zorble#0");
    unit.matched_term_count = config.min_knowledge_terms;
    let pack = pack_of(vec![unit]);

    assert!(
        compose(Some("stage-a"), &pack, &BTreeSet::new(), &config).is_some(),
        "the boundary: exactly min_knowledge_terms clears the floor for a source node too"
    );
}

#[test]
fn an_exact_rung_item_clears_the_floor_on_its_own() {
    let mut unit = weak_source_item("src#zorble#0");
    unit.reasons = vec![SelectionReason::ExactSymbol];
    let pack = pack_of(vec![unit]);
    assert!(
        compose(Some("stage-a"), &pack, &BTreeSet::new(), &default_config()).is_some(),
        "an exact-rung reason clears the floor regardless of term count"
    );
}

#[test]
fn a_knowledge_item_at_exactly_the_term_floor_is_emitted() {
    let config = default_config();
    let mut unit = item("arch#loop#0", "sha256:aa", Some("The loop polls."));
    unit.matched_term_count = config.min_knowledge_terms;
    let pack = pack_of(vec![unit]);

    assert!(
        compose(Some("stage-a"), &pack, &BTreeSet::new(), &config).is_some(),
        "the boundary: exactly min_knowledge_terms clears the floor"
    );
}

#[test]
fn a_knowledge_item_one_below_the_term_floor_is_silent() {
    let config = default_config();
    let mut unit = item("arch#loop#0", "sha256:aa", Some("The loop polls."));
    unit.matched_term_count = config.min_knowledge_terms - 1;
    let pack = pack_of(vec![unit]);

    assert!(
        compose(Some("stage-a"), &pack, &BTreeSet::new(), &config).is_none(),
        "one below the floor is exactly the case it exists to silence"
    );
}

#[test]
fn a_dedupe_drop_is_folded_into_the_omitted_count() {
    let pack = pack_of(vec![
        item("arch#loop#0", "sha256:aa", Some("The loop polls.")),
        item("arch#merge#0", "sha256:bb", Some("Merging is verified.")),
        item(
            "arch#verify#0",
            "sha256:cc",
            Some("Verification runs after."),
        ),
    ]);
    let two_already_delivered =
        delivered(&[("arch#loop#0", "sha256:aa"), ("arch#merge#0", "sha256:bb")]);

    let (_, handed_over) = compose(
        Some("stage-a"),
        &pack,
        &two_already_delivered,
        &default_config(),
    )
    .expect("one undelivered unit survives");

    assert_eq!(
        handed_over.omitted.omitted,
        pack.omitted.omitted + 2,
        "the two deduped units must count toward what the footer reports"
    );
}
