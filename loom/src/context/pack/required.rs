//! Reservation of explicitly required candidates before optional packing.

use super::{build_item, knowledge_twin};
use crate::context::rank::RankedCandidate;
use crate::context::schema::{
    ContextItem, KnowledgeChunk, RequiredRepresentation, SelectionReason, SourceNode,
    UnmetRequirement,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Reservation {
    pub(super) items: Vec<ContextItem>,
    pub(super) unmet: Vec<UnmetRequirement>,
    pub(super) estimated_tokens: usize,
    pub(super) omitted: usize,
    pub(super) superseded: BTreeSet<String>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reserve(
    budget_tokens: usize,
    initial_tokens: usize,
    ranked: &[RankedCandidate],
    chunks: &BTreeMap<&str, &KnowledgeChunk>,
    nodes: &BTreeMap<&str, &SourceNode>,
    terms: &[String],
    representation: RequiredRepresentation,
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
        if let Some(twin) = knowledge_twin(candidate) {
            reservation.superseded.insert(twin);
        }
        let Some(item) = build_item(candidate, chunks, nodes, terms, representation) else {
            reservation.omitted += 1;
            continue;
        };
        let available = budget_tokens.saturating_sub(reservation.estimated_tokens);
        if item.token_count > available {
            reservation.unmet.push(UnmetRequirement {
                id: candidate.id.as_str().to_string(),
                needed_tokens: item.token_count,
                available_tokens: available,
                reason: "required item exceeds the remaining budget".to_string(),
            });
            reservation.omitted += 1;
            continue;
        }

        reservation.estimated_tokens += item.token_count;
        reservation.items.push(item);
    }
    reservation
}
