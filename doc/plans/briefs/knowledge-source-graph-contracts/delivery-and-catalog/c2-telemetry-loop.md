# C2 — A telemetry loop with a reader: prompt briefs, abstentions, pulls, spooled from sandboxed sessions, summarised by `loom knowledge telemetry`

Plan: `doc/plans/PLAN-knowledge-source-graph-contracts.md` · Stage: `delivery-and-catalog` · Single wave (parallel with C1, C3, D1, D2, D3).
Lane: codex `gpt-5.6-sol`, effort `xhigh`. Paths are repo-root relative; the crate is `loom/`.

## Why (F16, telemetry half)

`loom/src/telemetry/mod.rs` writes one `ContextDelivered { stage_id, session_id, context_epoch, items }` or `ContextUnavailable` line per spawned session to `.loom/work/telemetry/events.jsonl` (writer: `orchestrator/core/stage_telemetry.rs`, called from `stage_executor.rs`). `read_events` has no production caller — its own doc comment says so. Nothing records whether a prompt brief was emitted or withheld, or what agents pulled with `loom knowledge context`. A session inside a stage sandbox cannot append to `.loom/work/telemetry/` at all (the symlinked state root is write-denied), so even a new writer there would silently lose events.

Merged before you: `memory-events` (the spool pattern in `fs/memory/spool.rs` and its drain points; the archive in `fs/memory/archive.rs` already copies `<work>/telemetry/`). C1 (parallel) emits the new events with the exact variant names below; D3 (parallel) adds the `Telemetry` clap variant and dispatch arm that call your `commands::knowledge::telemetry::telemetry(stage, json)`.

## Files you own (write)

- `loom/src/telemetry/mod.rs` (split into `telemetry/{spool.rs, summary.rs}` as needed; each file under 400 lines)
- `loom/src/commands/knowledge/telemetry.rs` — NEW (D3 declares `pub mod telemetry;` in `commands/knowledge/mod.rs`; do not edit `mod.rs`)
- `loom/src/commands/knowledge/context.rs` — emit `ContextPulled` after a successful retrieval (one call; nothing else)
- `loom/src/orchestrator/core/spool_drain.rs`, `loom/src/git/cleanup/batch.rs`, `loom/src/git/cleanup/worktree.rs` — drain and delete the telemetry spool exactly where the memory spool is drained and deleted
- `loom/src/git/worktree/settings.rs` — the sandbox write grant for the telemetry spool, next to `MEMORY_SPOOL_RELPATH`
- `.gitignore` — `.loom/telemetry-spool.jsonl`
- `loom/src/context/delivery.rs` — no change expected; owned so a helper can be added if `summary.rs` needs one

Read-only: `commands/hook/**` (C1), `cli/**` and `commands/knowledge/mod.rs` (D3), `fs/memory/**`.

## Contract

### Events (`telemetry/mod.rs`)

```rust
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum TelemetryEvent {
    ContextDelivered { stage_id: String, session_id: String, context_epoch: String, items: usize },
    ContextUnavailable { stage_id: String, session_id: String, reason: String },
    PromptBrief { stage_id: Option<String>, session_id: Option<String>, items: usize, estimated_tokens: usize, omitted: usize },
    PromptAbstained { stage_id: Option<String>, session_id: Option<String>, reason: String },
    ContextPulled { stage_id: Option<String>, session_id: Option<String>, query_chars: usize, budget_tokens: usize, items: usize, estimated_tokens: usize, unmet_required: usize },
}
pub struct TelemetryRecord { pub at: DateTime<Utc>, #[serde(flatten)] pub event: TelemetryEvent }   // one JSON line
pub fn emit(work_dir: &Path, event: &TelemetryEvent) -> Result<()>      // signature unchanged; never fails the caller
pub fn read_events(work_dir: &Path) -> Result<Vec<TelemetryRecord>>
```

`emit` appends a `TelemetryRecord` line under an exclusive lock to `<work_dir>/telemetry/events.jsonl`; when the write is denied (`PermissionDenied` or `EROFS` — reuse the `is_write_denied` shape from `commands/memory/handlers/record.rs`, copied into `telemetry/spool.rs`), it appends the same line to `<worktree>/.loom/telemetry-spool.jsonl` (`pub const TELEMETRY_SPOOL_RELPATH`, cap 1 MiB, `lock_exclusive`), found through `crate::commands::memory::handlers::work_dir::find_worktree_root_from_cwd` or the equivalent helper in `fs/memory/spool.rs` (grep it; reuse, do not copy the search).

### Drain

