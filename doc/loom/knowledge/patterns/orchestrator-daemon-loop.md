---
---
# Orchestrator Daemon Loop

> Signal gen, daemon IPC, poll loop, heartbeat, session backend, spool drain.

## Signal Generation Pattern

Uses Manus KV-cache optimization with four sections:

1. **Stable prefix** (~1000 bytes): Worktree rules, execution rules, CLAUDE.md reminders. SHA-256 hashed. Rarely changes. Includes self-review checklist (standard) or detailed review dimensions (integration-verify).
2. **Semi-stable** (~1500-2500 bytes): Knowledge refs, memory/knowledge management, agent teams, sandbox, skill recommendations. Changes per stage type.
3. **Dynamic** (variable): Target metadata, plan overview, dependency status, handoff content, git history, files, tasks. Changes per session.
4. **Recitation** (end): Memory entries (last 10), task state, critical context. Placed last for maximum attention weight.

Four stage-type-specific prefix generators: standard, knowledge, integration-verify, knowledge-distill. Six signal types: Regular, Knowledge, Recovery, Merge, MergeConflict, BaseConflict. Signals are self-contained via `EmbeddedContext` struct.

KnowledgeDistill prefix: focuses on memory reading and knowledge curation; includes `loom memory show --all` and `loom knowledge update` guidance. The stage itself runs on **opus** (every `StageType` defaults to opus); it is the _spot-read subagents_ the prefix tells the main agent to delegate to that are sonnet.

**Data flow:** Stage Ready -> start_stage() -> create worktree -> Session.new() -> build_signal_context() -> format_signal_content() -> write_signal_file() -> spawn Claude Code.

## Daemon IPC Pattern

Unix socket at `.work/orchestrator.sock`, created mode 0o600 under a mode-0700 `.work/` directory.
Each request starts with a fixed authentication preface, so invalid credentials are rejected before
allocating the JSON body. Requests are capped at 64 KiB, responses at 2 MiB, and reads use an
absolute five-second deadline. Admission is bounded by 8 workers, a 16-request queue, a 512 KiB
global in-flight request budget, and 32 subscribers per stream. User capabilities cover Ping,
status/log subscriptions, Unsubscribe, DisputeCriteria, and the data-only `CompleteStage` request.
Completion is accepted only for the exact active stage/session identity and remains under the
sessions-directory lock through the stage transition; replay and cross-stage/session requests fail
without mutating state. Stop requires a one-time action-bound operator proof. A stable-file `flock`
is authoritative for daemon ownership and is held for the server lifetime. Graceful shutdown sets
the shutdown flag, joins bounded workers, and removes only the control files owned by that daemon.

## Polling Orchestration Pattern

Main loop polls every 5 seconds: sync graph from stage files, sync queued status, spawn merge resolution sessions, start ready stages, poll monitor for events, handle events. Exit when all stages complete or (failed + no sessions + no ready).

## Monitoring Patterns

**Heartbeat**: Sessions write to `.work/heartbeat/{stage-id}.json`; the JSON's `session_id` identifies the current owner. The three shell writers share an ownership-checked `mkdir` lock and atomic rename, so an old session cannot replace its successor and readers never see a torn document. Timeout: 300s. PID alive + stale = Hung; PID dead = Crashed; PID dead + stage Completed = normal exit. The heartbeat carries real resident tokens (`context_tokens`, `transcript_path`) written by `hooks/post-tool-use.sh` from the transcript tail — `context_percent` no longer exists. Judge sessions write a separate `heartbeat/<stage-id>.adjudication.json` instead of the stage file, keyed by the same stage id, read by `HeartbeatWatcher::judge_heartbeat`, and removed by `heartbeat::cleanup_judge_heartbeat` once the judge is closed. **Context health**: `orchestrator/monitor/context.rs::context_health(tokens, ceiling)` bands the ratio Green `<60%`, Yellow `60-90%`, Red `>=90%` of the resolved `context_ceiling_tokens` (absolute tokens, default 150,000; per-stage override in tokens, not a percentage) — there is no auto-summarize step. **Retry**: Exponential backoff `min(30 * 2^retry_count, 300s)`. Retryable: SessionCrash, Timeout. Non-retryable: ContextExhausted, TestFailure, BuildFailure, CodeError. Max 3 retries.

