---
---
# Orchestrator Daemon And Sessions

> Orchestrator loop, daemon, monitor, signals, merges, dispute/verify

## Orchestrator Core

- `orchestrator/core/orchestrator.rs` - Main loop (5s polling)
- `orchestrator/core/stage_executor.rs` - Worktree creation, signal gen, session spawn
- `orchestrator/core/event_handler.rs` - Dispatches StageCompleted, SessionCrashed, etc.
- `orchestrator/core/crash_handler.rs` - Failure classification, exponential backoff
- `orchestrator/core/completion_handler.rs` - Auto-merge BEFORE marking completed
- `orchestrator/core/merge_handler.rs` - Conflict detection, merge session spawning
- `orchestrator/core/persistence.rs` - Load/save state to disk

## Data Models

- `models/stage/types.rs` - Stage struct, StageStatus enum (13 states, including NeedsAdjudication)
- `models/stage/transitions.rs` - State transition validation
- `models/stage/methods.rs` - Stage operations (try_mark_executing, try_complete, timing)
- `models/session/types.rs` - Session struct, SessionStatus enum (6 states)
- `models/failure.rs` - FailureType enum (10 variants, retryable vs non-retryable)

## Daemon

- `daemon/server/core.rs` - `DaemonServer` state and bounded-client constants
- `daemon/server/lifecycle.rs` - Daemonization, authoritative singleton-lock lifetime, socket binding, accept loop, and shutdown
- `daemon/protocol.rs` - IPC request/response and capability types
- `daemon/wire.rs` - Fixed authentication preface plus bounded JSON framing (64 KiB requests, 2 MiB responses)
- `daemon/server/admission.rs` - Absolute-deadline reads and the global in-flight byte budget
- `daemon/server/pool.rs` - Fixed worker pool and bounded admission queue
- `daemon/server/storage.rs` - No-follow, mode-0600 control-file publication under the mode-0700 `.work/` directory
- `daemon/server/broadcast.rs` - Status/log streaming to clients

## Monitor Subsystem

- `orchestrator/monitor/core.rs` - Coordinates detection, heartbeat, checkpoints
- `orchestrator/monitor/detection.rs` - Stage/session state change detection, budget checks
- `orchestrator/monitor/heartbeat.rs` - Hung detection (300s timeout)
- `orchestrator/monitor/context.rs` - `context_health(tokens, ceiling)`: Green `<60%`, Yellow `60-90%`, Red `>=90%` of the resolved absolute `context_ceiling_tokens` (not a fixed 200k window — see architecture.md "Context Budget Enforcement")
- `orchestrator/monitor/failure_tracking.rs` - Consecutive failure escalation

## Signal System

- `orchestrator/signals/generate.rs` - Signal file creation (generate_signal_with_skills)
- `orchestrator/signals/cache.rs` - Stable prefix generation (4 stage-type variants, SHA-256 hash)
- `orchestrator/signals/format/mod.rs` - Full signal formatting (Manus 4-section KV-cache pattern)
- `orchestrator/signals/format/sections.rs` - Section formatters (stable, semi-stable, dynamic, recitation)
- `orchestrator/signals/format/skills.rs` - Recommended Skills section: per-skill invocations plus one combined `loom-skills` loader call
- `orchestrator/signals/helpers.rs` - write_signal_file() (disk I/O)
- `orchestrator/signals/types.rs` - EmbeddedContext, DependencyStatus, SandboxSummary
- `orchestrator/signals/knowledge.rs` - generate_knowledge_signal() (knowledge stages)
- `orchestrator/signals/crud.rs` - Signal file CRUD
- `orchestrator/signals/merge.rs` - Merge conflict resolution signals
- `orchestrator/signals/recovery.rs` - Recovery signal generation

## Signal Generation — Key Files and Line References