`telemetry::spool::drain_into_events(work_dir: &Path, worktree_root: &Path) -> DrainOutcome` mirrors `fs::memory::drain_into_journal`: sink every line into `events.jsonl`, then truncate under one lock; malformed lines counted and dropped. Called from `spool_drain.rs`'s per-tick loop right after the memory drain for the same stage, and from `git/cleanup/batch.rs::drain_spool_before_removal` before the worktree is removed; `git/cleanup/worktree.rs` removes the spool file alongside the memory spool; `git/worktree/settings.rs` grants the path the same way it grants `MEMORY_SPOOL_RELPATH`.

### `ContextPulled` in `commands/knowledge/context.rs`

After `retrieve_for_stage` succeeds: `let _ = telemetry::emit(&work_dir_root, &TelemetryEvent::ContextPulled { stage_id: stage.clone(), session_id: non_empty LOOM_SESSION_ID, query_chars: query.chars().count(), budget_tokens, items: pack.items.len(), estimated_tokens: pack.estimated_tokens, unmet_required: pack.unmet_required.len() })`. The work root is `WorkDir::new(".")?.root()` (the same resolution `stage_dependency_ids` in that file already performs).

### `loom knowledge telemetry`

```rust
// commands/knowledge/telemetry.rs
pub fn telemetry(stage: Option<String>, json: bool) -> Result<()>
```

Reads `read_events(work_dir)` (via `readonly` resolution — never creates the work dir; prints `no telemetry recorded` when absent) and prints one row per stage (or checkout key), sorted by stage: `stage | briefs (items) | prompt briefs emitted / abstained (top reason) | pulls (avg budget, avg items, unmet) | last event`. `--stage <id>` narrows; `--json` prints `{"stages": [{"stage_id", "spawn_briefs", "spawn_items", "prompt_briefs", "prompt_abstained": {"total", "by_reason": {...}}, "pulls", "pull_avg_budget", "pull_avg_items", "pull_unmet", "last_at"}]}`. Summary logic lives in `telemetry/summary.rs` as a pure function over `&[TelemetryRecord]` so it is unit-testable without files.

## Steps

1. `telemetry/mod.rs`: `TelemetryRecord`, the variants, timestamped writer, spool fallback; keep `stage_telemetry.rs` compiling (it constructs the two existing variants — unchanged field names).
2. `telemetry/spool.rs` + drain wiring in the three cleanup/drain files + settings grant + `.gitignore`.
3. `telemetry/summary.rs` + `commands/knowledge/telemetry.rs`.
4. `context.rs`: the one emit call.
5. Tests: `telemetry/mod.rs` inline — `events_round_trip_with_timestamps`; `emit_falls_back_to_the_spool_when_the_state_root_is_read_only` (make `<work>/telemetry` a read-only dir on a temp fs; skip the test on Windows); `drain_moves_spooled_lines_and_truncates`; `summary_counts_briefs_abstentions_and_pulls_per_stage`; `orchestrator/core/spool_drain_tests.rs` gains `telemetry_spool_is_drained_with_the_memory_spool`; `commands/knowledge/tests_telemetry.rs` (NEW, `#[path]` like `tests_context.rs`) — `telemetry_command_prints_no_telemetry_recorded_without_a_work_dir` and one JSON-shape test.

## Done means

`cargo build --manifest-path loom/Cargo.toml` warning-free (with C1's and D3's parallel halves merged); `cargo test --lib telemetry::`, `--lib orchestrator::core::spool_drain`, `--lib commands::knowledge::tests_telemetry` green; `rg -n "read_events" loom/src --glob '!**/tests*'` shows a production caller in `commands/knowledge/telemetry.rs`.

## Proof (ONE narrow check, run once)

`cargo test --manifest-path loom/Cargo.toml --lib telemetry` — skip if unsure.

## Constraints and traps

- Telemetry is an OPTIMISATION record, never state a run depends on: `emit` must never return `Err` to a caller path that spawns a stage or answers a prompt (the existing `emit` swallows; keep that).
- Attribution of a spooled event is by worktree location, exactly like memory; do not trust a stage id inside the payload for the drain's destination (it is written into `events.jsonl` verbatim as data).
- Do not store prompt text or chunk bodies in events — counts, sizes, ids only (the report's "separate content from operational counters").
- C1 calls the variants with exactly the field names above; D3 declares your module and wires `telemetry(stage, json)`. If either does not compile against your code, report it — do not edit their files.
- No git commands.

## Report back

Files changed and created; the final `TelemetryEvent` definition verbatim; the spool path constant; the human table's exact header line; anything unresolved.
