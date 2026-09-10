# Architecture

> High-level component relationships, data flow, and module dependencies.
>
> **Related files:** [patterns.md](patterns.md) for design patterns, [entry-points.md](entry-points.md) for code navigation, [conventions.md](conventions.md) for coding standards.

## Project Overview

Loom is a Rust CLI (~15K lines) for orchestrating parallel Claude Code sessions across git worktrees. It enables concurrent task execution with automatic crash recovery, context handoffs, and progressive merging.

## Directory Structure

Full `loom/src/` module tree, `.work/` state layout, repo-root asset directories.

→ [Directory Structure](architecture/directory-structure.md)

## Core Abstractions

`ExecutionGraph`, `Stage`, `Session`, `Orchestrator`, `TerminalBackend`, `KnowledgeDir`, data flow, `.work/` file ownership.

→ [Core Abstractions, Data Flow & File Ownership](architecture/core-abstractions.md)

## Worktree Isolation (4-Layer Defense)

→ [Worktree Isolation & Security](architecture/security-and-isolation.md)

## Layering Violations (Known Issues)

→ [Core Abstractions § Layering Violations](architecture/core-abstractions.md)

## Context Budget Enforcement

`context_ceiling_tokens` — an ABSOLUTE resident-token ceiling, resolved stage -> config -> default, enforced at three independent thresholds (1.0x hook, 1.25x daemon, 1.5x native compaction).

→ [Context Ceiling](architecture/context-ceiling.md)

## Security Model

→ [Worktree Isolation & Security](architecture/security-and-isolation.md)

## Merge Lock (git/merge/lock.rs)

→ [Merge Flow § Merge Lock](architecture/merge-flow.md)

## Merge Flow (post-completion auto-merge) [DETAILED]

→ [Merge Flow](architecture/merge-flow.md)

## Skills Module (loom/src/skills/)

→ [Skill Catalog](architecture/skill-catalog.md)

## Diagnosis Module (loom/src/diagnosis/)

→ [Skill Catalog § Component Architecture](architecture/skill-catalog.md)

## Map Module (loom/src/map/)

→ [Skill Catalog § Component Architecture](architecture/skill-catalog.md)

## Handoff System

→ [Context Ceiling § Handoff System](architecture/context-ceiling.md)

## macOS Terminal Detection Priority

→ [Terminal Backends](architecture/terminal-backends.md)

## find_claude_path() (src/claude.rs)

→ [Terminal Backends](architecture/terminal-backends.md)

## KnowledgeDir API (fs/knowledge/dir.rs)

KnowledgeFile enum: Architecture, EntryPoints, Patterns, Conventions, Mistakes, Stack, Concerns. Core methods: new(root), exists(), initialize(), read(file), read_all(), append(file, content), generate_summary(), list_files().

## Adding New Plan Fields Checklist

The 10-step checklist for adding a new stage/plan field end-to-end.

→ [Plan Lifecycle & Fields](architecture/plan-lifecycle-and-fields.md)

## Goal-Backward Verification (verify/goal_backward/)

Four verification layers; **`truths` is NOT one of them** — merged into acceptance.

→ [Plan Lifecycle & Fields § Goal-Backward Verification](architecture/plan-lifecycle-and-fields.md)

## Per-Worktree Gitignore for settings.local.json

→ [Worktree Isolation & Security § Per-Worktree Gitignore](architecture/security-and-isolation.md)

## Claude Code Worktree Isolation Disabled in Generated Settings

→ [Worktree Isolation & Security § Claude Code Worktree Isolation Disabled](architecture/security-and-isolation.md)

## Hook System Architecture (hooks/)

The `hooks/` scripts, how they are embedded and installed, the SessionStart
`hookSpecificOutput` contract, and the enforcement layers that keep subagents inside their
lane (`commit-filter.sh`, `subagent-verify-guard.sh`, the worktree guards).

→ [Hook System](architecture/hook-system.md)

## Monitor Subsystem (orchestrator/monitor/)

→ [Orchestrator Loop & Monitor](architecture/orchestrator-loop.md)

## Status Data Model [DETAILED]

Where every `loom status` field comes from, the 13 `StageStatus` variants, and what is not yet surfaced.

→ [Status Data Model](architecture/status-data-model.md)

## Status Command Architecture (commands/status/)

→ [Status Data Model § Status Command Module Layout](architecture/status-data-model.md)

## Orchestrator Main-Loop Tick Sequence (Exact Call Order)

→ [Orchestrator Loop & Monitor](architecture/orchestrator-loop.md)

## Stage State Machine — 13 Variants

→ [Core Abstractions § Stage State Machine](architecture/core-abstractions.md)

## Adjudication Persistence and Stage Resumption [DETAILED]

Durable artifact chain under `.loom/work/disputes/`, materialization into stage state, and what a fresh successor session receives.

→ [Adjudication Persistence and Stage Resumption](architecture/adjudication-lifecycle.md)

## Plan Versioning / Runtime Amendment (Shipped — `plan/amendment.rs`)

→ [Plan Lifecycle & Fields § Plan Versioning / Runtime Amendment](architecture/plan-lifecycle-and-fields.md)

## Plan Immutability Invariant (Narrowed, Not Removed)

→ [Plan Lifecycle & Fields § Plan Immutability Invariant](architecture/plan-lifecycle-and-fields.md)

