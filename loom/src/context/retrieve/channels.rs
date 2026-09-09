//! Per-channel ranking dispatch for [`super::retrieve_for_stage`].
//!
//! One place decides which ranker a [`Channel`] goes to, what happens when the
//! corpus behind a channel is missing, and which channel's corpus diagnostics
//! the pack ends up reporting. Split out of `retrieve.rs` so that entry point
//! stays a readable sequence of steps rather than a file with a ranker
//! dispatcher wedged into the middle of it.

use crate::context::config::RetrievalConfig;
use crate::context::graph_store::ResolvedGraph;
use crate::context::lexical::tokenize;
use crate::context::lexical_index::LexicalCache;
use crate::context::rank::{rank_channel_cached, ChannelRanking, RankQuery, RankedCandidate};
use crate::context::rank_source::rank_source_channel_cached;
use crate::context::schema::{
    Channel, KnowledgeChunk, LifecyclePolicy, LifecycleState, SelectionReason,
};
use crate::fs::knowledge::catalog::Catalog;
use std::collections::BTreeMap;
use std::path::Path;

use super::StageQuery;

/// Every requested channel's candidate list, plus the one dropped-term set the
/// pack reports.
pub(super) struct RankedChannels {
    /// One candidate list per requested channel, in `scope` order.
    pub(super) lists: Vec<Vec<RankedCandidate>>,
    /// Query terms dropped before scoring, for `ContextPack::dropped_terms`.
    pub(super) dropped_terms: Vec<String>,
    /// Query terms retained by at least one requested channel's corpus.
    pub(super) surviving_terms: Vec<String>,
}

/// Rank the catalog once per requested channel, producing one candidate list
/// per channel.
///
/// Each channel is ranked over its own corpus: the knowledge channel over the
/// catalog's chunks via [`crate::context::rank::rank_channel`], the source
/// channel over `graph`'s nodes via
/// [`crate::context::rank_source::rank_source_channel`]. `graph` is `None` when
/// the source graph was never built or could not be read for this query — that
/// is a degraded pack, not an error, so the source channel simply contributes
/// no candidates rather than failing the whole retrieval.
///
/// The pack reports the UNION of both channels' dropped-term sets, not just
/// one: both channels tokenize the same query text, but since A.2 they
/// stopword it against DIFFERENT corpora with different ubiquity floors
/// (`rank/corpus.rs:194`), so the two sets genuinely differ. Reporting only
/// one channel's set used to read as complete while silently discarding the
/// other half of the answer — see [`union_dropped_terms`] for why that is the
/// worse failure for a field whose entire job is telling a reader what the
/// ranker ignored.
///
/// Ranks over freshly tokenized corpora, with no persistent lexical index
/// behind either channel. [`super::retrieve_for_stage`] always has a context
/// cache root and so always goes through [`rank_channels_cached`], which leaves
/// `channels/tests.rs` as this form's only caller — hence the `cfg`. It earns
/// its keep there: it is what lets the dispatcher's dropped-term wiring be
/// pinned without a cache directory, over the scan that is the index's own
/// correctness oracle (`lexical_index.rs:20-29`).
#[cfg(test)]
pub(super) fn rank_channels(
    channels: &[Channel],
    rank_query: &RankQuery,
    catalog: &Catalog,
    graph: Option<&ResolvedGraph>,
    config: &RetrievalConfig,
) -> RankedChannels {
    let chunks_by_id = catalog
        .chunks
        .iter()
        .map(|chunk| (chunk.id.as_str(), chunk))
        .collect();
    rank_channels_cached(
        channels,
        rank_query,
        catalog,
        &chunks_by_id,
        graph,
        LifecyclePolicy::Current,
        config,
        None,
    )
}

/// Rank the catalog once per requested channel, with the persistent lexical
/// index (A.13) under both channels when `cache_root` names a context cache.
///
/// The dispatch, the degraded-`graph` handling and the dropped-term union are
/// all as `rank_channels` above documents them; this is that pass with a cache
/// threaded through it, and the form [`super::retrieve_for_stage`] calls.
///
/// One root in, two caches out: the channels have separate corpora with
/// separate lifetimes, so [`LexicalCache::knowledge`] is keyed by the catalog
/// revision and [`LexicalCache::source`] by the resolved source layer. Each is
/// constructed INSIDE its own match arm rather than up front, because both keys
/// cost real work — `source_layer_key` hashes every resolved file — and a query
/// that narrows `scope` to one channel must not pay for the other's.
///
/// `None` keeps the full corpus scan, which is what every existing caller and
/// test gets: the scan is the oracle the index is checked against
/// (`lexical_index.rs:20-29`), so it has to stay on a live path rather than
/// becoming code that only runs once a cache file is deleted.
// Every parameter is a distinct input `rank_channels` above threads through
// unchanged; a params struct would just wrap that pass-through, not shrink it.
#[allow(clippy::too_many_arguments)]
pub(super) fn rank_channels_cached(
    channels: &[Channel],
    rank_query: &RankQuery,
    catalog: &Catalog,
    chunks_by_id: &BTreeMap<&str, &KnowledgeChunk>,
    graph: Option<&ResolvedGraph>,
    lifecycle: LifecyclePolicy,
    config: &RetrievalConfig,
    cache_root: Option<&Path>,
) -> RankedChannels {
    let mut lists = Vec::with_capacity(channels.len());
    let mut knowledge_dropped = None;
    let mut source_dropped = None;
    let mut knowledge_surviving = None;
    let mut source_surviving = None;
    for channel in channels {
        let mut ranking =
            rank_one_channel(*channel, rank_query, catalog, graph, config, cache_root);
        if *channel == Channel::Knowledge {
            let eligible = apply_lifecycle_policy(ranking.candidates, chunks_by_id, lifecycle);
            ranking.candidates = eligible;
        }
        let surviving = if channel_has_corpus(*channel, catalog, graph) {
            surviving_query_terms(&rank_query.text, &ranking.dropped_terms)
        } else {
            Vec::new()
        };
        let (dropped_slot, surviving_slot) = match channel {
            Channel::Knowledge => (&mut knowledge_dropped, &mut knowledge_surviving),
            Channel::Source => (&mut source_dropped, &mut source_surviving),
        };
        *dropped_slot = Some(ranking.dropped_terms);
        *surviving_slot = Some(surviving);
        lists.push(ranking.candidates);
    }
    RankedChannels {
        lists,
        dropped_terms: union_dropped_terms(knowledge_dropped, source_dropped),
        surviving_terms: union_terms(knowledge_surviving, source_surviving),
    }
}

