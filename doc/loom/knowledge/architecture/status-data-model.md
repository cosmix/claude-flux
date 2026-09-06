# Status Data Model

> Where each field shown by `loom status` (static, compact, and `--live`) comes from, and what the live TUI does not yet surface.

## Sources of Truth

Static and compact `loom status` read `StatusData`, built by `collect_status_data` (`loom/src/commands/status/data/collector.rs`). `loom status --live` does not poll it: the TUI subscribes to the daemon's push channel over the Unix socket (`SubscribeStatus`, `loom/src/commands/status/ui/tui/daemon_client.rs:15-71`; daemon side registers the subscriber in `status_subscribers`, `loom/src/daemon/server/core.rs:62`). Since the payload swap, the pushed value IS a `StatusData` built by the same collector, so static, compact and live all read one model; only the transport differs (file poll vs socket push). The TUI event loop separately polls terminal input every 100ms (`POLL_TIMEOUT`, `loom/src/commands/status/ui/tui/app.rs:38`) and re-reads scheduler alerts from files at most once a second (`ALERT_REFRESH_INTERVAL`, `app.rs:56`; `refresh_alerts`, `app.rs:184-195`); the same throttle also refreshes `TuiApp::tick_age_secs` from `orchestrator::tick::read`.

Stage files `.loom/work/stages/<id>.md` and session files `.loom/work/sessions/<id>.md` are the `Stage`/`Session` structs serialized directly as frontmatter — there is no reduced on-disk schema for status (`loom/src/fs/stage_loading.rs:91`, `collector.rs:99-105`).

## CLI Flags and Dispatch

Exactly three bool flags exist; there is no `--json`, `--watch`, or refresh-interval flag.

| Flag | Short | Type |
| --- | --- | --- |
| `--live` | `-l` | bool |
| `--compact` | `-c` | bool |
| `--verbose` | `-v` | bool |

Defined in `loom/src/cli/types.rs:81-93`; dispatched in `loom/src/commands/status.rs:148-188`.

## StageStatus — 13 Variants

`StageStatus` has 13 variants (`loom/src/models/stage/types.rs:853-917`). Icon, label, color, and bold come from `types.rs:986-1096`; the progress bucket from `types.rs:1108-1120`.

| Variant | Icon | Label | Color | Bold | Bucket | Needs a human |
| --- | --- | --- | --- | --- | --- | --- |
| Completed | `✓` U+2713 | Completed | Green | yes | Completed | no |
| Executing | `●` U+25CF | Executing | Blue | yes | Executing | no |
| Queued | `▶` U+25B6 | Queued | Cyan | yes | Pending | no |
| WaitingForDeps | `○` U+25CB | Waiting | Gray | no | Pending | no |
| WaitingForInput | `?` | Input | Magenta | yes | Executing | yes |
| Blocked | `✗` U+2717 | Blocked | Red | yes | Blocked | yes |
| NeedsHandoff | `⟳` U+27F3 | Handoff | Yellow | yes | Executing | no |
| Skipped | `⊘` U+2298 | Skipped | DarkGray | no | Completed (excluded from the completed count, `collector.rs:281-284`) | no |
| MergeConflict | `⚡` U+26A1 | Conflict | Yellow | yes | Blocked | yes |
| CompletedWithFailures | `⚠` U+26A0 | Failed | Red | yes | Blocked | yes, but auto-retried up to `max_retries` |
| MergeBlocked | `⊗` U+2297 | MergeBlk | Red | yes | Blocked | yes |
| NeedsHumanReview | `⏸` U+23F8 | Review | Magenta | no | Blocked | yes (`review_reason` set) |
| NeedsAdjudication | `⚖` U+2696 | Adjudicate | Yellow | yes | Blocked | yes |

Meanings worth remembering: `WaitingForDeps` means dependencies are not yet both completed and merged; `Queued` means ready to spawn; `NeedsHandoff` means the context ceiling was hit and a fresh session resumes the stage; `Skipped` is terminal and does not satisfy dependents; `CompletedWithFailures` and `MergeBlocked` are retryable, not terminal. `ProgressSummary` counts (`data/mod.rs:144-151`) are computed only through `bucket()` (`collector.rs:267-297`), so a caller that wants raw variant counts must not reuse `ProgressSummary`.