| File                                      | Purpose                                                                                                                   | Key Lines                                                                                 |
| ------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `orchestrator/signals/generate.rs`        | Entry point: `generate_signal_with_skills()`, `build_signal_context()`, `build_embedded_context_with_stage_and_session()` | 137-536                                                                                   |
| `orchestrator/signals/cache.rs`           | 4 stable-prefix generators + 8 `append_*` helpers + SignalMetrics SHA-256                                                 | helpers:51-169, standard:174-310, IV:313-444, KnowledgeDistill:447-524, Knowledge:527-633 |
| `orchestrator/signals/format/mod.rs`      | `format_signal_with_metrics()` — selects stable prefix by stage type, assembles 4 sections                                | 62-78                                                                                     |
| `orchestrator/signals/format/sections.rs` | Semi-stable (19-310), Dynamic (314-593), Recitation (597-666)                                                             | see per-section notes                                                                     |
| `orchestrator/signals/types.rs`           | `EmbeddedContext` struct (24-50), `DependencyStatus`, `SandboxSummary`                                                    | 24-50                                                                                     |
| `orchestrator/signals/knowledge.rs`       | Knowledge-stage signal path: `generate_knowledge_signal()`, `format_knowledge_signal_content()`                           | 23-135                                                                                    |
| `orchestrator/signals/recovery.rs`        | Recovery signal: recovery context header, last known state, recovery actions                                              | —                                                                                         |
| `orchestrator/signals/recovery_format.rs` | `format_recovery_signal()` if exists as separate file                                                                     | —                                                                                         |
| `orchestrator/signals/helpers.rs`         | `write_signal_file()` disk I/O                                                                                            | 17+                                                                                       |
| `orchestrator/signals/crud.rs`            | Signal file CRUD                                                                                                          | —                                                                                         |

**Insertion point for new shared helper:** `cache.rs` lines 51-169 (the "Shared content blocks" cluster). Call it from each of the 4 generator functions.

## Stage Completion (CLI)

- `commands/stage/complete.rs` - Top-level CLI completion entry; **`route_complete_for_conflicts` is the pure routing test seam** that decides Proceed vs ForceUnsafeAssumeMergedVerified vs SpawnResolver vs RevertAndSpawnResolver vs Refuse before any persistence.
- `commands/stage/merge.rs` - `loom stage merge [--resolved]`; wires `merge_verify` for ancestry check.
- `commands/stage/merge_resolver.rs` - CLI-side resolver spawn; uses `find_live_merge_session_for_stage` for single-resolver-per-stage guard.
- `commands/stage/merge_verify.rs` - `verify_or_derive_completed_commit` (read-only ancestry check shared by `--assume-merged` and `--resolved`).
- `orchestrator/merge_attribution.rs` - `attribute_main_repo_merge` and `reconcile_main_repo_active_merge` (free functions; the daemon-recovery test seam — no `Orchestrator` instance required).

## Terminal Backend

- `orchestrator/terminal/backend.rs` - `SessionBackend` dispatcher for native/tmux spawn, kill, and liveness
- `orchestrator/terminal/mod.rs` - terminal module root; re-exports backend and emulator types
- `orchestrator/terminal/native/mod.rs` - NativeBackend (spawn/kill/alive)
- `orchestrator/terminal/native/spawner.rs` - Claude Code session spawning (native)
- `orchestrator/terminal/emulator.rs` - 11 terminal emulator configs
- `orchestrator/terminal/native/detection.rs` - Auto-detect terminal
- `orchestrator/terminal/native/pid_tracking.rs` - Wrapper script, PID tracking, env vars
- `orchestrator/liveness.rs` - LivenessService: wraps the shared SessionBackend; fixed_for_tests() stub for unit tests

## Orchestrator Core Recovery Functions (Exact Locations)

| Function                            | File                                  | Lines   | Called From                        |
| -------------------------------------- | ---------------------------------------- | --------- | ------------------------------------- |
| `sync_graph_with_stage_files()`     | `orchestrator/core/recovery.rs`       | 179-567 | orchestrator.rs main loop (tick 2) |
| `sync_queued_status_to_files()`     | `orchestrator/core/recovery.rs`       | 569-593 | orchestrator.rs main loop (tick 3) |
| `recover_orphaned_sessions()`       | `orchestrator/core/recovery.rs`       | 595-791 | startup init only                  |
| `reconcile_and_update_graph()`      | `orchestrator/core/recovery.rs`       | 149-177 | orchestrator.rs (tick 1 + startup) |
| `spawn_merge_resolution_sessions()` | `orchestrator/core/merge_handler.rs`  | 637-758 | orchestrator.rs (tick 4)           |
| `start_ready_stages()`              | `orchestrator/core/stage_executor.rs` | 64-86   | orchestrator.rs (tick 6)           |

## Daemon Credentials and Operator Proofs

Daemon startup generates independent user and admin secrets. Both are published with no-follow,
mode-0600 creation beneath the mode-0700 `.work/` directory. The user secret authenticates Ping,
status/log subscriptions, Unsubscribe, and DisputeCriteria. Authentication is checked from a fixed,
allocation-free request preface before the bounded JSON body is accepted.

