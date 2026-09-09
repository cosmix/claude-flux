//! Shared fixtures for the packer tests split across `pack.rs` and
//! `pack_required.rs`, mirroring `rank_fixtures.rs`'s style for the ranker.
//!
//! `pub(super)` makes every item here visible to any sibling module under
//! `context::tests` via `use super::pack_fixtures::...`.

use crate::context::pack::PackRequest;
use crate::context::rank::RankedCandidate;
use crate::context::schema::{
    Channel, ChunkId, Freshness, KnowledgeChunk, LifecycleState, RequiredRepresentation,
    SelectionReason, BRIEF_FRAME_TOKENS,
};
use std::path::PathBuf;

pub(super) fn chunk(id: &str, body: &str, tokens: usize) -> KnowledgeChunk {
    KnowledgeChunk {
        id: id.to_string(),
        file: PathBuf::from(format!("{id}.md")),
        anchor: format!("{id}-anchor"),
        heading: format!("{id} heading"),
        body: body.to_string(),
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

pub(super) fn candidate(
    id: &str,
    channel: Channel,
    score: f32,
    token_count: usize,
) -> RankedCandidate {
    RankedCandidate {
        id: ChunkId::from(id),
        channel,
        score,
        reasons: vec![SelectionReason::Lexical],
        token_count,
        matched_term_count: 1,
        confidence_ceiling: None,
    }
}

pub(super) fn required_candidate(id: &str, token_count: usize) -> RankedCandidate {
    let mut candidate = candidate(id, Channel::Knowledge, 1.0, token_count);
    candidate.reasons = vec![SelectionReason::ExplicitId];
    candidate
}

/// A request whose budget leaves room for the brief frame plus
/// `item_budget_tokens` worth of items — what most packer tests want when
/// they are reasoning about how many ITEMS fit, not about the frame itself.
pub(super) fn request_with_item_budget(item_budget_tokens: usize) -> PackRequest {
    request_with_raw_budget(BRIEF_FRAME_TOKENS + item_budget_tokens)
}

/// A request whose `budget_tokens` is exactly what is passed in, with no
/// frame padding added — for tests that need to reach the boundary at or
/// below `BRIEF_FRAME_TOKENS` itself, which the padded helper above can
/// never express.
pub(super) fn request_with_raw_budget(budget_tokens: usize) -> PackRequest {
    PackRequest {
        query: "query".into(),
        scope: vec![Channel::Knowledge],
        budget_tokens,
        structural_freshness: Freshness::default(),
        semantic_freshness: Freshness::default(),
        dropped_terms: Vec::new(),
        surviving_terms: vec!["query".to_string()],
        required_representation: RequiredRepresentation::Full,
        degraded: None,
    }
}