Related enums on `Stage`: `StageType` (`types.rs:14-29`) is Standard, Knowledge, IntegrationVerify, or KnowledgeDistill, with `default_model()` at `types.rs:46-60`. `implementers` (`types.rs:139-239`) is an ordered list of `Implementer::Claude` / `Implementer::Codex` lanes. `ExecutionMode` (`types.rs:75-81`) is Single or Team.

## Stage Fields Worth a Dashboard

`Stage` (`loom/src/models/stage/types.rs:634-831`) carries far more than status. Fields grouped by purpose:

- **Identity/graph**: id, name, description, status, dependencies, stage_type, worktree, session, held.
- **Timing**: created_at/updated_at/completed_at, started_at (first transition to Executing, `types.rs:658-661`), duration_secs (final elapsed time), execution_secs (accumulated active time, excludes backoff, `types.rs:666-670`), attempt_started_at (`types.rs:671-675`).
- **Retry/failure**: retry_count (`types.rs:685`), max_retries (`types.rs:688`, `None` means the global default of 3), last_failure_at (`types.rs:690`), failure_info: `Option<FailureInfo>` with failure_type, detected_at, evidence (`loom/src/models/failure.rs:54-63`).
- **Merge**: base_branch, base_merged_from, completed_commit, cleanup_warning (`types.rs:712-718`), merged, merge_assumed, merge_conflict, verification_status.
- **Context**: context_ceiling_tokens.
- **Adjudication/verification**: fix_attempts, max_fix_attempts, dispute_count (capped at 3, `methods.rs:11`), evidence_rounds, amendments_applied, stall_recoveries (`types.rs:777-784`), review_reason.
- **Execution policy**: model (override), reasoning_effort, implementers, subagent_timeout_secs, files, auto_merge, outputs.

Helper accessors: `effective_model()` (`methods.rs:131-135`), `effective_subagent_timeout_secs()` (`methods.rs:153-156`), `get_effective_max_fix_attempts()` (`methods.rs:476-483`).

There are five distinct counters on `Stage` — retry_count, fix_attempts, dispute_count, evidence_rounds, stall_recoveries — that track different loops. Name which one is meant; they are easy to conflate.

## Session Fields

`Session` (`loom/src/models/session/types.rs:98-139`): id, stage_id, worktree_path, pid, status, context_tokens, transcript_path, created_at, last_active, session_type, merge_source_branch/merge_target_branch, tracking_key (`types.rs:128-133`), backend.

`SessionStatus` (`types.rs:38-49`): Spawning, Running, Paused, Completed, Crashed, ContextExhausted.

`SessionType` (`types.rs:6-24`): Stage, Merge, BaseConflict, Knowledge, Adjudication.

`context_tokens: u32` is the resident token count as of the last heartbeat that carried a reading (`types.rs:104-108`); a heartbeat tick with no measurement leaves the previous value in place.

`backend: SessionBackendKind` (`types.rs:68-76,138`) is Native or Tmux.

Two things that do NOT exist on `Session`: no `model` field (model is stage-level, via `Stage.model`/`effective_model()`), and nothing that records which `Implementer` lane the session runs (that lives on `Stage.implementers`).

## Heartbeat and Activity

`Heartbeat` (`loom/src/orchestrator/monitor/heartbeat.rs:34-55`): stage_id, session_id, timestamp, context_tokens: `Option<u32>` (`None` means not measured this tick, so the previous value is kept), transcript_path, last_tool, activity.

Written by the agent hook to `.loom/work/heartbeat/<stage_id>.json` (`write_heartbeat`, `heartbeat.rs:324`), read by `read_heartbeat_for_stage` (`collector.rs:25-35`).

Staleness is seconds since the heartbeat timestamp (`collector.rs:230-234`); the threshold is 5 minutes (`collector.rs:57`).