## Session Spawning and Liveness Pattern

The orchestrator holds one `Arc<SessionBackend>` and shares it with `LivenessService`. Spawn resolves
the native or tmux lane per call and records the chosen lane on `Session.backend`; kill and liveness
dispatch by that persisted lane. Use `LivenessService::is_alive(session)` rather than raw PID probes.
Process identity is PID plus kernel start time, and destructive signaling fails closed when identity
cannot be verified. For tests, `LivenessService::fixed_for_tests(bool)` avoids constructing a backend.

## Session Backend Dispatch Details

The orchestrator holds
`Arc<SessionBackend>` (`orchestrator/core/orchestrator.rs:91`, constructed at `:148` via
`SessionBackend::from_config`), and shares that same `Arc` with the `LivenessService`
(`orchestrator/liveness.rs:17,32`):

```rust
let backend = Arc::new(SessionBackend::from_config(config.work_dir.clone())?);
let liveness = LivenessService::new(Arc::clone(&backend));
```

`SessionBackend` dispatches each call to the `Native` or `Tmux` lane. Two rules follow:

- **Spawn** resolves the lane per call (config + tmux availability + fallback marker), then records the
  lane actually used on `Session.backend`.
- **Kill and liveness** dispatch on `session.backend` — the lane that _spawned_ it — never on the
  currently-configured backend, so sessions survive a config change or a daemon restart.

Every spawn site uses the shared handle; the other `SessionBackend::from_config` callers are
`orchestrator/continuation/mod.rs:89`, `commands/sessions.rs:130`, `commands/stage/state.rs:57`,
`commands/stage/merge_resolver.rs:72` and `commands/stage/skip_retry.rs:254`.

`LivenessService::fixed_for_tests(bool)` still returns a fixed value without constructing a backend.

→ [Terminal Backends](../architecture/terminal-backends.md)

## Spool-and-Drain: Writing Through a Sandbox You Cannot Widen

A stage agent's `.work` is a symlink into the main repo and the sandbox denies writes to
it, so `loom memory note` could not reach its own journal. Rather than widening the
sandbox, the write was made asynchronous:

- the agent appends to `<worktree_root>/.loom/memory-spool.jsonl` (`SPOOL_RELPATH`,
  `fs/memory/spool.rs:33`), size-capped at `SPOOL_MAX_BYTES` (1 MiB);
- `record()` (`commands/memory/handlers/record.rs:17`) falls into `record_via_spool`
  ONLY when the direct write failed AND `is_write_denied(&error)` matches
  `PermissionDenied`/EROFS — every other error still propagates unchanged;
- the daemon drains it: `Orchestrator::drain_stage_spools`
  (`orchestrator/core/spool_drain.rs:38`) every tick, plus a teardown drain
  `drain_spool_before_removal` (`git/cleanup/batch.rs:67`) so worktree-removal paths with
  no live orchestrator do not destroy pending entries.

Two design points to keep if you copy it. **The spool payload carries no stage id** — the
daemon attributes entries to the stage that owns the worktree it drained, so an agent
cannot forge another stage's journal (a real prompt-injection channel: a stage's journal
is quoted into that stage's later prompts). And **`drain_stage_spools` enumerates stages
by scanning `.work/stages/` on disk**, not from `active_worktrees`/`active_sessions`:
neither in-memory map survives a daemon restart, so disk is the only source of truth for
a stage recovered as still-Executing.

Known gap: with no daemon running at all, spooled entries stay pending until the next
tick or a teardown drain. `record_via_spool` says so in its own warning
(`record.rs:131`) rather than pretending the write landed.
