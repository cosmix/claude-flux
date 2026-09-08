# Entry Points

> Key files agents should read first to understand the codebase.
>
> **Related files:** [architecture.md](architecture.md) for system overview, [patterns.md](patterns.md) for design patterns.

## CLI Entry Point

- `loom/src/main.rs` - CLI entry (clap `#[derive(Parser)]`), `Commands` enum dispatch
- `loom/src/lib.rs` - Module exports (14 public modules)

## Command Dispatch (cli/types.rs)

| Command       | Entry File                    | Purpose                                        |
| ------------- | ----------------------------- | ---------------------------------------------- |
| `init`        | `commands/init/execute.rs`    | Initialize `.work/` from plan                  |
| `run`         | `commands/run/mod.rs`         | Start orchestrator daemon                      |
| `status`      | `commands/status.rs`          | Dashboard with stage/session info              |
| `stop`        | `commands/stop.rs`            | Shutdown daemon                                |
| `resume`      | `commands/resume.rs`          | Resume work on a stage                         |
| `sessions`    | `commands/sessions.rs`        | List/kill active sessions                      |
| `worktree`    | `commands/worktree_cmd.rs`    | List/clean/remove worktrees                    |
| `graph`       | `commands/graph/mod.rs`       | Show execution graph                           |
| `stage`       | `commands/stage/`             | Stage lifecycle (15+ subcommands)              |
| `handoff`     | `commands/handoff/create.rs`  | Create handoff files                           |
| `knowledge`   | `commands/knowledge/mod.rs`   | Manage codebase knowledge                      |
| `memory`      | `commands/memory/handlers.rs` | Session memory journal                         |
| `review`      | `commands/review/mod.rs`      | Generate review docs from memories             |
| `self-update` | `commands/self_update/mod.rs` | Update loom binary                             |
| `clean`       | `commands/clean.rs`           | Clean up resources                             |
| `repair`      | `commands/repair.rs`          | Fix workspace issues                           |
| `map`         | `commands/map.rs`             | Codebase structure analysis                    |
| `pressure`    | `commands/pressure/mod.rs`    | Plan pressure-testing driver (Claude + Codex)  |
| `diagnose`    | `commands/diagnose.rs`        | Stage failure diagnosis                        |
| `plan verify` | `commands/plan/verify.rs`     | Validate plan file without side effects        |
| `check`       | `commands/verify.rs`          | Goal-backward verification (`verify::execute`) |
| `skill-index` | `commands/skill_index.rs`     | Build skill keyword index for skill-trigger    |
| `completions` | `commands/completions/mod.rs` | Shell completions (custom scripts + dynamic)   |
| `complete`    | Hidden (dynamic completions)  | Backend for shell tab completions              |

Total: 23 visible commands + 1 hidden (`complete`, for dynamic completions). Dispatch: `cli/dispatch.rs` match-based, two-level for nested commands.

**Three commands that do NOT exist** (an earlier version of this table listed all three — verify against `cli/dispatch.rs` before citing one):

- `loom hooks` — there is no `commands/hooks.rs`. Hook install lives in `fs/permissions/hooks.rs` and runs as part of `loom init` / `loom repair --fix`.
- `loom sandbox` — there is no `commands/sandbox/`. Sandbox config generation lives in `sandbox/config.rs` + `sandbox/settings.rs`, driven by the plan.
- `loom verify` — there is no top-level `verify` command and no `commands/check.rs`. `commands/verify.rs::execute` is reached via `loom check`; `loom plan verify` is the only separate verify subcommand. The unsafe `loom stage verify` completion pipeline was removed.

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

## Plan Parsing Pipeline

- `plan/parser.rs` - Markdown plan parser (extracts YAML from `<!-- loom METADATA -->`)
- `plan/schema/types.rs` - LoomMetadata, StageDefinition structs
- `plan/schema/validation.rs` - Stage validation (goal-backward required for Standard only)
- `plan/graph/mod.rs` - Execution DAG with cycle detection

## Git Operations