`ActivityStatus` (`data/mod.rs:14-30`): Idle, Working, Error (session crashed), Stale, Orphaned (stage Executing but no session record). Computed by `determine_activity_status` (`collector.rs:43-61`). A finished stage (Completed/Skipped) and a session that ended without crashing both report `Idle`, regardless of heartbeat age.

PID liveness comes from `crate::process::is_process_alive` (`collector.rs:175`), recomputed on every collection and never persisted.

A context reading is shown only when `context_tokens > 0` and the session is not terminal — `reported_reading()` (`collector.rs:145-147`). The ceiling shown alongside it comes from `resolve_context_ceiling_tokens` (`loom/src/fs/work_dir/config_sections.rs:309-320`), resolution order: stage override, then workspace `[context]` config, then `~/.loom/config.toml`, then a hardcoded default.

## Payload Shapes

**`StageSummary`** (`data/mod.rs:77-147`, static/compact/live view — all three read the same struct): id, name, status, stage_type, dependencies, context_tokens, elapsed_secs (started_at to completed_at, or to now while open; None until started), execution_secs, base_branch, base_merged_from, failure_info, activity_status, last_tool, last_activity, staleness_secs, context_ceiling_tokens, review_reason, merged, merge_assumed, cleanup_warning, held, retry_count, max_retries, pid, session_alive, model, session_type, incoherence (`data/mod.rs:131`), execution_models, dispute_count, judge_heartbeat_secs, session_backend (`data/mod.rs:136-146`).

`execution_models: Vec<String>` is distinct execution-model display names observed for the stage's subagents, first-seen order (spawn ledger then codex ledger), empty until a subagent spawns — see "Execution-Model Ledgers" below. `dispute_count` and `judge_heartbeat_secs` surface adjudication state. `session_backend: Option<SessionBackendKind>` (Native/Tmux) is populated by the collector but has no reader anywhere in the tree today — see concerns.md.

**`StatusData`** (`data/mod.rs:62-73`): stages, merge (`MergeSummary`: merged, pending, conflicts, `data/mod.rs:163`), progress (`ProgressSummary`: total, completed, executing, pending, blocked, `data/mod.rs:171`), plan_name (the first H1 heading of the plan file, `collector.rs:342-349`), and quota (`QuotaSnapshot`, `loom/src/quota/model.rs:10-15`: `claude` and `codex`, each `Option<ProviderQuota>` carrying `observed_at`, up to two `windows` (five-hour first; `used_percent` 0..=100; `resets_at` epoch seconds or `None`), `plan`, `error`). The collector fills it from the on-disk cache through `quota::read_snapshot(work_dir.root())` (`collector.rs:394`), never by polling. The field is `#[serde(default)]` so an older daemon's frame still parses, but it is always serialized, so the web schema (`quotaSnapshotSchema` in `web/src/api/schema.ts`) requires it. Who writes the cache: [quota-poller.md](quota-poller.md).

**Live-mode daemon push**: `Response::StatusUpdate` (`loom/src/daemon/protocol.rs:257-259`) carries one field, `data: Box<StatusData>` — boxed because the quota field pushed the variant past clippy's large-enum threshold; serde serializes the box transparently. It is the identical model the static and compact views build. `collect_status` (`loom/src/daemon/server/status.rs`) calls the same collector, so every `StageSummary` field reaches the live TUI; there is no narrower live payload and no per-bucket split. The `StageInfo` type this section used to describe, and the four-`Vec`-per-bucket push it used, are gone from the tree.

`Response::OrchestrationComplete` carries a completion summary (`protocol.rs:36-48`; per-stage completion info, `protocol.rs:10-30`). `DaemonConfig` (`protocol.rs:60-69`: manual_mode, max_parallel, watch_mode, auto_merge) exists on the wire but nothing under `commands/status` reads it.

## Daemon Liveness and Loop Health

`DaemonStatus` (`loom/src/daemon/server/core.rs:16-36`): NotRunning, Running, ProcessOnly (process alive, socket unreachable), Unreachable (a flock proves a daemon owns the work dir, but the current sandbox denies the connect attempt — rendered as healthy).

`daemon_status_line` (`status.rs:102-145`) maps that status plus loop-stall detection to a marker, message, and hint.