## Remote Control Module (loom/src/remote_control.rs)

→ [Remote Control Module](architecture/remote-control.md)

## Signal Generation Pipeline (orchestrator/signals/) [DETAILED]

→ [Signal Generation Pipeline](architecture/signal-generation.md)

## before_stage / after_stage / code_review Schema Fields — Execution Status

→ [Plan Lifecycle & Fields § before_stage / after_stage / code_review](architecture/plan-lifecycle-and-fields.md)

## load_stage_definition_from_plan — Centralized Plan Lookup

→ [Plan Lifecycle & Fields § load_stage_definition_from_plan](architecture/plan-lifecycle-and-fields.md)

## `loom pressure` Command (Plan Pressure-Testing Driver)

→ [Remote Control Module § loom pressure Command](architecture/remote-control.md)

## Tiered Knowledge Base (`fs/knowledge/`, `commands/knowledge/`)

Two-tier curated knowledge: generated `INDEX.md`, tier-1 summary files, and tier-2 topics at
`<category>/<slug>.md`. Layout is `Hierarchical` **iff** `INDEX.md` exists. Covers module split,
target parsing, index generation, the `catalog::build` diagnostics (duplicate heading, generic
blurb, broken link, missing source ref), opt-in migration, and lock ordering.

→ [Knowledge Hierarchy](architecture/knowledge-hierarchy.md)

## Codex Plugin (openai-codex) [DETAILED]

Marketplace install/scope, the `loom-codex-forwarder` lane, per-stage `implementers`, sonnet fallback.

→ [Codex Plugin](architecture/codex-plugin.md)

## Codex Concurrency [DETAILED]

Foreground fan-out verified safe to 6 concurrent codex-companion tasks; background fan-out ruled out.

→ [Codex Concurrency](architecture/codex-concurrency.md)

## Terminal Backends: SessionBackend / TmuxBackend [DETAILED]

`SessionBackend` dispatches every spawn/kill/liveness call to a `Native` or opt-in `Tmux` lane, recorded on `Session.backend`.

→ [Terminal Backends](architecture/terminal-backends.md)

## Context Retrieval (`loom/src/context/`)

Deterministic, model-free, network-free retrieval over the curated knowledge
hierarchy: chunk the prose (curated plus indexed project prose under `doc/`),
rank per channel, fuse by **two-tier fusion** (exact-rung candidates first by
raw score, the lexical remainder by reciprocal-rank fusion — NOT plain RRF),
pack to a token budget. One entry point — `context::retrieve_for_stage` —
serves the `loom knowledge context`/`loom knowledge eval` commands, signal
generation and the prompt hook alike. Two graphs exist — the knowledge-chunk
catalog and the tree-sitter source graph — and both are ranked and fused into
one pack, each through a persistent per-revision BM25 index behind the full
scan (the scan stays the correctness oracle).

Full detail, including the base/overlay layering rule and what is derived versus
durable: [architecture/context-retrieval.md](architecture/context-retrieval.md).

## Source Graph (`loom/src/context/source_graph/`, `context/extract/`)

A derived tree-sitter graph of the repo's own source, with two live consumers:
`loom map` (via `context::graph_store`) and the `Source` retrieval channel,
ranked by `context::rank_source` (`context/rank_source.rs:154`) and fused into the
same `ContextPack` as knowledge chunks. Its defining property is an explicit
honesty contract: every edge carries provenance and a confidence ceiling, and no
file is ever silently omitted — a degraded file is reported as degraded.

Extractor trait, cache identity, coverage contract, the ranker and the
publish/reconcile lifecycle: [architecture/source-graph.md](architecture/source-graph.md).

## Execution Containment (`loom/src/verify/criteria/confine.rs`)

Plan-authored commands run through `spawn_confined` — environment scrubbing, not isolation.

→ [Execution Containment](architecture/execution-containment.md)

## Memory Spool and Drain (`fs/memory/spool.rs`, `orchestrator/core/spool_drain.rs`) [DETAILED]

A sandboxed stage cannot write `.work/memory/<stage>.md` — `.work` is a symlink out of the
worktree and the sandbox grants no `Edit` there — so `loom memory` appends to
`<worktree>/.loom/memory-spool.jsonl` instead and the daemon drains it each tick, plus once
more in `cleanup_after_merge` before the worktree is destroyed. The payload carries **no
stage id**: attribution comes from which worktree an entry was drained from, which an agent
cannot forge.

Why the allowlist could not simply be widened, the drain invariants that are easy to break,
and the read-path merge: [architecture/memory-spool.md](architecture/memory-spool.md).

## Telemetry (`loom/src/telemetry/`)

→ [Signal Generation Pipeline § Telemetry](architecture/signal-generation.md)

## Quota Poller

A daemon thread (`loom/src/quota/poller.rs`) polls the Claude OAuth usage endpoint and `codex app-server` every 180 s and caches one `ProviderQuota` per provider under `.loom/work/quota/`; `StatusData.quota` is read from that cache, never polled, and rendered in the `--live` and `--web` footers. Sources, backoff, cache hygiene, and the token-handling rules: [architecture/quota-poller.md](architecture/quota-poller.md).

## Web Dashboard

`loom status --web [PORT]` serves an embedded React SPA over HTTP/WebSocket, streaming the same `StatusData` the live TUI renders.

→ [Web Dashboard](architecture/web-dashboard.md)