- `git/worktree/operations.rs` - Create/remove worktrees at `.worktrees/{stage-id}/`
- `git/worktree/base.rs` - Base branch resolution for dependencies
- `git/worktree/settings.rs` - Worktree symlinks (.work, .claude/CLAUDE.md, CLAUDE.md)
- `git/merge/mod.rs` - Merge automation, conflict handling; `require_no_active_merge` guard
- `git/merge/in_progress.rs` - Single source of truth for `MERGE_HEAD` detection (handles `.git`-as-file, relative gitdirs, octopus merges)
- `git/merge/lock.rs` - Stable-inode OS lock that serializes concurrent merges without stale-file reclamation races
- `git/merge/status.rs` - `check_merge_state` (Merged | Pending | Conflict | BranchMissing | Unknown)
- `git/branch.rs` - Branch creation, deletion, ancestry checks

## File System State

- `fs/work_dir.rs` - `.work/` directory management (initialize, load, main_project_root)
- `fs/stage_files.rs` - Stage file naming (`{depth}-{stage-id}.md`)
- `fs/session_files.rs` - Session file operations
- `fs/knowledge.rs` - Knowledge directory operations
- `fs/memory.rs` - Session memory operations
- `fs/verifications.rs` - Goal-backward verification results

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

## Handoff System

- `commands/handoff/create.rs` - CLI `loom handoff create` implementation
- `handoff/detector.rs` - Context threshold detection
- `handoff/generator/mod.rs` - Handoff file generation
- `handoff/schema.rs` - HandoffV2 structured format

## Sandbox

- `sandbox/config.rs` - MergedSandboxConfig, merge_config(), expand_paths()
- `sandbox/settings.rs` - generate_settings_json(), write_settings()

## Hooks

The full hook roster — every script in `hooks/`, the event it binds to, what it blocks — plus
`hooks/_common.sh`'s shared helpers and the registration sites a new hook must be added to.

→ [Hook Entry Points](entry-points/hooks.md)

## Schema-to-Runtime Conversion

- `plan/schema/types.rs` - StageDefinition (YAML input); SandboxConfig + StageSandboxConfig with `permission_mode: Option<PermissionMode>`
- `models/stage/types.rs` - Stage (runtime model)
- `models/stage/methods.rs` - canonical `Stage::from_definition()` conversion
- `commands/init/plan_setup.rs` - delegates stage creation to the canonical conversion

## CLI Subcommand Registration Pattern

Three files to add a new subcommand:

1. `cli/types_memory.rs` - Define variant in KnowledgeCommands/MemoryCommands enum
2. `cli/dispatch.rs` - Add dispatch match arm
3. `commands/<module>/` - Implement handler

## Remote Control Module

- `loom/src/remote_control.rs` - `resolve_invocation(work_dir, name)` per-spawn gate (layers a `--help` probe over `resolve()`, now called only by the crash handler), `preflight(path)`, `write_unsupported_marker(work_dir)`, `run_startup_preflight(path, work_dir)`, `RemoteControlInvocation` / `RemoteControlConfig` / `RemoteControlMode` types

## Other Modules

- `src/claude.rs` - Shared find_claude_path() utility
- `completions/generator.rs` - Custom shell script generation (bash/zsh/fish)
- `completions/dynamic/mod.rs` - Context-aware dynamic completion engine
- `completions/dynamic/commands.rs` - Per-command completion definitions
- `completions/scripts/` - Shell-specific completion script templates
- `completions/install.rs` - Auto-install and migration for shell completions
- `commands/status/ui/tui.rs` - TUI dashboard entry (run_tui)
- `commands/self_update/mod.rs` - Installation, update, skill download
- `process/mod.rs` - bounded subprocess execution and structured timeout errors
- `process/identity.rs` - PID plus start-time identity verification and fail-closed signaling
- `process/environment.rs` - minimal allowlisted environment reconstruction for stage processes
- `skills/` - SkillIndex, SkillMatch, SkillMetadata (index.rs, matcher.rs, types.rs)
- `diagnosis/signal.rs` - generate_diagnosis_signal(), DiagnosisContext
- `map/analyzer.rs` - analyze_codebase(root, deep, focus)

## Key Config Files

- `.work/config.toml` - Active plan reference and settings
- `.work/stages/{depth}-{stage-id}.md` - Stage state (YAML frontmatter)
- `.work/sessions/{session-id}.md` - Session tracking
- `.work/signals/{session-id}.md` - Agent instruction signals
- `doc/plans/PLAN-*.md` - Plan definition files

## Verification System