Loop liveness is tracked by a single-slot tick file, `.loom/work/orchestrator.tick` (`loom/src/orchestrator/tick.rs`), stamped per phase (`tick.rs:38-47`) and overwritten each tick. It is considered stalled at 60 seconds or more (`STALL_THRESHOLD_SECS`, `tick.rs:33`), and surfaced through `scheduling_report::alerts` (`status.rs:228-235`). The orchestrator's poll interval is 5 seconds (`loom/src/orchestrator/core/orchestrator.rs:64`).

## Merge State

`MergeState` (`loom/src/git/merge/status.rs:14-26`): Merged, Pending, Conflict, BranchMissing, Unknown.

`check_merge_state` (`status.rs:62-110`) checks git ancestry first (`completed_commit` against the merge point), falling back to `Stage.merge_conflict` only when ancestry cannot be determined.

`MergeStatusReport` (`status.rs:113-123`): merged, pending, conflicts, warnings. `commands/status/merge_status.rs` only re-exports this.

## Retry and Backoff

`loom/src/orchestrator/retry.rs` holds pure logic with no persisted state: `should_auto_retry` (`retry.rs:12`), `calculate_backoff` (exponential, base times 2 to the power of attempt minus one, capped, `retry.rs:35-47`), `is_backoff_elapsed` (`retry.rs:55`), `classify_failure` (`retry.rs:100`).

There is no persisted next-retry timestamp on disk; the backoff window is recomputed each time from `last_failure_at` plus the calculated backoff.

## Attention Rendering (Static View)

Despite the heading (kept because `loom knowledge` cannot rename a section — see concerns.md), this section is now shared by both views. `attention_entries(stages: &[StageSummary]) -> Vec<AttentionEntry>` (`commands/status/render/attention_model.rs:27`) is the single place a stage is judged to need a human; it is re-exported from `commands/status/render/mod.rs:13`. `render/attention.rs` consumes it for the static blocks (`attention.rs:8,18`) and `TuiApp::render` consumes it for the ledger's attention panel (`app.rs:24,288`, feeding `LedgerView.attention`, rendered by `commands/status/ui/tui/ledger/panels.rs`) — the two views cannot diverge on which stages get flagged.

Each block/entry carries a title per status: BLOCKED, MERGE CONFLICT, ACCEPTANCE FAILED, MERGE ERROR, NEEDS REVIEW (`attention.rs:84-88`).

Hints shown per status (`attention.rs:115-119`): a retry hint for Blocked and stages that finished with failures; a merge hint for MergeConflict and MergeBlocked; a human-review hint for NeedsHumanReview, whose review command takes an approve flag, a force-finish flag, or a reject flag with a reason (`attention.rs:45-71,108-111`).

Evidence lines are capped at `MAX_EVIDENCE_LINES=32` (`commands/status/data/sanitize.rs:30`, raised from an original 20 that silently dropped the last line of a full startup-refusal crash — `loom/src/orchestrator/core/crash_classification.rs:210-211` builds up to 21 evidence lines). A cap that bites now pushes an explicit truncation marker rather than staying silent. A cleanup hint (worktree removal) is shown when `cleanup_warning` is set (`attention.rs:171`).

The static execution graph's legend comes from `render_legend` (`render/graph.rs:312-325`), driven by `LEGEND_STATUSES` (`graph.rs:27-41`); the live ledger's on-demand legend overlay is separate (`commands/status/ui/tui/ledger/legend.rs`, toggled by `?`).

## What the Live TUI Renders Today

The live TUI is a responsive TABLE (the "ledger"), one row per stage, built under `loom/src/commands/status/ui/tui/ledger/` (nine modules: `cells`, `columns`, `header`, `layout`, `legend`, `mod`, `panels`, `rows`, `text`). Entry point `ledger::render` is re-exported from `commands/status/ui/tui/ledger/layout.rs:41` via `commands/status/ui/tui/ledger/mod.rs`, and called from `TuiApp::render` at `app.rs:344`.

