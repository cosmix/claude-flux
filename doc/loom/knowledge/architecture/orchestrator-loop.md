---
---
# Orchestrator Loop

> Daemon main-loop tick order, Monitor subsystem, heartbeat liveness.

## Orchestrator Main-Loop Tick Sequence (Exact Call Order)

Main loop at `orchestrator/core/orchestrator.rs:258-376` — 5s poll cycle (100ms chunks for shutdown responsiveness):

```text
1. reconcile_and_update_graph()              [recovery.rs]       — catch phantom merges pre-sync
2. sync_graph_with_stage_files()             [recovery.rs]       — disk → in-memory graph
3. sync_queued_status_to_files()             [recovery.rs]       — graph Queued → disk
4. check_pending_disputes()                  [adjudicator]       — scan .work/disputes for new requests
5. apply_pending_verdicts()                  [adjudicator]       — apply ready verdicts, re-queue stages
6. drain_completed_adjudicator_workers()     [adjudicator]       — reap finished worker threads
7. spawn_merge_resolution_sessions()         [merge_handler.rs]  — detect/spawn merge resolvers
8. start_ready_stages()                      [stage_executor.rs] — worktrees + sessions for Queued
9. monitor.poll() → handle_events()          [event_handler.rs]  — completion/crash events
```

**Corrected 2026-07-30.** This section previously carried a plan-authoring note — `*** INSERT: check_pending_disputes() + apply_pending_verdicts() HERE ***` — proposing an insertion point _after_ merge resolution. The adjudicator hooks shipped and sit **before** merge resolution (steps 4-6), not after. The ordering property that matters is unchanged and still holds: verdicts are applied before `start_ready_stages()`, so a stage re-queued by a verdict is picked up in the same cycle.

The same three calls also run once during startup init, after `refresh_ready_status()` / `sync_queued_status_to_files()`. All three are idempotent and cheap no-ops when no disputes exist on disk.

The adjudicator is the codebase's first worker-thread + mpsc pattern; the rest of the loop remains polling-based. See patterns.md § Worker Thread + mpsc Pattern.

## Monitor Subsystem (orchestrator/monitor/)

Full file list:

- `core.rs` — `Monitor` struct, `poll()` API, stage/session loading
- `config.rs` — `MonitorConfig` (work_dir, hung_timeout, etc.)
- `detection.rs` — `Detection` struct: `detect_stage_changes()`, `detect_session_changes()`, `detect_heartbeat_events()`
- `events.rs` — `MonitorEvent` enum (stage/session/heartbeat event variants)
- `failure_tracking.rs` — Consecutive failure escalation logic
- `handlers.rs` — `Handlers` struct: handoff/crash-report generation; holds optional `LivenessService`
- `heartbeat.rs` — `HeartbeatWatcher` with 300s hung timeout
- `hung_latch.rs` — when a silent session's report becomes escalation evidence rather than a warning (see [Signal Generation Pipeline](signal-generation.md) § Soft Signals for the detection pipeline this drives)
- `context.rs` — `context_health(tokens, ceiling)` bands an absolute token count as a fraction of its resolved ceiling: Green `<60%`, Yellow `60-90%`, Red `>=90%` (see [Context Ceiling](context-ceiling.md))
- `tests.rs` — Unit tests

**`Monitor::poll()` flow:**

1. Load all stages from `.work/stages/*.md`
2. Load all sessions from `.work/sessions/*.md`
3. `detection.detect_stage_changes()` — file-level changes
4. `detection.detect_session_changes()` — PID liveness, status transitions
5. `detection.detect_heartbeat_events()` — hung detection via `HeartbeatWatcher`
6. Return `Vec<MonitorEvent>`

**LivenessService injection:** `Monitor::set_liveness(liveness: LivenessService)` is called by the
orchestrator after the shared `SessionBackend` is constructed. Backend liveness uses the lane recorded
on `Session.backend` and verified process identity; missing identity is unverifiable, not permission to
fall back to a raw PID signal.
