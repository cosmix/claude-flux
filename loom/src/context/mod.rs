//! Deterministic, model-free, network-free context retrieval over the curated
//! knowledge hierarchy.
//!
//! The subsystem answers one question — "which curated prose is worth spending
//! N tokens on for this query?" — and answers it the same way every time. There
//! is no embedding model, no network call, and no randomness anywhere below this
//! module; a [`schema::ContextPack`] is a pure function of the bytes on disk and
//! the query string.
//!
//! ## Pipeline
//!
//! ```text
//! knowledge/*.md ──chunker──> KnowledgeChunk ──catalog──> Catalog (revision)
//!        │                                                    │
//!    fingerprint ──> Freshness                             ingest
//!        │                                                    │
//!        └────────────> store (.loom/cache/context-v1/) <─────┘
//!                              │
//!                    rank ──> fuse ──> pack ──> ContextPack
//! ```
//!
//! [`rank`] scores each requested channel independently, [`fuse`] merges the
//! per-channel rank lists in TWO TIERS, and [`pack`] walks the fused list in
//! order taking whole chunks until the budget is spent. The packer never
//! exceeds its budget and always reports what it left out.
//!
//! Both channels contribute candidates. [`rank`] scores the knowledge chunks of
//! the catalog — curated prose and, behind a `prose:` id prefix, the project's
//! own design documents; [`rank_source::rank_source`] scores the nodes
//! [`source_graph`] extracted, read back through [`graph_store`] as a resolved
//! base-plus-overlay view.
//!
//! Fusion is not plain reciprocal rank: every candidate holding an exact rung
//! is ordered ahead of the lexical remainder by its raw ladder score, and only
//! that remainder is fused by RRF. Plain RRF reduced each candidate to a rank
//! position, so a chunk scoring 1080 on an explicit-id hit and a node scoring
//! 0.3 on a weak lexical match tied exactly and fell through to alphabetical
//! order by id. See [`fuse`] for the tier contract, and note the invariant
//! stated there: a tier-1 score and a tier-2 score live on unrelated scales and
//! are not comparable across the boundary — the ordering is the contract, the
//! magnitude is not.
//!
//! ## One entry point
//!
//! [`retrieve::retrieve_for_stage`] runs that whole pipeline and is the only way
//! in: the `loom knowledge context` command, signal generation and the prompt
//! hook all call it, so a brief rendered at spawn time and a brief pulled by
//! hand are built the same way. [`delivery`] then records what a recipient was
//! actually given, so the next retrieval in the same generation can skip it.
//!
//! ## Tunables
//!
//! Every threshold the pipeline depends on comes from a [`config::RetrievalConfig`],
//! loaded once by [`retrieve::retrieve_for_stage`] and threaded down by
//! reference. There is no global: the tunables belong to a retrieval, not to
//! the process.

pub mod config;
pub mod coverage;
pub mod delivery;
pub mod extract;
pub mod fingerprint;
pub mod freshness;
pub mod fuse;
pub mod graph_store;
pub mod ingest;
mod lexical;
pub mod lexical_index;
pub mod local_overlay;
pub mod pack;
pub mod rank;
pub mod rank_source;
pub mod refresh;
pub mod render;
pub mod resolve;
pub mod retrieve;
pub mod schema;
pub mod source_graph;
pub mod store;
pub(crate) mod untrusted;

pub use config::RetrievalConfig;
pub use coverage::CoverageReport;
pub use rank_source::rank_source;
pub use resolve::{impact, resolve_graph, ImpactHit, ResolutionStats, SymbolIndex};
pub use retrieve::{retrieve_for_stage, StageQuery};

pub use schema::{
    estimate_tokens, Channel, ChunkId, Confidence, ContextItem, ContextPack, Coverage,
    EdgeProvenance, FileCoverage, Freshness, ItemKind, KnowledgeChunk, LifecyclePolicy,
    LifecycleState, NodeLanguage, OmissionSummary, RequiredRepresentation, SelectionReason,
    SourceEdge, SourceEdgeKind, SourceNode, SourceNodeKind, SourcePointer, Span, UnmetRequirement,
    BRIEF_FRAME_TOKENS, BYTES_PER_TOKEN_ESTIMATE, EXCERPT_MAX_TOKENS, EXCERPT_TRUNCATION_MARKER,
};

#[cfg(test)]
mod tests;