Eight columns (`ColumnKind`, `commands/status/ui/tui/ledger/mod.rs:44-61`): State, Stage, DependsOn, Models, Activity, Context, Time, Merge, laid out at designed width when `FULL_WIDTH=120` (`ledger/mod.rs:40`) and dropped in priority order as terminal width shrinks. Below `MIN_COLS=64` or `MIN_ROWS=16` (`ledger/mod.rs:36,38`) a notice replaces the dashboard entirely (`commands/status/ui/tui/ledger/layout.rs:41-43`). The MODELS column is 16 cells wide, pinned by the `FULL_WIDTH=120` budget and the drop-order termination proof at 64; content wider than that truncates (e.g. `opus›sonnet+1`).

`layout::budget(height, alerts, attention_entries, activity_len)` (`commands/status/ui/tui/ledger/layout.rs:63`) apportions rows across the alert band, attention panel, activity panel, and the table itself; `render` returns a `RenderOutcome` whose `table_viewport_rows` the app writes back into `graph_state.viewport_height`.

A legend is available on demand (not shown by default): `TuiApp.legend_open: bool`, toggled by `?` (`KeyCode::Char('?') => KeyEventResult::ToggleLegend`, `ui/tui/event_handler.rs:33`), rendered as an overlay listing every state by `commands/status/ui/tui/ledger/legend.rs`.

The footer error line is wired end to end: `TuiApp.last_error` (`app.rs:46`) is set on daemon exit and on `Response::Error`, cleared on every successful `StatusUpdate`, passed through `LedgerView.last_error` and rendered by `panels::render_footer` (`commands/status/ui/tui/ledger/layout.rs:142`, `commands/status/ui/tui/ledger/panels.rs:50-52`).

The header's liveness indicator ("daemon running, tick Ns ago") is computed CLIENT-side from `.loom/work/orchestrator.tick`, not from the daemon broadcast (`app.rs` `refresh_alerts`) — it can keep asserting health while the daemon is failing to push status at all. A daemon-side "skip this broadcast" branch (oversized-payload guard) now sends `Response::Error` instead of silently skipping, specifically so the footer can show the fault; see mistakes.md for why a silent skip was the wrong shape here.

