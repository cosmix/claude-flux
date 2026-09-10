# A2 — Bounded, task-seeded graph expansion in the source lane

Plan: `doc/plans/PLAN-knowledge-source-graph-contracts.md` · Stage: `retrieval-contracts` · Wave 1 (parallel with A1, A3; after A0).
Lane: codex `gpt-5.6-sol`, effort `xhigh`. Paths are repo-root relative; the crate is `loom/`.

## Why (F13, ranking half)

`rank_source_channel_cached` (`loom/src/context/rank_source.rs`) receives the whole `ResolvedGraph`, then immediately narrows to a node list and scores each node on its `scope` segments and `signature` only (`node_document`). `ResolvedGraph::edges()` and `FileEntry::edges` are never read anywhere in `rank_source.rs` or `rank_source/*.rs`. A query that names `retrieve_for_stage` therefore returns that one node and never the callers, callees, or implementations that a change to it must touch. The fix is a small, capped expansion from exact-rung seeds, added as tier-2 candidates so it can never crowd out an exact match.

A0 already added `SelectionReason::GraphNeighbor` (Display `graph-neighbor`), and made sure `fuse::has_exact_rung` and `Confidence::from_reasons` do NOT count it as exact. Codex's `loom map` answers from the published base and will not show A0's edits — read `schema.rs` and `fuse.rs` directly.

## Files you own (write)

- `loom/src/context/rank_source.rs` — call the expansion from the ranking entry point; keep the file under 400 lines by delegating
- `loom/src/context/rank_source/expand.rs` — NEW: the expansion
- `loom/src/context/rank_source/mod.rs` only if the submodule list lives there (check how `rank_source/candidacy.rs` and `rank_source/paths.rs` are declared and follow that exactly)
- Tests: `loom/src/context/tests/rank_source_expand.rs` — NEW; register it in `loom/src/context/tests/mod.rs` next to the other `rank_source*` test modules

Read-only: `rank.rs` (`RankedCandidate`, `RankQuery`, boost constants), `schema.rs`, `fuse.rs`, `graph_store/mod.rs` (`ResolvedGraph::{edges, node}`), `source_graph/edge.rs` (`SourceEdge`, `SourceEdgeKind`, `is_unresolved`), `source_graph/node.rs`, `rank_source/paths.rs` (`apply_test_path_factor`), `rank_source/candidacy.rs`, `context/tests/source_fixtures.rs` (`node`, `full_node`, `graph`, `graph_with_node`, `source_candidate` — reuse them; `graph(files)` builds every `FileEntry` with `edges: Vec::new()`, so build edges explicitly in your tests). Do not touch `pack.rs`, `retrieve.rs`, `channels.rs` (A1) or anything under `commands/` or `orchestrator/` (A3).

## Contract

In `rank_source/expand.rs`:

```rust
pub(super) const MAX_EXPANSION_SEEDS: usize = 5;
pub(super) const MAX_NEIGHBORS_PER_SEED: usize = 4;
pub(super) const MAX_EXPANDED: usize = 12;
pub(super) const NEIGHBOR_SCORE_FACTOR: f32 = 0.2;
pub(super) const MIN_NEIGHBOR_EDGE_CONFIDENCE: f32 = 0.5;

/// Add graph neighbours of the strongest exact-rung candidates as tier-2 candidates.
/// `ranked` is the channel's scored list BEFORE truncation to `MAX_SOURCE_CANDIDATES`,
/// sorted strongest first. Returns the list with neighbours appended (unsorted).
pub(super) fn expand_from_seeds(
    ranked: Vec<RankedCandidate>,
    graph: &ResolvedGraph,
    query: &RankQuery,
    config: &RetrievalConfig,
) -> Vec<RankedCandidate>
```

Rules:

- Seeds: the first `MAX_EXPANSION_SEEDS` candidates in `ranked` whose `reasons` contain at least one of `ExplicitId | ExactPath | ExactSymbol | StageDependency` (`LinkedFrom` is not awarded in the source lane). No seeds → return `ranked` unchanged.
- Neighbours of a seed node id `s`: every edge with `edge.from == s` (callee direction) or `edge.to == s` (caller direction) where `edge.kind ∈ {Calls, Implements, Extends, References}`, `!edge.is_unresolved()`, and `edge.confidence >= MIN_NEIGHBOR_EDGE_CONFIDENCE`. The neighbour id is the other endpoint; keep it only when `graph.node(id)` exists and its kind is not `SourceNodeKind::File`. Order neighbours by `(confidence desc, id asc)`; take at most `MAX_NEIGHBORS_PER_SEED` per seed and `MAX_EXPANDED` overall (seeds processed strongest first).
- Skip a neighbour already present in `ranked` (by id) or already added.
- Each added candidate: `id = neighbour id`, `channel = Channel::Source`, `score = seed.score * NEIGHBOR_SCORE_FACTOR`, then `apply_test_path_factor` exactly as the ranker applies it to ordinary candidates, `reasons = vec![SelectionReason::GraphNeighbor]`, `token_count = estimate_node_tokens(node)` (make that helper `pub(super)`), `matched_term_count = 0`, `confidence_ceiling = Some(Confidence::Medium)`.
- Deterministic: with identical input the output order and content are identical (sort keys named above; no `HashMap` iteration order anywhere).

Wire it in `rank_source.rs`: call `expand_from_seeds` inside the ranking entry point after scoring and before the final sort + truncation to `MAX_SOURCE_CANDIDATES` (so the cap still bounds the channel). Build one reverse and one forward adjacency `BTreeMap<&str, Vec<&SourceEdge>>` inside `expand_from_seeds` lazily — only when at least one seed exists — so a query with no exact rung pays nothing.

## Steps

1. Read `rank_source.rs` end to end once; find the function that produces the scored list handed to the ordering/truncation step (`rank_order` and its caller). Add the call there.
2. Write `expand.rs` per the contract; keep it under 200 lines.
3. Make `estimate_node_tokens` visible to the submodule.
4. Tests in `tests/rank_source_expand.rs` (build graphs with explicit edges through `source_fixtures::graph` plus manual `FileEntry.edges` assignment):
   - `an_exact_symbol_seed_pulls_in_its_resolved_callers_and_callees_as_graph_neighbours`
   - `contains_and_imports_edges_never_expand`
   - `unresolved_and_low_confidence_edges_never_expand`
   - `a_file_node_is_never_a_neighbour`
   - `a_neighbour_that_is_already_a_candidate_is_not_duplicated`
   - `expansion_is_capped_per_seed_and_overall` (build a hub with 6 callers; assert 4 kept, ordered by confidence then id; build 5 seeds × 4 → 12 total)
   - `no_seed_means_no_expansion_and_no_adjacency_work` (lexical-only candidates; output identical)
   - `a_neighbour_scores_below_its_seed_and_carries_only_the_graph_neighbour_reason`
   - `a_neighbour_in_a_test_path_is_downweighted_like_any_other_node`
   - `expansion_output_is_deterministic_across_two_runs`
   - One end-to-end assertion through `crate::context::fuse::fuse`: a `GraphNeighbor` candidate ends up in tier 2 below every exact-rung candidate (import `fuse` and assert on order).

## Done means

`cargo test --manifest-path loom/Cargo.toml --lib context::tests::rank_source` green (old and new); `cargo test --lib context::tests::fuse` unchanged and green; `rank_source.rs` ≤ 400 lines.

## Proof (ONE narrow check, run once)

`cargo test --manifest-path loom/Cargo.toml --lib context::tests::rank_source_expand` — skip if unsure.

## Constraints and traps

- Never award `GraphNeighbor` together with any other reason; never let it into tier 1 (`fuse::has_exact_rung` lists the exact rungs explicitly after A0 — do not edit `fuse.rs`).
- `withhold_partial_coverage` and the lexical candidacy floor apply to ranked candidates; neighbours bypass candidacy by design (they are graph evidence, not lexical evidence) but must still be nodes with `FileCoverage::Full` — skip neighbours whose node coverage is not `Full`, so a partially parsed file never reaches the pack through the back door.
- The existing `context/tests/rank_source*.rs` tests assert exact candidate lists for some graphs; expansion only fires with exact-rung seeds AND edges present, and the shared fixture `graph()` builds no edges, so they should stay green. If one fails, its graph has edges; add the assertion rather than weakening the test.
- No git commands. Do not create files beyond the two named.

## Report back

Files changed; the exact call site you chose in `rank_source.rs`; the final line counts of `rank_source.rs` and `expand.rs`; anything unresolved.
