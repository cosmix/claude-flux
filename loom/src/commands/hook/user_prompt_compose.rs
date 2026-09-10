//! Turn one retrieved [`ContextPack`] into the hook's single stdout line, or
//! into nothing.
//!
//! Split out of `user_prompt.rs` to keep that file under the maintainability
//! line limit: recipient resolution and delivery filing are one concern,
//! composing a pack into a payload is another, and this file owns only the
//! second. Items must first earn admission individually, then survive the
//! recipient's per-epoch dedupe, and finally fit the serialized byte ceiling.

use crate::context::config::RetrievalConfig;
use crate::context::schema::{ContextItem, ContextPack, SelectionReason};
use std::collections::BTreeSet;

/// What the shared renderer reports on its `Selected from:` line. A prompt hook
/// retrieves against the one question that was just typed, not against the
/// stage's whole query surface.
const QUERY_INPUTS: &str = "this prompt";

pub(super) enum ComposeOutcome {
    Emitted(String, Box<ContextPack>),
    Abstained(&'static str),
}

/// Compose the hook's single stdout line, together with the pack that line
/// actually delivers.
///
/// The brief itself is rendered by
/// [`crate::orchestrator::signals::format_knowledge_brief`] — the same renderer
/// the signal path uses. Its fencing rule is what keeps an untrusted excerpt
/// from escaping its own quoted block, and a containment rule with two copies
/// is a containment rule that drifts, so this path never forks it. Only the
/// emit floor, the per-epoch dedupe, and the serialized ceiling are the
/// hook's own.
///
/// An over-budget pack DEGRADES rather than being discarded: its weakest units
/// are dropped until the object fits. Discarding instead would recompose and
/// throw away the same pack on every prompt for the rest of the epoch, and would
/// file no record — so the strongest matches, which do fit, would never arrive.
///
/// `None` means there is no honest payload to send. [`compose_with_reason`]
/// retains the reason for the telemetry-producing caller.
#[cfg(test)]
pub(super) fn compose(
    pull_stage: Option<&str>,
    pack: &ContextPack,
    delivered: &BTreeSet<(String, String)>,
    config: &RetrievalConfig,
) -> Option<(String, ContextPack)> {
    match compose_with_reason(pull_stage, pack, delivered, config) {
        ComposeOutcome::Emitted(payload, handed_over) => Some((payload, *handed_over)),
        ComposeOutcome::Abstained(_) => None,
    }
}

pub(super) fn compose_with_reason(
    pull_stage: Option<&str>,
    pack: &ContextPack,
    delivered: &BTreeSet<(String, String)>,
    config: &RetrievalConfig,
) -> ComposeOutcome {
    let Some(admitted) = admitted(pack, config) else {
        return ComposeOutcome::Abstained("floor");
    };
    let Some(mut handed_over) = undelivered(&admitted, delivered) else {
        return ComposeOutcome::Abstained("all-delivered");
    };
    // `undelivered` already dropped every repeat; what survives here is fresh,
    // just too weak on its own — that is the emit floor's call, not a repeat,
    // so it gets the same reason a first-time pull that never cleared the
    // floor would get.
    if !clears_emit_floor(&handed_over, config) {
        return ComposeOutcome::Abstained("floor");
    }

    loop {
        let Some(line) = render_payload(pull_stage, &handed_over) else {
            return ComposeOutcome::Abstained("over-ceiling");
        };
        if line.len() <= config.max_payload_bytes {
            return ComposeOutcome::Emitted(line, Box::new(handed_over));
        }
        let Some(narrowed) = without_weakest(&handed_over) else {
            return ComposeOutcome::Abstained("over-ceiling");
        };
        handed_over = narrowed;
        if !clears_emit_floor(&handed_over, config) {
            return ComposeOutcome::Abstained("over-ceiling");
        }
    }
}

/// Would a fresh session receive a hook payload for this pack?
pub(crate) fn would_emit(pack: &ContextPack, config: &RetrievalConfig) -> bool {
    matches!(
        compose_with_reason(None, pack, &BTreeSet::new(), config),
        ComposeOutcome::Emitted(_, _)
    )
}

/// True when `item` earns admission on its own, or is a graph neighbour of
/// an exact-rung item that remains in this retrieved pack.
fn admits(item: &ContextItem, pack: &ContextPack, config: &RetrievalConfig) -> bool {
    clears_item_floor(item, config)
        || (item.reasons.contains(&SelectionReason::GraphNeighbor)
            && pack
                .items
                .iter()
                .any(|item| item.reasons.iter().any(is_exact_rung)))
}

/// Narrow `pack` to individually admitted items and account for every drop.
fn admitted(pack: &ContextPack, config: &RetrievalConfig) -> Option<ContextPack> {
    let (kept, dropped): (Vec<ContextItem>, Vec<ContextItem>) = pack
        .items
        .iter()
        .cloned()
        .partition(|item| admits(item, pack, config));
    carrying_after_drop(pack, kept, dropped.len())
}

/// True when `pack` is worth emitting at all.
///
/// Silence is cheaper than a low-confidence brief that says nothing, and —
/// the real point — once silence is meaningful, a reader can trust PRESENCE as
/// signal. A brief that appears for every prompt carries no information by
/// appearing, so this only clears for a pack that actually found something.
///
/// Only ONE item needs to clear the bar: an exact-rung [`SelectionReason`]
/// (see [`is_exact_rung`] — these are post-gating reasons now, so a hit on one
/// of them means something a bare lexical score does not), or a lexical match
/// on at least `config.min_knowledge_terms` distinct query terms —
/// `matched_term_count` is exactly that per-item strength signal, carried on
/// the item for this reason.
///
/// The term-count clause applies to ANY item, not just a `KnowledgeChunk` —
/// deliberately, not as a loosening. `matched_term_count` counts DISTINCT
/// SURVIVING query terms: corpus-ubiquitous terms are already gone, stripped
/// by the stopwording pass in `context/rank/corpus/stopwords.rs`, so two
/// surviving terms is genuine evidence whichever channel produced it. If
/// anything the bar is HARDER for a source node than a knowledge chunk: its
/// BM25 document is only its scope segments plus a one-line signature
/// (`rank_source.rs::node_document`), a far smaller surface than a prose
/// chunk's whole body, so matching two distinct surviving terms there is a
/// stronger signal than the same count against a knowledge chunk.
///
/// A knowledge-only clause would silently blackout a real configuration: a
/// checkout with a mapped source graph (`loom map`) but no curated knowledge
/// tree has no `KnowledgeChunk` items at all, so a knowledge-only second
/// clause could never fire — every prompt that does not spell an identifier
/// in identifier form (most of them; see the identifier-shaped-evidence
/// gating behind [`is_exact_rung`]) would retrieve nothing, permanently, for
/// exactly the questions people actually ask. The floor's job is "is there
/// enough signal to say anything", not "did this come from curated prose".
///
/// **This floor applies only to this hook's unsolicited injection.**
/// `loom knowledge context` is NOT floor-gated — it prints what it found
/// because a human asked for exactly that — and the stage spawn brief is NOT
/// floor-gated either, because an autonomous session should see the best
/// available retrieval even when every match is weak. Both live outside this
/// file (`commands::knowledge::context`, `orchestrator::signals`), so this
/// predicate does not special-case them — that omission is deliberate, not an
/// oversight: do not "unify" the three paths by adding this floor to the
/// other two.
fn clears_emit_floor(pack: &ContextPack, config: &RetrievalConfig) -> bool {
    pack.items
        .iter()
        .any(|item| clears_item_floor(item, config))
}

fn clears_item_floor(item: &ContextItem, config: &RetrievalConfig) -> bool {
    item.reasons.iter().any(is_exact_rung) || item.matched_term_count >= config.min_knowledge_terms
}

/// A [`SelectionReason`] strong enough to justify emitting on its own — every
/// rung above plain lexical overlap.
fn is_exact_rung(reason: &SelectionReason) -> bool {
    matches!(
        reason,
        SelectionReason::ExplicitId
            | SelectionReason::ExactPath
            | SelectionReason::ExactSymbol
            | SelectionReason::LinkedFrom
            | SelectionReason::StageDependency
    )
}

/// The single stdout line for `handed_over`: the shared brief wrapped in the
/// hook's JSON envelope.
fn render_payload(pull_stage: Option<&str>, handed_over: &ContextPack) -> Option<String> {
    let brief =
        crate::orchestrator::signals::format_knowledge_brief(handed_over, pull_stage, QUERY_INPUTS);
    let payload = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "UserPromptSubmit",
            "additionalContext": brief,
        }
    });
    serde_json::to_string(&payload).ok()
}