- `verify/criteria/runner.rs` - Acceptance criteria execution: handles AcceptanceCriterion::Simple (5min) and Extended (30s + output checks) + detect_stderr_warnings()
- `verify/criteria/executor.rs` - Single criterion with timeout, SIGKILL on timeout
- `verify/goal_backward/mod.rs` - Goal-backward verification (artifacts, wiring, wiring_tests, dead_code) — truths removed from goal-backward
- `verify/goal_backward/truths.rs` - verify_truth_checks() retained for before_after.rs only
- `verify/transitions/state.rs` - Atomic stage status changes
- `verify/baseline/` - Change impact detection (capture, compare)
- `verify/before_after.rs` - Before/after stage checks using TruthCheck definitions

## Plan Validation Functions (plan/schema/validation.rs)

Key public functions for `loom plan verify` to call:

| Function                                            | Return                             | Severity                                       |
| --------------------------------------------------- | ---------------------------------- | ---------------------------------------------- |
| `validate(&metadata)`                               | `Result<(), Vec<ValidationError>>` | Fatal — called by `parse_plan()` automatically |
| `validate_structural_preflight(&stages, repo_root)` | `Vec<String>`                      | Advisory warnings                              |
| `check_knowledge_recommendations(&stages)`          | `Vec<String>`                      | Advisory suggestions                           |
| `check_sandbox_recommendations(&metadata)`          | `Vec<String>`                      | Advisory suggestions                           |

`validate()` runs inside `parse_and_validate()` → called by `parse_plan_content()` → called by `parse_plan()`. Any new command that calls `parse_plan()` automatically gets fatal validation for free.

## Plan Parser Module (plan/parser/mod.rs)

**Note:** `plan/parser` is a **subdirectory**, not a single file. Entry point is `plan/parser/mod.rs`.

- `parse_plan(path: &Path) -> Result<ParsedPlan>` — reads file + validates
- `parse_plan_content(content: &str, source_path: &Path) -> Result<ParsedPlan>` — for tests without I/O
- `load_stage_definition_from_plan(work_dir, stage_id) -> Result<StageDefinition>` — reads config.toml for plan path, resolves path, parses plan, finds stage by ID. Centralized here after PLAN-anti-slop-thoroughness; was previously inlined in commands/verify.rs and re-inlined in generate.rs.

`ParsedPlan` fields: `id` (from filename stem), `name` (first H1), `source_path`, `stages: Vec<StageDefinition>`, `metadata: LoomMetadata`.

Internal modules: `extraction.rs` (YAML block extraction, plan name), `validation.rs` (YAML parse + `validate()`).

## Execution Graph Build (plan/graph/mod.rs)

- `ExecutionGraph::build(stages: Vec<StageDefinition>) -> Result<Self>` — two-pass: first creates nodes, second builds reverse-dependency edges, then calls `cycle::detect_cycles()` via DFS
- `ExecutionGraph::update_ready_status()` → returns stage IDs that became `Queued`
- Cycle detection: `cycle/mod.rs` uses recursive DFS with `visiting` / `visited` sets; returns `Err` with cycle path on detection
- `plan/graph/loader.rs` has `build_execution_graph()` that loads stage files from `.work/stages/` and calls `ExecutionGraph::build()`

## Status Command (commands/status/)

- `commands/status.rs` - Entry point; dispatches to 3 modes
- `commands/status/data.rs` - `collect_status_data()` — loads stages, sessions, plan into `StatusData`
- `commands/status/render/` - Renderers: `render_progress()`, `render_graph()`, `render_merge_status()`, `render_compact()`, `render_attention()`
- `commands/status/ui/` - TUI for `--live` mode (subscribes to daemon via IPC)
- `commands/status/diagnostics.rs` - `check_directory_structure()`, `check_parsing_errors()` for `loom status validate` / `doctor`
- `commands/status/display.rs` - `count_files()` helper
- `commands/status/merge_status.rs` - Merge section data
- `commands/status/validation.rs` - `validate_markdown_files()`, `validate_references()`

**3 display modes:**

| Mode             | Flag        | Behavior                                                                                  |
| ---------------- | ----------- | ----------------------------------------------------------------------------------------- |
| Static (default) | none        | Snapshot: logo → plan name → daemon indicator → progress bar → stage graph → merge status |
| Compact          | `--compact` | Single-line scripting output via `render_compact()`                                       |
| Live             | `--live`    | TUI subscribed to daemon IPC; requires daemon running (`DaemonServer::is_running()`)      |

**Verbose mode (`--verbose`):** Shows `render_attention()` — detailed failure information for blocked/failed stages.

