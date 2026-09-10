//! Reservation of explicitly required candidates before optional packing.

use super::{build_item, knowledge_twin, tentative_total, PackRequest};
use crate::context::rank::RankedCandidate;
use crate::context::schema::{
    ContextItem, KnowledgeChunk, SelectionReason, SourceNode, UnmetRequirement,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Reservation {
    pub(super) items: Vec<ContextItem>,
    pub(super) unmet: Vec<UnmetRequirement>,
    pub(super) estimated_tokens: usize,
    pub(super) omitted: usize,
    pub(super) superseded: BTreeSet<String>,
}

/// `request` carries `budget_tokens`, `surviving_terms` and
/// `required_representation` as one borrowed unit rather than three loose
/// parameters — the same shape [`Reservation`] models for this function's
/// output.
pub(super) fn reserve(
    request: &PackRequest,
    initial_tokens: usize,
    ranked: &[RankedCandidate],
    chunks: &BTreeMap<&str, &KnowledgeChunk>,
    nodes: &BTreeMap<&str, &SourceNode>,
) -> Reservation {
    let mut reservation = Reservation {
        items: Vec::new(),
        unmet: Vec::new(),
        estimated_tokens: initial_tokens,
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

        match fit_decision(request, &reservation, candidate, &item) {
            Ok(total) => {
                reservation.estimated_tokens = total;
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

/// Whether `item` fits `request.budget_tokens` once weighed against
/// `reservation`'s items and unmet lines so far — via the same helper
/// `select_optional` uses, not against a running scalar: chrome — the
/// section heading, this item's own group prefix if its path starts a new
/// run — is a function of the whole item list, so a per-candidate running
/// total can never price it in. See `tentative_total`'s doc comment.
///
/// `Ok` carries the reservation's new running total; `Err` carries the
/// [`UnmetRequirement`] line to report instead.
fn fit_decision(
    request: &PackRequest,
    reservation: &Reservation,
    candidate: &RankedCandidate,
    item: &ContextItem,
) -> Result<usize, UnmetRequirement> {
    let total = tentative_total(&reservation.items, &reservation.unmet, item);
    if total <= request.budget_tokens {
        return Ok(total);
    }
    // What's left for the candidate's own body once the chrome its inclusion
    // would add is paid for — chosen so `needed_tokens > available_tokens`
    // stays equivalent to `total > budget_tokens`.
    let available = request
        .budget_tokens
        .saturating_sub(total.saturating_sub(item.token_count));
    Err(UnmetRequirement {
        id: candidate.id.as_str().to_string(),
        needed_tokens: item.token_count,
        available_tokens: available,
        reason: "required item exceeds the remaining budget".to_string(),
    })
}