/// `pack` minus every unit already delivered to this recipient this epoch, or
/// `None` when nothing survives.
///
/// A unit dropped here for dedupe is folded into `omitted` the same way
/// [`without_weakest`] folds its own per-unit drops in: `pack.omitted` as
/// retrieval built it describes only what did not fit the BUDGET, so left
/// alone it would tell the reader they were handed everything retrieval found,
/// when some of it was simply repeated from an earlier prompt this epoch.
fn undelivered(pack: &ContextPack, delivered: &BTreeSet<(String, String)>) -> Option<ContextPack> {
    let (kept, dropped): (Vec<ContextItem>, Vec<ContextItem>) =
        pack.items.iter().cloned().partition(|item| {
            let key = (item.id.as_str().to_string(), item.content_hash.clone());
            !delivered.contains(&key)
        });
    carrying_after_drop(pack, kept, dropped.len())
}

fn carrying_after_drop(
    pack: &ContextPack,
    kept: Vec<ContextItem>,
    dropped: usize,
) -> Option<ContextPack> {
    if kept.is_empty() {
        return None;
    }
    let mut narrowed = carrying(pack, kept);
    narrowed.omitted.omitted += dropped;
    if dropped > 0 {
        narrowed.omitted.weakest_included_score = narrowed
            .items
            .iter()
            .map(|item| item.score)
            .fold(f32::INFINITY, f32::min);
    }
    Some(narrowed)
}