Startup also publishes `.ignore` and `ripgreprc` at the state root, before either token, via
`daemon/server/tokens.rs::publish_fresh_tokens`. The session wrapper
(`orchestrator/terminal/native/wrapper.rs`) exports `RIPGREP_CONFIG_PATH` pointing at the published
`ripgreprc`, but only when that file already exists at spawn time; a `loom run --foreground`
orchestrator runs no daemon, so it publishes neither tokens nor exclusions and its sessions get no
export. A sandboxed agent's `rg`/`fd`/`ag` opening `admin.token` or `user.token` hits the sandbox's
own deny rule and stalls auto mode on an operator prompt, so both files keep ordinary sweeps and
`-uu`/`--no-ignore` sweeps away from the credential files. No `Read(...)` permission deny is
written for the tokens at all; the OS-level `sandbox.filesystem.denyRead` list and
`hooks/credential-guard.sh` cover them, for the reason in concerns.md § "No `Read(...)` Deny Rule
May Exist in Any Settings File".

Privileged actions do not treat the mere presence of `.work/admin.token` as authorization. The
operator supplies that secret only to the proof-minting process through `LOOM_ADMIN_TOKEN`; the
target command receives an action-bound proof through `LOOM_ADMIN_PROOF` and never reads the token.
Proofs are HMAC-SHA256-bound to the project, action, stage (when applicable), and privileged flag
set, then consumed through a private atomic replay marker. The integrated shutdown flow is
`loom stage admin-proof --daemon-stop`, followed by `loom stop` with the minted proof in
`LOOM_ADMIN_PROOF`.

`daemon/server/client.rs` verifies the preface in constant time and fails closed on missing or
malformed credentials. `commands/stage/admin_proof.rs` owns minting, exact-request verification,
and replay protection.

## Dispute Criteria — Current Implementation

`commands/stage/dispute_criteria.rs` is a **thin RPC client**, not a state mutator:

```rust
pub fn dispute_criteria(
    stage_id: String,
    criterion_index: usize,
    reason: String,
    evidence_commit: Option<String>,
    failure_output_path: Option<PathBuf>,
) -> Result<()>
```

- CLI: `loom stage dispute-criteria <stage-id> --criterion-index N --reason <text> [--evidence-commit <sha>] [--failure-output <path>]`
- Sends `Request::DisputeCriteria` over the daemon socket. The **daemon** writes `.work/disputes/<stage>/<n>/request.md` and transitions the stage to `NeedsAdjudication`, then returns the allocated id.
- **Credentials: a missing `.work/user.token` is the NORMAL case here, not an error.** An earlier version of this section said the client "reads `.work/user.token`" and treated absence as fatal — that made the command unusable from the one place it was ever needed, because the sandbox denies a stage agent that read by design (S-1: the token authorizes every User RPC, not just the ones a stage agent is entitled to). The client now presents `daemon::rpc::user_credential()`, which falls back to a non-empty placeholder, and names the session it is running inside via `LOOM_SESSION_ID`. The daemon authorizes it by the connection instead — see `daemon/server/self_service.rs`.
- `--failure-output` is a path; the client loads it and truncates to 4KB on a UTF-8 char boundary.
- The agent never writes `.work/disputes/<stage>/<n>/verdict.md` or `applied.marker` — both are daemon-only.
- With no daemon listening the dispute cannot be filed at all (the daemon is what persists it), and the command says so rather than reporting a bare connect error.
- Server-side handler: `daemon/server/dispute.rs`. On-disk schema: `models/dispute.rs`.

## Fix Attempts Counter — Current Usage

`models/stage/types.rs:254` — `fix_attempts: u32` field:

- Incremented: `commands/stage/merge.rs` for an actual merge retry, using locked `update_stage`
- Reset to 0: `commands/stage/human_review.rs:87` on human approve
- Default max: 3 (via `get_effective_max_fix_attempts()` in methods.rs)
- Warning printed when limit reached with hint to `loom stage dispute-criteria`

Alongside it on the `Stage` struct (all shipped): `dispute_count` (600), `evidence_rounds` (603), `amendments_applied` (606).

## TruthCheck / before_stage / after_stage / code_review