fn rank_one_channel(
    channel: Channel,
    rank_query: &RankQuery,
    catalog: &Catalog,
    graph: Option<&ResolvedGraph>,
    config: &RetrievalConfig,
    cache_root: Option<&Path>,
) -> ChannelRanking {
    match channel {
        Channel::Knowledge => {
            let cache =
                cache_root.map(|root| LexicalCache::knowledge(root, catalog.revision.as_str()));
            rank_channel_cached(
                rank_query,
                &catalog.chunks,
                Channel::Knowledge,
                config,
                cache.as_ref(),
            )
        }
        Channel::Source => graph
            .map(|graph| {
                let cache = cache_root.map(|root| LexicalCache::source(root, graph));
                rank_source_channel_cached(rank_query, graph, config, cache.as_ref())
            })
            .unwrap_or_default(),
    }
}

fn channel_has_corpus(channel: Channel, catalog: &Catalog, graph: Option<&ResolvedGraph>) -> bool {
    match channel {
        Channel::Knowledge => !catalog.chunks.is_empty(),
        Channel::Source => graph.is_some_and(|graph| graph.nodes().next().is_some()),
    }
}

/// Admit current knowledge unless the caller explicitly asks for history.
pub(super) fn apply_lifecycle_policy(
    candidates: Vec<RankedCandidate>,
    chunks_by_id: &BTreeMap<&str, &KnowledgeChunk>,
    policy: LifecyclePolicy,
) -> Vec<RankedCandidate> {
    candidates
        .into_iter()
        .filter(|candidate| {
            candidate.reasons.contains(&SelectionReason::ExplicitId)
                || policy == LifecyclePolicy::Historical
                || chunks_by_id.get(candidate.id.as_str()).is_none_or(|chunk| {
                    matches!(chunk.state, LifecycleState::Active | LifecycleState::Draft)
                })
        })
        .collect()
}

fn surviving_query_terms(query: &str, dropped: &[String]) -> Vec<String> {
    tokenize(query)
        .into_iter()
        .filter(|term| !dropped.contains(term))
        .collect()
}

fn union_terms(first: Option<Vec<String>>, second: Option<Vec<String>>) -> Vec<String> {
    let mut union = Vec::new();
    for term in first
        .into_iter()
        .flatten()
        .chain(second.into_iter().flatten())
    {
        if !union.contains(&term) {
            union.push(term);
        }
    }
    union
}

/// Both channels' dropped-term sets, deduplicated, in first-seen order with
/// the knowledge channel's first.
///
/// A union, not a preference. The two channels tokenize the same query text but
/// stopword it against DIFFERENT corpora with different ubiquity floors
/// (`rank/corpus.rs:194`), so since A.2 the sets genuinely differ: reporting
/// only one silently drops half the answer while still reading as complete,
/// which is the worse failure for a field whose entire job is telling a reader
/// what the ranker ignored.
///
/// The order is knowledge-then-source regardless of `scope` order, so two runs
/// over identical bytes agree completely even if a caller reorders its
/// channels - determinism is a hard requirement of this pipeline
/// (`retrieve.rs:9-11`).
fn union_dropped_terms(knowledge: Option<Vec<String>>, source: Option<Vec<String>>) -> Vec<String> {
    union_terms(knowledge, source)
}

/// Build the ranker's view of `query`.
///
/// A [`StageQuery`] additionally names where to look; a [`RankQuery`] is only
/// what to look for, which is why the two types exist separately.
pub(super) fn build_rank_query(query: &StageQuery) -> RankQuery {
    RankQuery {
        text: query.text.clone(),
        required_ids: query.required_ids.clone(),
        stage_dependency_ids: query.stage_dependency_ids.clone(),
        dependency_paths: query.dependency_paths.clone(),
    }
}

#[cfg(test)]
#[path = "channels/tests.rs"]
mod tests;