## Post-Tool Heartbeat

`hooks/post-tool-use.sh` writes only private heartbeat metadata under `.work/heartbeat/`. It does not persist tool names, commands, output, byte counts, or previews. This prevents credentials and private source printed by tools from becoming durable shared state.

The legacy `ToolEvent` reader remains able to consume an older `.work/tool-events.jsonl`, but no production hook creates or appends that file. New stuck detection therefore relies on heartbeat/session liveness rather than tool-output heuristics. If event observability is restored, it must use a bounded no-follow Rust writer and metadata-only records.

## Orchestrator Core Recovery Functions (Exact Locations)

| Function                            | File                                  | Lines   | Called From                        |
| ----------------------------------- | ------------------------------------- | ------- | ---------------------------------- |
| `sync_graph_with_stage_files()`     | `orchestrator/core/recovery.rs`       | 179-567 | orchestrator.rs main loop (tick 2) |
| `sync_queued_status_to_files()`     | `orchestrator/core/recovery.rs`       | 569-593 | orchestrator.rs main loop (tick 3) |
| `recover_orphaned_sessions()`       | `orchestrator/core/recovery.rs`       | 595-791 | startup init only                  |
| `reconcile_and_update_graph()`      | `orchestrator/core/recovery.rs`       | 149-177 | orchestrator.rs (tick 1 + startup) |
| `spawn_merge_resolution_sessions()` | `orchestrator/core/merge_handler.rs`  | 637-758 | orchestrator.rs (tick 4)           |
| `start_ready_stages()`              | `orchestrator/core/stage_executor.rs` | 64-86   | orchestrator.rs (tick 6)           |

## Plan Graph Loader — Stage File Preference (Critical)

`plan/graph/loader.rs:56` — `build_graph_impl()`:

- **Lines 60-86**: Prefers `.work/stages/` over plan file. If stages_dir exists with .md files → load from `fs::load_stages_from_work_dir()` + recover sandbox from `.work/config.toml [plan_sandbox]`. Falls back to parsing plan file only if stages_dir is empty/missing.
- This means plan-file edits are NOT automatically reflected until stages_dir is absent (i.e., fresh init).
- **`plan/amendment.rs` honors this** — a runtime amendment rewrites the plan file **and** the target stage's `.work/stages/<n>-<id>.md` under the same lock. This is a shipped guarantee, not an outstanding requirement (an earlier version of this bullet read as a TODO for a future "plan-amendment stage"). Any _other_ code path that edits a plan file at runtime must do the same, or the daemon keeps serving the old criteria.

## Plan Schema — StageDefinition Amendable Fields

`plan/schema/types.rs:306` — `StageDefinition` struct:

- Line 316: `acceptance: Vec<AcceptanceCriterion>` — amendable in v1
- Line 336: `wiring: Vec<WiringCheck>` — amendable in v1
- Line 347/352: `before_stage`/`after_stage: Vec<TruthCheck>` — deferred to v2
- Line 333: `artifacts: Vec<String>` — deferred to v2
- NOT amendable: `id`, `name`, `dependencies`, `working_dir`, `model`, `sandbox`, `execution`

## WorkDir Directory Helpers (Existing vs. Missing)

`fs/work_dir.rs:270-294` — existing helpers:

- `signals_dir()` → `.work/signals/`
- `handoffs_dir()` → `.work/handoffs/`
- `archive_dir()` → `.work/archive/`
- `stages_dir()` → `.work/stages/`
- `sessions_dir()` → `.work/sessions/`
- `crashes_dir()` → `.work/crashes/`
- `knowledge_dir()` → `.work/knowledge/`
- `ensure_dir(&self, name: &str) -> Result<PathBuf>` — create any subdir on demand

**Both helpers are now implemented:** `disputes_dir()` → `.work/disputes/` (`fs/work_dir.rs:239`) and `plan_versions_dir()` → `.work/plan_versions/` (`fs/work_dir.rs:244`)

Two more subdirectories, resolved by their own modules rather than a `WorkDir` helper:

- `acceptance-cache/` — cached acceptance passes, `<sha256>.json`, keyed by criterion text and tree digest (`verify/criteria/cache.rs`)
- `capsules/` — per-session generated Claude Code settings files (`<session-id>.settings.json`) for judge sessions, written at spawn and removed on close (`terminal/native/session_settings.rs`)