| Location                                                        | Purpose                                                                                                                                                          |
| ------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `models/stage/types.rs:280-303`                                 | `TruthCheck` struct: `command`, `stdout_contains`, `stdout_not_contains`, `stderr_empty`, `exit_code`, `description`                                             |
| `plan/schema/types.rs:100-261`                                  | `StageDefinition`: `before_stage: Vec<TruthCheck>` (221), `after_stage: Vec<TruthCheck>` (226), `code_review: Option<CodeReviewConfig>` (261)                    |
| `plan/schema/types.rs:100-111`                                  | `CodeReviewConfig`: `dimensions: Vec<String>`, `require_all: bool`                                                                                               |
| `models/stage/methods.rs::Stage::from_definition`               | Canonically copies before/after checks, `code_review`, sandbox, and all execution policy into persisted Stage state                                              |
| `orchestrator/core/stage_executor.rs::before_stage_gate_passed` | Executes before_stage checks BEFORE session spawn; failure → stage Blocked. Skips the checks when `find_prior_stage_work` shows the workspace already holds work |
| `verify/before_after.rs::find_prior_stage_work`                 | Pristine-workspace probe: commits on `loom/<id>` beyond base, or non-scaffold worktree changes → `Some(evidence)` (skip the gate)                                |
| `git/branch/status.rs::list_working_tree_changes`               | `git status --porcelain` paths INCLUDING untracked (`has_uncommitted_changes` excludes them)                                                                     |
| `git/worktree/settings.rs::is_worktree_scaffold_path`           | Discounts loom-planted `.work` / `.claude/` / `CLAUDE.md` when judging whether a worktree holds agent work                                                       |
| `commands/stage/complete.rs:847-866`                            | Executes after_stage checks AFTER acceptance criteria; failure → stage stays Executing                                                                           |
| `verify/before_after.rs`                                        | `run_before_stage_checks()` + `run_after_stage_checks()` — both delegate to `verify_truth_checks()`                                                              |
| `verify/goal_backward/truths.rs:16-134`                         | `verify_truth_checks(checks, working_dir)` → `Vec<VerificationGap>`, 30s timeout per check                                                                       |

## Subagent Verification Guard (2026-07-28)

| Path                                                             | Role                                                                                                   |
| -------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `hooks/subagent-verify-guard.sh`                                 | PreToolUse:Bash guard — blocks project-wide verification for subagents (over the Rule 17 400-line cap — see [concerns.md](../concerns.md)) |
| `hooks/_common.sh`                                               | `loom_is_subagent()` — payload-first detection gate, process-tree walk as fallback; 619 lines, also over the Rule 17 cap — see [concerns.md](../concerns.md) |
| `loom/src/orchestrator/signals/tests_doctrine.rs`                | pins the doctrine blocks byte-for-byte across signal, template, and hook                               |
| `loom/tests/integration/hooks_subagent_verify_guard.rs`          | harness: process-tree construction, env scrubbing, payload building                                    |
| `loom/tests/integration/hooks_subagent_verify_guard_cases.rs`    | `BLOCK_CASES` / `ALLOW_CASES` table data                                                               |
| `loom/tests/integration/hooks_subagent_verify_guard_carveout.rs` | integration-verify carve-out **refusal** directions (decoy, wrong type, missing)                       |

Split into three files because they grow for different reasons; wired with `#[path]` submodules
so the children reach the parent's private helpers via `use super::*` without widening visibility.

## Post-Tool Heartbeat

`hooks/post-tool-use.sh` writes only private heartbeat metadata under `.work/heartbeat/`. It does not persist tool names, commands, output, byte counts, or previews. This prevents credentials and private source printed by tools from becoming durable shared state.

The legacy `ToolEvent` reader remains able to consume an older `.work/tool-events.jsonl`, but no production hook creates or appends that file. New stuck detection therefore relies on heartbeat/session liveness rather than tool-output heuristics. If event observability is restored, it must use a bounded no-follow Rust writer and metadata-only records.

## Status Command (commands/status/)

- `commands/status.rs` - Entry point; dispatches to 3 modes
- `commands/status/data/collector.rs` - `collect_status_data()` — loads stages, sessions, plan into `StatusData` (module root `commands/status/data/mod.rs` re-exports it)
- `commands/status/render/` - Renderers: `render_progress()`, `render_graph()`, `render_merge_status()`, `render_compact()`, `render_attention()`
- `commands/status/ui/` - TUI for `--live` mode (subscribes to daemon via IPC); entry point `commands/status/ui/tui/mod.rs` (`run_tui`)
- `commands/status/diagnostics.rs` - `check_directory_structure()`, `check_parsing_errors()` for `loom status validate` / `doctor`
- `commands/status/display/mod.rs` - `count_files()` helper
- `commands/status/merge_status.rs` - Merge section data
- `commands/status/validation.rs` - `validate_markdown_files()`, `validate_references()`

**3 display modes:**

| Mode             | Flag        | Behavior                                                                                  |
| ------------------ | ------------- | --------------------------------------------------------------------------------------------- |
| Static (default) | none        | Snapshot: logo → plan name → daemon indicator → progress bar → stage graph → merge status |
| Compact          | `--compact` | Single-line scripting output via `render_compact()`                                       |
| Live             | `--live`    | TUI subscribed to daemon IPC; requires daemon running (`DaemonServer::is_running()`)      |

**Verbose mode (`--verbose`):** Shows `render_attention()` — detailed failure information for blocked/failed stages.
