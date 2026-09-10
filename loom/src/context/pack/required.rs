//! Reservation of explicitly required candidates before optional packing.

use super::{build_item, knowledge_twin, tentative_total, PackRequest};
use crate::context::rank::RankedCandidate;
use crate::context::render::{rendered_brief_tokens, rendered_chrome_tokens};
use crate::context::schema::{
    ContextItem, KnowledgeChunk, SelectionReason, SourceNode, UnmetRequirement,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Reservation {
    pub(super) items: Vec<ContextItem>,
    pub(super) unmet: Vec<UnmetRequirement>,
    pub(super) omitted: usize,
    pub(super) superseded: BTreeSet<String>,
}

impl Reservation {
    /// What this reservation finalizes to once the pack charges its items and
    /// chrome — the number [`crate::context::schema::ContextPack::within_budget`]
    /// is decided on.
    fn total(&self) -> usize {
        rendered_brief_tokens(self.items.iter(), &self.unmet)
    }

    /// The cost of this reservation's unmet lines alone, plus the one token of
    /// floor-division slack their block can gain when `estimate_tokens`
    /// measures it concatenated after the items' chrome rather than on its own.
    fn unmet_cost(&self) -> usize {
        rendered_chrome_tokens(std::iter::empty(), &self.unmet) + 1
    }
}

/// [`reserve`], re-run against a budget held back by what its own unmet lines
/// cost, until what it produces fits `request.budget_tokens`.
///
/// A single pass cannot: it prices each required candidate against the unmet
/// lines that exist WHEN THAT CANDIDATE IS WEIGHED, while
/// `rendered_chrome_tokens` charges every unmet line in the finished pack, so
/// a line pushed after an item was admitted is never priced against the budget
/// it lands in. `--budget-tokens 256 --require-id A --require-id B`, with A
/// rendering to ~120 tokens and B too large, admits A at 251, then reports B
/// unmet for ~25 more and finalizes at 276 — a brief whose own `Budget:`
/// header advertises the overshoot.
///
/// Holding the measured unmet cost back and re-reserving is normally the end
/// of it. A further pass happens only when the smaller budget turns another
/// required candidate away, since that adds a line nobody had held budget for.
/// `held_back` grows strictly on every retry and is capped at the budget
/// itself, where nothing can be admitted at all and the walk stops: at most
/// one pass per required candidate, and every pass deterministic.
///
/// Admitted items are never evicted to make room for a line. An unmet line is
/// not reliably cheaper than the row it would replace — the source-item test
/// in `tests::pack_required` turns on exactly the opposite case, a row whose
/// huge span costs far more than reporting it unmet — so trading one for the
/// other can overshoot rather than recover. Reserving less budget instead can
/// only ever admit fewer items, never cost more.
pub(super) fn reserve_within_budget(
    request: &PackRequest,
    ranked: &[RankedCandidate],
    chunks: &BTreeMap<&str, &KnowledgeChunk>,
    nodes: &BTreeMap<&str, &SourceNode>,
) -> Reservation {
    let mut held_back = 0;
    loop {
        let reservation = reserve(
            request,
            request.budget_tokens - held_back,
            ranked,
            chunks,
            nodes,
        );
        if reservation.total() <= request.budget_tokens {
            return reservation;
        }
        let needed = reservation.unmet_cost().min(request.budget_tokens);
        if needed <= held_back {
            // Either the retry would reserve no more than this one did — in
            // which case it would produce this same reservation — or the whole
            // budget is already held back and nothing can be admitted. What is
            // left over budget is then the unmet lines themselves, which the
            // brief must report whatever they cost.
            return reservation;
        }
        held_back = needed;
    }
}

/// One reservation pass over the explicitly required candidates, admitting
/// each that fits `budget_tokens` and reporting the rest unmet.
///
/// `budget_tokens` is a parameter rather than `request.budget_tokens` because
/// [`reserve_within_budget`] re-runs this against a REDUCED budget; everything
/// else a pass needs (`surviving_terms`, `required_representation`) rides on
/// `request` as one borrowed unit.
fn reserve(
    request: &PackRequest,
    budget_tokens: usize,
    ranked: &[RankedCandidate],
    chunks: &BTreeMap<&str, &KnowledgeChunk>,
    nodes: &BTreeMap<&str, &SourceNode>,
) -> Reservation {
    let mut reservation = Reservation {
        items: Vec::new(),
        unmet: Vec::new(),
        omitted: 0,
        superseded: BTreeSet::new(),
    };

    for candidate in ranked
        .iter()
        .filter(|candidate| candidate.reasons.contains(&SelectionReason::ExplicitId))
    {
        let Some(item) = build_item(
            candidate,
            chunks,
            nodes,
            &request.surviving_terms,
            request.required_representation,
        ) else {
            reservation.omitted += 1;
            continue;
        };

        match fit_decision(budget_tokens, &reservation, candidate, &item) {
            Ok(()) => {
                // Only suppress the tier-1 twin once the detail actually
                // makes the pack (`pack::twins`'s module doc) — earlier
                // would suppress the summary for a detail that ends up unmet.
                if let Some(twin) = knowledge_twin(candidate) {
                    reservation.superseded.insert(twin);
                }
                reservation.items.push(item);
            }
            Err(unmet) => {
                reservation.unmet.push(unmet);
                reservation.omitted += 1;
            }
        }
    }
    reservation
}

/// Whether `item` fits `budget_tokens` once weighed against `reservation`'s
/// items and unmet lines so far — via the same helper `select_optional` uses,
/// not against a running scalar: chrome — the section heading, this item's own
/// group prefix if its path starts a new run — is a function of the whole item
/// list, so a per-candidate running total can never price it in. See
/// `tentative_total`'s doc comment.
///
/// `Err` carries the [`UnmetRequirement`] line to report instead.
fn fit_decision(
    budget_tokens: usize,
    reservation: &Reservation,
    candidate: &RankedCandidate,
    item: &ContextItem,
) -> Result<(), UnmetRequirement> {
    let total = tentative_total(&reservation.items, &reservation.unmet, item);
    if total <= budget_tokens {
        return Ok(());
    }
    // What's left for the candidate's own body once the chrome its inclusion
    // would add is paid for — chosen so `needed_tokens > available_tokens`
    // stays equivalent to `total > budget_tokens`.
    let available = budget_tokens.saturating_sub(total.saturating_sub(item.token_count));
    Err(UnmetRequirement {
        id: candidate.id.as_str().to_string(),
        needed_tokens: item.token_count,
        available_tokens: available,
        reason: "required item exceeds the remaining budget".to_string(),
    })
}