## Sandbox Settings — ANTHROPIC_API_KEY

`sandbox/settings.rs:16-34` — `SENSITIVE_ENV_KEYS` array filters `ANTHROPIC_API_KEY` from agent sandbox environments.

- This is env hygiene only. It no longer has anything to do with adjudication: an earlier version of this line claimed an absent `ANTHROPIC_API_KEY` disabled adjudication and sent disputes straight to `NeedsHumanReview`, which stopped being true when the adjudicator moved to a spawned `claude -p` session. What disables it now is a missing `claude` binary — see conventions.md § Adjudicator Transport Convention.

## HTTP Client Pattern — self_update/client.rs

`commands/self_update/client.rs` — `create_http_client() -> Result<Client>`:

- `Client::builder().connect_timeout(10s).timeout(120s).user_agent("loom-self-update").build()`
- `validate_response_status(&response, context)` — checks `is_success()`, returns descriptive HTTP errors
- Streaming download with size limit enforcement (buffer size 8192)
- Error propagation: `.context("Failed to ...")` pattern throughout

This is the pattern for loom's HTTP consumers (self-update). The adjudicator is NOT one of them: an earlier version of this line said an adjudicator HTTP client should mirror it, but the adjudicator spawns a `claude -p` session and makes no HTTP call at all — see conventions.md § Adjudicator Transport Convention.

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
- The agent never writes `verdict.md` or `applied.marker` — both are daemon-only.
- With no daemon listening the dispute cannot be filed at all (the daemon is what persists it), and the command says so rather than reporting a bare connect error.
- Server-side handler: `daemon/server/dispute.rs`. On-disk schema: `models/dispute.rs`.

## Fix Attempts Counter — Current Usage

`models/stage/types.rs:254` — `fix_attempts: u32` field:

- Incremented: `commands/stage/merge.rs` for an actual merge retry, using locked `update_stage`
- Reset to 0: `commands/stage/human_review.rs:87` on human approve
- Default max: 3 (via `get_effective_max_fix_attempts()` in methods.rs)
- Warning printed when limit reached with hint to `loom stage dispute-criteria`

Alongside it on the `Stage` struct (all shipped): `dispute_count` (600), `evidence_rounds` (603), `amendments_applied` (606).

## Remote Control & Permission Mode Integration Points

Every file and call site involved in remote-control capability detection and permission-mode
resolution, with line references.

→ [Remote Control & Permission Mode](entry-points/remote-control.md)

## Signal Generation — Key Files and Line References

| File                                      | Purpose                                                                                                                   | Key Lines                                                                                 |
| ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
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

## TruthCheck / before_stage / after_stage / code_review

| Location                                                        | Purpose                                                                                                                                                          |
| --------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
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

## `loom pressure` — Plan Pressure-Testing Files

