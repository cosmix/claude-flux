//! Tests for [`super::union_dropped_terms`] and its wiring into
//! [`super::rank_channels`].
//!
//! Fixtures are hand-built here rather than pulled from a shared fixtures
//! module: `union_dropped_terms` and `rank_channels` are private/`pub(super)`
//! to `channels`, so this file only needs `use super::*` plus the schema
//! types for one small `KnowledgeChunk` builder.

use super::*;
use crate::context::schema::{
    Channel, ChunkId, KnowledgeChunk, LifecyclePolicy, LifecycleState, SelectionReason,
};
use std::path::PathBuf;

/// Build a `KnowledgeChunk` with every field explicit but `body`, mirroring
/// `context/tests/rank_fixtures.rs::chunk`'s shape.
fn chunk(id: &str, body: &str) -> KnowledgeChunk {
    KnowledgeChunk {
        id: id.to_string(),
        file: PathBuf::from(format!("{id}.md")),
        anchor: String::new(),
        heading: String::new(),
        body: body.to_string(),
        content_hash: String::new(),
        estimated_tokens: 1,
        aliases: Vec::new(),
        category: None,
        source_paths: Vec::new(),
        symbols: Vec::new(),
        links: Vec::new(),
        state: LifecycleState::Active,
    }
}

#[test]
fn both_channels_dropped_terms_are_reported() {
    // "beta" overlaps both sets; "alpha" is knowledge-only, "gamma" is
    // source-only. The expected vector proves both first-seen order
    // (knowledge before source) and dedup (one "beta", not two).
    let knowledge = Some(vec!["alpha".to_string(), "beta".to_string()]);
    let source = Some(vec!["beta".to_string(), "gamma".to_string()]);

    let got = union_dropped_terms(knowledge, source);

    assert_eq!(
        got,
        vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()]
    );
}

#[test]
fn one_channel_alone_reports_its_own_terms() {
    let knowledge_only = union_dropped_terms(Some(vec!["alpha".to_string()]), None);
    assert_eq!(knowledge_only, vec!["alpha".to_string()]);

    let source_only = union_dropped_terms(None, Some(vec!["beta".to_string()]));
    assert_eq!(source_only, vec!["beta".to_string()]);
}

#[test]
fn neither_channel_yields_an_empty_vec() {
    assert!(union_dropped_terms(None, None).is_empty());
}

#[test]
fn knowledge_channel_dropped_terms_surface_through_rank_channels() {
    // "hi" is shorter than `min_query_token_len` (3) and not backticked, so
    // `partition_terms` (`rank/corpus.rs:157-182`) drops it regardless of
    // corpus size; "cache" survives because it appears in the one chunk below
    // and its document frequency (1) sits well under the ubiquity floor
    // (`max(corpus_size * stop_df_ratio, df_ident_max)` = 5 here).
    let catalog = Catalog {
        revision: "test".to_string(),
        chunks: vec![chunk("a.md#topic#0", "cache is nice")],
        issues: Vec::new(),
    };
    let rank_query = RankQuery {
        text: "hi cache".to_string(),
        ..RankQuery::default()
    };
    let config = RetrievalConfig::default();

    let got = rank_channels(&[Channel::Knowledge], &rank_query, &catalog, None, &config);

    assert_eq!(got.dropped_terms, vec!["hi".to_string()]);
    assert_eq!(got.surviving_terms, vec!["cache".to_string()]);
    assert_eq!(
        got.lists.len(),
        1,
        "one requested channel, one candidate list"
    );
    assert_eq!(
        got.lists[0].len(),
        1,
        "the chunk matched 'cache' and should be a candidate"
    );
}

#[test]
fn apply_lifecycle_policy_keeps_explicitly_required_ineligible_chunks() {
    let mut superseded = chunk("old", "rare wording");
    superseded.state = LifecycleState::Superseded;
    let chunks = BTreeMap::from([("old", &superseded)]);
    let candidate = RankedCandidate {
        id: ChunkId::from("old"),
        channel: Channel::Knowledge,
        score: 1.0,
        reasons: vec![SelectionReason::ExplicitId],
        token_count: 3,
        matched_term_count: 0,
        confidence_ceiling: None,
    };

    let kept = apply_lifecycle_policy(vec![candidate.clone()], &chunks, LifecyclePolicy::Current);
    assert_eq!(kept, vec![candidate]);
}