TUI cell padding measures with `Span::width()` (unicode-width aware, `commands/status/ui/tui/ledger/header.rs`'s `text_width()` and `commands/status/ui/tui/ledger/text.rs`), never `chars().count()` — at least one status icon (`⚡` MergeConflict, U+26A1) is East-Asian Wide (one char, two terminal cells), and a char-count pad shifts every later column by one cell on that row.

Colors and icons for `StageStatus` are unchanged from `loom/src/models/stage/types.rs:986-1096` (see "StageStatus — 13 Variants" above); the ledger has no separate per-stage color cycle.

**2026-09-04 correction — a third consumer exists.** `loom status --web [PORT]` (see [web-dashboard.md](web-dashboard.md)) is a second live renderer of the identical `StatusData`/`StageSummary` payload described in this file, alongside the compact view and this TUI ledger. It does not reimplement the Rust-side derivation: it reuses `render::attention_entries`, `render::failure_label`, `scheduling_report::alerts`, `tick::read`, and `data::collect_status_data` directly, and ports only the TUI's display semantics (icons, labels, legend text) to TypeScript in `web/src/lib/format.ts`, pinned against this file's serde output by `loom/src/commands/status/web/model_tests.rs`.

The build version reaches all three surfaces from one pair of consts, `crate::version::VERSION` and `crate::version::LABEL` (`loom/src/version/mod.rs`, both derived from `build.rs`'s `LOOM_VERSION`). The static view passes `LABEL` as the logo subtitle (`commands/status.rs:198`); the TUI right-aligns it on the header's merge line and drops it when the width cannot hold the counts plus a space plus the label (`ledger/header.rs`'s `merge_line`); the web dashboard reads it off the wire as `WebSnapshot.version` (`commands/status/web/model.rs`, mirrored in `web/src/api/schema.ts`) and prints it beside the plan name in `web/src/components/header.tsx`. The web fixture pair (`web/src/api/fixtures/snapshot.json` and `fixture_snapshot()` in `model_tests.rs`) holds the same synthetic `0.0.0-fixture` on both sides, so that comparison never tracks a release; `tests/pure.rs`'s `poll_files_once_carries_the_degrade_notice` is what pins the real publish path to `crate::version::VERSION`.

## What Does Not Exist on the Status Path

Stated explicitly because these are natural things to expect and go looking for:

- **Subagent state.** Subagent STATE (Done/ToolWait/Generating classification from `loom subagents`, `classify.rs:83-99`) is still never read by the status path, and `peak_resident_tokens`/`request_count` remain outside the status models. What the status path DOES now read from `.loom/work/subagents` is the spawn ledger (`spawns.jsonl`) and the codex ledger (`codex.jsonl`), for `StageSummary.execution_models` — distinct execution-model display names in first-seen order, empty until a subagent spawns (see "Execution-Model Ledgers" below). Both ledgers are written by PreToolUse hooks running outside the stage's Bash sandbox, so a worktree stage session records normally and `execution_models` is empty only until the first subagent spawns.
- **Per-criterion acceptance results.** `CriterionResult`/`AcceptanceResult` (`loom/src/verify/criteria/result.rs:7-20,87-95`) are never serialized to disk; only `failure_info.evidence` survives a run. The pass cache (`verify/criteria/cache.rs:93,369-386`) is keyed by command digest and is not read by status.
- **Historical handoff counts.** Status only checks that the handoffs directory exists (`diagnostics.rs:11,37`); compact mode counts stages currently in `NeedsHandoff`, not historical handoff events (`render/compact.rs:30-37`).
- **A persisted orchestrator event log.** There is none.
- **`plan_id`, the plan's source path, the IN_PROGRESS/DONE filename prefix, or a run start time** on `StatusData`.
- **A `RemoteControl` struct on the status path.** Only `[remote_control]` and `[terminal]` config sections exist in `.work/config.toml`.

## Execution-Model Ledgers

`StageSummary.execution_models` is populated by `execution_models_for_stage(work_dir, stage_id)` (`commands/status/data/execution_models.rs:33-54`), which reads two append-only ledgers under `.loom/work/subagents/<stage_id>/`: `spawns.jsonl` (written by the `hooks/spawn-guard.sh` PreToolUse hook, running outside the stage's Bash sandbox) and `codex.jsonl` (written by the `hooks/codex-forward-guard.sh` PreToolUse hook, at the moment it authorizes a forward, also outside that sandbox — a blocked command records nothing). `codex-forward.sh` used to write that row itself, from INSIDE the stage's Bash sandbox, where the append through the worktree's `.loom/work` symlink is denied and silently swallowed, so a codex run left no trace at all in the MODELS column — the forwarder's own `spawns.jsonl` row carries the shim's sonnet tier and is deliberately skipped. Each ledger is capped at `MAX_LEDGER_BYTES=256*1024` (`commands/status/data/execution_models.rs:23`) and the result at `MAX_EXECUTION_MODELS=8` distinct names (`commands/status/data/execution_models.rs:30`).

Row values are untrusted: the `model` field is the caller-controlled `.tool_input.model` written verbatim by the hook. `normalize_model`/`strip_date_suffix` (`commands/status/data/execution_models.rs:118-140`) strip a `claude-` prefix and a trailing `-YYYYMMDD` stamp using `rsplit_once('-')` (byte-index slicing panicked on a multi-byte model name before this fix). Flattening (`context::untrusted::inline_safe`) runs BEFORE dedup/normalize, not after — deduping on the raw ledger string let `sonnet` and `sonnet<zero-width char>` count as two distinct models.

`valid_stage_id` guards the ledger path on both sides of the read/write boundary, but at different strengths: the Rust reader (`commands/status/data/sanitize.rs:84`) explicitly rejects `.` and `..`; the shell writers (`hooks/spawn-guard.sh:309`, `hooks/codex-forward.sh:43`) are character-class allowlists (`[A-Za-z0-9._-]`) that accept `.` and `..` because both are made only of allowed characters — a stage id of `..` resolves the ledger directory to the work dir itself. See concerns.md for the outstanding shared-helper cleanup.