- `loom/src/commands/pressure/mod.rs` — the driver. Key fns: `resolve_plan_path` (raw→`doc/plans/` fallback, `is_file()` check, repo-relative `invocation`), `codex_report_path` (`codex-<basename>` sibling), `codex_log_path`/`claude_marker_path` (per-pid temp paths), `plan_steps` (ordered pipeline; `Step::{DeleteReport, Pressure{claude,codex}, Address}` — the `Pressure` variant is the parallel Claude+Codex pair), `claude_args(slash, marker, model)`/`codex_args(repo_root, skill, model)` (single-source argv builders; `claude_args` injects `completion_instruction(marker)` via `--append-system-prompt`), `render_dry_run`/`render_dry_run_step`, `classify_exit`/`classify_code`, `run_claude_foreground` (foreground TTY + marker-watch, delegating shutdown to `terminate_idle_session` — SIGTERM→grace→SIGKILL; returns `ClaudeOutcome`), `spawn_codex_background`/`wait_codex` (background codex → log + spinner), `should_stop`/`claude_should_stop`, `print_run_header`, `run_pressure_step`/`run_pipeline` (both take `StepContext<'a>`, the bundle of paths + models that keeps the per-step helpers to one parameter), `execute`. Unit tests in `loom/src/commands/pressure/tests.rs`.
- `loom/src/commands/pressure/models.rs` — `PressureModels{claude,codex,address}` + `resolve(flags…, &UserConfig)`: per-slot precedence CLI flag → `~/.loom/config.toml` → built-in default. The two Claude slots (`/pressure`, `/address`) are independent.
- `loom/src/claude.rs` / `loom/src/codex.rs` — `CLAUDE_MODELS`/`CODEX_MODELS` (accepted vocabularies) and `DEFAULT_PRESSURE_CLAUDE_MODEL`/`DEFAULT_PRESSURE_CODEX_MODEL`. One source of truth for the clap parser, the user-config registry and the completions.
- `loom/src/codex.rs` — `find_codex_path()` codex binary resolver (exported via lib.rs).
- `loom/src/cli/types_pressure.rs` — `PressureArgs { plan, rounds (default 2, ≥1), dry_run, claude_model, codex_model, address_model }`, each model flag validated by a clap `PossibleValuesParser` over the vocabularies above. `cli/types.rs` holds only the tuple variant `Commands::Pressure(PressureArgs)` (it sits at its 400-line ceiling); dispatched in `loom/src/cli/dispatch.rs`.
- `loom/src/user_config/keys.rs` — the `pressure.{claude,codex,address}_model` keys in `KEYS`, as `ValueKind::Enum` over the same vocabularies, so `loom config` and its TUI edit them with no per-key wiring.
- `commands/{pressure,address,distill}.md` — vendored Claude slash commands (source for `~/.claude/commands/`).
- `codex/skills/pressure/SKILL.md` — vendored Codex pressure skill (source for `~/.codex/skills/pressure/`).
- `install.sh` — `install_commands()` (~line 336) and `install_codex_skill()` (~line 356), called only in the LOCAL (non-curl-pipe) branch of `main()` (~line 619).

## Tiered Knowledge Base (2026-07-28)

| Path                                               | Role                                                                                                                        |
| -------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| `loom/src/fs/knowledge/types.rs`                   | `KnowledgeFile`, `KnowledgeTarget`, `KnowledgeLayout`, tier-1 alias table                                                   |
| `loom/src/fs/knowledge/dir.rs`                     | `KnowledgeDir` — initialize, append, layout detection; `replace_section`/`replace_section_target` delegate the actual splicing to `splice.rs`                                                      |
| `loom/src/fs/knowledge/splice.rs`                  | `splice_section` — level-agnostic (`##` through `######`) section splicer; returns `SectionOutcome`                                                   |
| `loom/src/fs/knowledge/index.rs`                   | `scan_topics`, `generate_index`, `write_index`                                                                              |
| `loom/src/fs/knowledge/templates.rs`               | tier-1/tier-2 scaffolds                                                                                                     |
| `loom/src/commands/knowledge/mod.rs`               | the four knowledge verbs — `update` (append), `replace-section` (overwrite the section body at any heading level `##` to `######`), `context`, `sync`               |
| `loom/src/cli/types_memory.rs`                     | clap definitions for the knowledge subcommands                                                                              |

`loom knowledge sync`, or any `loom knowledge update`, regenerates `INDEX.md` — the index
regenerates on every knowledge write — and on a flat directory creates it, which is what flips
the layout to hierarchical. See [Knowledge Hierarchy](architecture/knowledge-hierarchy.md).

## Subagent Verification Guard (2026-07-28)

| Path                                                             | Role                                                                                                   |
| ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `hooks/subagent-verify-guard.sh`                                 | PreToolUse:Bash guard — blocks project-wide verification for subagents (over the Rule 17 400-line cap — see [concerns.md](concerns.md)) |
| `hooks/_common.sh`                                               | `loom_is_subagent()` — payload-first detection gate, process-tree walk as fallback; 619 lines, also over the Rule 17 cap — see [concerns.md](concerns.md) |
| `loom/src/orchestrator/signals/tests_doctrine.rs`                | pins the doctrine blocks byte-for-byte across signal, template, and hook                               |
| `loom/tests/integration/hooks_subagent_verify_guard.rs`          | harness: process-tree construction, env scrubbing, payload building                                    |
| `loom/tests/integration/hooks_subagent_verify_guard_cases.rs`    | `BLOCK_CASES` / `ALLOW_CASES` table data                                                               |
| `loom/tests/integration/hooks_subagent_verify_guard_carveout.rs` | integration-verify carve-out **refusal** directions (decoy, wrong type, missing)                       |

Split into three files because they grow for different reasons; wired with `#[path]` submodules
so the children reach the parent's private helpers via `use super::*` without widening visibility.

