---
---
# Context And Source Graph

> Context retrieval pipeline and the source-graph channel/lifecycle

## Context Retrieval Subsystem (2026-08-17)

Read `loom/src/context/mod.rs` FIRST — its docstring carries an accurate pipeline diagram
and states plainly which channel is wired. Then the file for what you touch:

| Path                                                    | What it owns                                                                                                             |
| ---------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `context/mod.rs`                                        | pipeline overview, public re-exports, the one-entry-point rule                                                           |
| `context/retrieve.rs`                                   | `retrieve_for_stage`, `StageQuery`, `context_epoch` — the ONLY way into the pipeline                                     |
| `context/schema.rs`                                     | `ContextPack`, `ContextItem`, `Channel`, `Freshness`, token constants; re-exports source-graph names                     |
| `context/ingest.rs`, `rank.rs`, `fuse.rs`, `pack.rs`    | chunk ingest, per-channel scoring, two-tier fusion (exact rungs, then reciprocal-rank fusion), budget-bounded packing    |
| `context/store.rs`                                      | derived-artifact store under `.loom/cache/context-v1/`; **`open` follows the `.work` symlink to the MAIN project root**  |
| `context/delivery.rs`, `delivery/session.rs`             | delivery records, `plan_key`/`plan_key_from`, epoch-scoped suppression; `session.rs` adds the prompt hook's per-session dedupe (`hook_recipient_id`, `delivered_to_session`, `discard_session_delivery`, A.16/A.21) |
| `context/untrusted.rs`                                  | `inline_safe` — the ONE flattener for untrusted values, now on three surfaces (two agent-facing, one operator-facing) — see patterns.md |
| `context/freshness.rs`, `fingerprint.rs`, `coverage.rs` | staleness tracking, content fingerprints, `CoverageReport`                                                               |
| `context/refresh/source_graph.rs`                       | `reconcile_source_graph`, `SourceGraphScope`, `SourceGraphOutcome`                                                       |
| `context/graph_store/`                                  | base/overlay layering, `GraphLayer`, canonical serialization                                                             |
| `context/source_graph/`                                 | `SourceNode`, `SourceEdge`, `EdgeProvenance`, confidence ceilings, `node_id`                                             |
| `context/extract/`                                      | `SourceGraphExtractor` trait, `registry()`, `ExtractorIdentity`, `context/extract/treesitter/` shared harness (directory: `mod.rs`, `build.rs`, `collect.rs`), one module per language |
| `context/resolve/`                                      | cross-file symbol resolution, `impact`, `SymbolIndex`                                                                    |
| `telemetry/mod.rs`                                      | `TelemetryEvent`, `emit`, `read_events` over `.work/telemetry/events.jsonl`                                              |
| `orchestrator/signals/format/brief.rs`                  | renders the Knowledge Brief into a stage signal                                                                          |
| `orchestrator/core/stage_telemetry.rs`                  | the only telemetry writer, called from `stage_executor.rs:570`                                                           |
| `orchestrator/merge_lifecycle.rs`                       | merge/verify/cleanup ordering; the single door to post-merge cleanup                                                     |

## Source Graph as a Retrieval Channel, and Its Lifecycle (2026-08-18)

| Surface | File | Notes |
| --- | --- | --- |
| `context::rank_source` | `context/rank_source.rs:154` | ranks source-graph nodes for `Channel::Source`; re-exported at `context/mod.rs:74` |
| shared BM25 core | `context/rank/corpus.rs` | `prepare_lexical` / `prepare_lexical_cached` / `score_bm25`, re-exported `pub(crate)` from `rank.rs`, used by both rankers and by the persistent index (`context/lexical_index.rs`, A.13) |
| `context/local_overlay.rs` | whole file | the ONE definition of the working-tree overlay address: `LOCAL_PLAN_KEY`, `local_overlay_stage_name`, `local_overlay_key`, `OverlayScope` |
| `advisory_source_graph_preflight` | `commands/run/checks.rs:103-111` | never returns `Result`; called from `init/execute.rs:187`, `run/mod.rs:101`, `run/foreground.rs:39` |
| `SemanticLayer` / `SemanticOutcome` | `context/refresh/semantic.rs:50-64` | what `loom knowledge sync` reports: `base` \| `local-overlay` \| `skipped` |
| `stage_overlay_scope` | `orchestrator/signals/retrieval.rs:~110` | gives a stage brief its own overlay; plan component MUST equal `delivery::plan_key(stage)` |
| `loom stage amend` | `commands/stage/amend.rs` | operator repair of an impossible criterion; thin wrapper over the pre-existing `apply_amendment` (atomic, flock, snapshot + audit row) |
| `criterion_needs_ungrantable_resource` | `plan/schema/validation.rs:647` | plan-time warning when a criterion needs `loom map`, `loom knowledge context`, `tmux` or `docker` — resources a worktree sandbox cannot grant |
| memory spool | `fs/memory/spool.rs:33,59,191` + `orchestrator/core/spool_drain.rs:38` + `git/cleanup/batch.rs:67` | see the spool-and-drain pattern |

`loom map` is now three read-only flags and nothing else: `--outline <PATH>`,
`--find-all <SYMBOL>`, `--impact <SYMBOL_OR_PATH>` (`commands/map.rs:17-28`). `--deep`
and `--focus` are gone, along with `map/{analyzer,detectors,knowledge_sync}.rs`. Note
that the GLOBAL agent doctrine file still documents `loom map [--deep] [--focus <area>]`
— that text is stale against this repo.
