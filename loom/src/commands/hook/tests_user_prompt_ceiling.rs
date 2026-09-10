//! Tests for the byte ceiling: a pack too big to emit whole either sheds its
//! weakest units until it fits, or — with nothing left to shed — emits
//! nothing at all.
//!
//! Split out of `tests_user_prompt.rs` itself so that file stays under the
//! maintainability line limit — same idiom `tests_user_prompt.rs` uses for its
//! own `tests_user_prompt_e2e.rs` child: no repeated `#[cfg(test)]`, since this
//! whole file is already gated by it.

use super::super::compose::compose;
use super::ContextItem;
use super::{brief_of, default_config, item, pack_of};
use crate::context::render::rendered_chrome_tokens;
use crate::context::schema::BRIEF_FRAME_TOKENS;
use std::collections::BTreeSet;

/// One unit with an explicit rank, for the cases that turn on which unit is the
/// weakest.
fn scored(id: &str, score: f32, excerpt: &str) -> ContextItem {
    let mut unit = item(id, "sha256:aa", Some(excerpt));
    unit.score = score;
    unit
}

#[test]
fn an_empty_pack_produces_no_payload() {
    assert!(compose(
        Some("stage-a"),
        &pack_of(Vec::new()),
        &BTreeSet::new(),
        &default_config()
    )
    .is_none());
}

#[test]
fn a_single_unit_over_the_ceiling_is_not_emitted() {
    let config = default_config();
    let excerpt = "x".repeat(config.max_payload_bytes + 1);
    let pack = pack_of(vec![item("arch#loop#0", "sha256:aa", Some(&excerpt))]);

    // Nothing left to shed: one unit that does not fit cannot be trimmed into
    // fitting, so this is the one case that still emits nothing.
    assert!(compose(Some("stage-a"), &pack, &BTreeSet::new(), &config).is_none());
}

#[test]
fn an_oversized_pack_sheds_its_weakest_units_until_it_fits() {
    let config = default_config();
    let body = "y".repeat(9 * 1024);
    let pack = pack_of(vec![
        scored("arch#strong#0", 9.0, &body),
        scored("arch#middling#0", 5.0, &body),
        scored("arch#weak#0", 1.0, &body),
    ]);

    let (line, handed_over) = compose(Some("stage-a"), &pack, &BTreeSet::new(), &config)
        .expect("a trimmed payload, not silence");

    assert!(
        line.len() <= config.max_payload_bytes,
        "{} bytes",
        line.len()
    );
    let ids: Vec<&str> = handed_over
        .items
        .iter()
        .map(|item| item.id.as_str())
        .collect();
    assert_eq!(ids, vec!["arch#strong#0"], "the strongest match survives");
    assert_eq!(
        handed_over.estimated_tokens,
        BRIEF_FRAME_TOKENS
            + handed_over.items[0].token_count
            + rendered_chrome_tokens(handed_over.items.iter(), &handed_over.unmet_required),
        "the estimate must describe what is actually handed over, frame and chrome included"
    );
    // The delivery record is written from `handed_over`, so it can only ever
    // list what was really emitted.
    assert_eq!(handed_over.omitted.omitted, 2);
    assert!(brief_of(&line).contains("Omitted: 2 weaker matches."));
}