## Terminal Backends and `loom attach` (2026-08-08)

The dispatcher/lanes, `[terminal]` config, `--backend` flag, and `loom attach` entry
points all live in one topic file now.

Full detail: [terminal-backends.md](architecture/terminal-backends.md).

## Context Retrieval Subsystem (2026-08-17)

Read `loom/src/context/mod.rs` FIRST — its docstring carries an accurate pipeline diagram
and states plainly which channel is wired. Then the file for what you touch:

| Path                                                    | What it owns                                                                                                             |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
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
| `context/extract/`                                      | `SourceGraphExtractor` trait, `registry()`, `ExtractorIdentity`, `treesitter.rs` shared harness, one module per language |
| `context/resolve/`                                      | cross-file symbol resolution, `impact`, `SymbolIndex`                                                                    |
| `telemetry/mod.rs`                                      | `TelemetryEvent`, `emit`, `read_events` over `.work/telemetry/events.jsonl`                                              |
| `orchestrator/signals/format/brief.rs`                  | renders the Knowledge Brief into a stage signal                                                                          |
| `orchestrator/core/stage_telemetry.rs`                  | the only telemetry writer, called from `stage_executor.rs:570`                                                           |
| `orchestrator/merge_lifecycle.rs`                       | merge/verify/cleanup ordering; the single door to post-merge cleanup                                                     |

## Execution Containment (2026-08-17)

| Path                                          | What it owns                                                                                                            |
| --------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `verify/criteria/confine.rs`                  | `spawn_confined` (the single leaf primitive for every plan-authored command), `resolve_confinement`, `plan_confinement` |
| `process/environment.rs`                      | `STAGE_HOST_ENV_ALLOWLIST`, `apply_stage_environment`                                                                   |
| `models/stage/types.rs:255`                   | `CommandConfinement`; `:340` `NetworkConfig`                                                                            |
| `orchestrator/terminal/native/wrapper.rs:181` | a SECOND, diverging copy of the env allowlist                                                                           |

Start from `architecture/execution-containment.md` — it states precisely what these do and
do not guarantee, which is narrower than the word "containment" suggests.

## New CLI Surface (2026-08-17)

**Documented from the clap definitions, not from `--help`.** The installed PATH binary can
lag `main` mid-plan: at the end of this plan `loom knowledge context` worked while
`loom map --outline`, `loom context` and `loom hook` all reported "unrecognized subcommand"
from the same binary. Verify a flag against `loom/src/cli/` before documenting it.

| Command                              | Defined in                      | Options                                                                                                                                                                                                                                  |
| ------------------------------------ | ------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `loom knowledge context`             | `commands/knowledge/context.rs` | `--query <QUERY>` (**required**), `--stage <STAGE>`, `--budget-tokens <N>` (default 2000), `--scope <knowledge\|source\|all>` (default `all`), `--require-id <ID>` (repeatable), `--explain`, `--json`.                                  |
| `loom knowledge sync`                | `commands/knowledge/sync.rs`    | `--structural-only`, `--json`. Rebuilds derived context artifacts after the knowledge tree changes.                                                                                                                                      |
| `loom map --outline <PATH>`          | `commands/map.rs:34`            | prints the indexed symbols of one file, in source order                                                                                                                                                                                  |
| `loom map --find-all <SYMBOL>`       | `commands/map.rs:37`            | prints every indexed node whose name matches                                                                                                                                                                                             |
| `loom map --impact <SYMBOL_OR_PATH>` | `commands/map.rs:40`            | prints what reaches a symbol or file, with path confidence                                                                                                                                                                               |
| `loom context record-edit`           | `cli/types_ops.rs:78`           | `--stage <STAGE>`, `--path <PATH>` (**required**, repeatable). Keeps a stage's context overlay current.                                                                                                                                  |
| `loom hook user-prompt`              | `cli/types_ops.rs:92`           | no options. `UserPromptSubmit` entry point; emits a retrieval brief or nothing.                                                                                                                                                          |

The three `loom map` view flags short-circuit the original knowledge-file analysis
(`map.rs:56-58`): if any is set, `map` is a read-only source-graph view and does not write
knowledge. `--deep`, `--focus` and `--overwrite` remain the analysis-mode flags.

Plan YAML gained `command_confinement: confined | inherit` at plan level
(`plan/schema/types.rs:52`) and as a per-stage override (`models/stage/types.rs:305`).

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