/// `pack` without its lowest-scoring unit, or `None` when a single unit is all
/// that is left — one unit that does not fit cannot be trimmed into fitting.
fn without_weakest(pack: &ContextPack) -> Option<ContextPack> {
    if pack.items.len() <= 1 {
        return None;
    }
    let weakest = (0..pack.items.len()).min_by(|&left, &right| {
        pack.items[left]
            .score
            .total_cmp(&pack.items[right].score)
            // Ties drop the later unit: pack order is strongest first.
            .then(right.cmp(&left))
    })?;

    let mut items = pack.items.clone();
    items.remove(weakest);
    let mut narrowed = carrying(pack, items);
    // A unit dropped for size is a ranked candidate that did not fit, which is
    // exactly what the brief's "Omitted: N weaker matches" line reports. Left
    // alone it would tell the reader it had been given everything.
    narrowed.omitted.omitted += 1;
    narrowed.omitted.weakest_included_score = narrowed
        .items
        .iter()
        .map(|item| item.score)
        .fold(f32::INFINITY, f32::min);
    Some(narrowed)
}

/// `pack` carrying exactly `items`, with the token estimate that describes
/// them.
///
/// `coverage.included`/`coverage.included_tokens` are recomputed from `items`
/// too, alongside the caller's own `omitted.omitted` bump: `pack.omitted` as
/// retrieval built it describes the pack this narrowed from, and left alone
/// it would tell the reader more candidates fit than actually shipped —
/// breaking the `included + omitted == candidates` invariant the packer's own
/// property test holds it to (`context::tests::pack::property_pack_never_exceeds_budget`).
fn carrying(pack: &ContextPack, items: Vec<ContextItem>) -> ContextPack {
    let mut narrowed = pack.clone();
    narrowed.items = items;
    narrowed.recompute_estimate();
    narrowed.omitted.coverage.included = narrowed.items.len();
    narrowed.omitted.coverage.included_tokens =
        narrowed.items.iter().map(|item| item.token_count).sum();
    narrowed
}
