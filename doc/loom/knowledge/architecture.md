# Architecture

> High-level component relationships, data flow, and module dependencies.
>
> **Related files:** [patterns.md](patterns.md) for design patterns, [entry-points.md](entry-points.md) for code navigation, [conventions.md](conventions.md) for coding standards.

## Project Overview

Loom is a Rust CLI (~15K lines) for orchestrating parallel Claude Code sessions across git worktrees. It enables concurrent task execution with automatic crash recovery, context handoffs, and progressive merging.

## Directory Structure

Full `loom/src/` module tree with one-line responsibilities per directory, plus the `.work/`
state layout and the repo-root asset directories (`hooks/`, `agents/`, `skills/`, `codex/`).

→ [Directory Structure](architecture/directory-structure.md)

## Core Abstractions

The load-bearing types — `ExecutionGraph`, `Stage`, `Session`, `Orchestrator`, `TerminalBackend`,
`KnowledgeDir` — together with the end-to-end data flow from plan parse to merge, and the
file-ownership rules that say which process may write which `.work/` file.

→ [Core Abstractions, Data Flow & File Ownership](architecture/core-abstractions.md)

## Worktree Isolation (4-Layer Defense)

1. **Git layer** -- Separate worktrees at `.worktrees/<stage-id>/` with branch `loom/<stage-id>`. Symlinks: `.work` -> shared state, `.claude/CLAUDE.md` -> instructions, root `CLAUDE.md` -> project guidance.
2. **Sandbox layer** -- `MergedSandboxConfig` (`sandbox/config.rs`) generates `settings.local.json` with filesystem deny/allow policy, network domains, and fail-closed sandbox availability. Plan-configured `excluded_commands` are rejected; generated settings do not grant broad executable exemptions. Knowledge writes use the narrow Loom control path rather than direct file edits.
3. **Signal layer** -- Four stage-type-specific stable prefix generators in cache.rs (standard, knowledge, integration-verify, knowledge-distill). Include isolation rules and subagent restrictions.
4. **Hook layer** -- commit-guard.sh blocks exit without commit. commit-filter.sh blocks subagent git operations and subagent-verify-guard.sh blocks subagent full-suite verification, both gated on `loom_is_subagent()` (live-ancestor `LOOM_MAIN_AGENT_PID`, then a payload-first classification via `loom_payload_agent_verdict` — the intervening-Claude-process walk is only the fallback — not a PPID comparison).

## Layering Violations (Known Issues)

Correct dependency direction: commands/ -> orchestrator/ -> models/ (top), daemon/ / git/ / plan/ (middle), fs/ (bottom).

Known violations (all four are pre-existing, none introduced by the context work):

- daemon imports commands (mark_plan_done_if_all_merged) -- fix: move to fs/plan_lifecycle.rs
- orchestrator imports commands (check_merge_state) -- fix: move to git/merge/status.rs
- git/worktree imports orchestrator (hook config) -- fix: extract hooks/ as top-level
- models imports plan/schema (WiringCheck, StageType) -- fix: move types to models/

### The newer modules are clean (verified 2026-08-17)

The list above predates `context/`, `telemetry/` and `process/`, so its silence about them was
ambiguous rather than reassuring. Verified with
`rg '^use crate::[a-z_]+' loom/src/context`:

- **`context`** imports only `crate::context`, `crate::fs`, `crate::language`, `crate::models`
  and `crate::git`. **No upward edge** to `orchestrator`, `commands` or `daemon`. The single
  `git` edge is deliberate — `git::runner::run_git_checked` at
  `context/refresh/source_graph.rs:30`, needed to list tracked files and judge tree
  cleanliness — and it points downward, so it is not a violation. Record it rather than
  rediscovering it.
- **`telemetry`** is a leaf: one module, no `crate::` imports beyond its own serde types. The
  orchestrator calls into it (`orchestrator/core/stage_telemetry.rs`), never the reverse.
- **`process`** holds the env allowlist and is consumed by both `verify` and the terminal
  spawner without importing either.

The rule to preserve: the orchestrator calls into `context`, never the other way around. A
`use crate::orchestrator` appearing anywhere under `loom/src/context/` is a regression, and the
one-line check above is the way to catch it.

(Note that a match for `crate::external` under `context/extract/rust.rs:132` is inside a golden
test fixture string, not a real import.)

## Context Budget Enforcement

Stages define `context_ceiling_tokens: Option<u32>` — an ABSOLUTE resident-token ceiling (validated `>= MIN_CONTEXT_CEILING_TOKENS`, 60,000). A plan still writing the retired percentage field `context_budget` is REJECTED (`removed_context_budget` serde trap in `plan/schema/types.rs`, checked in `validation.rs`). Resolution order: `stage.context_ceiling_tokens` -> `.work/config.toml [context] ceiling_tokens` -> `DEFAULT_CONTEXT_CEILING_TOKENS` (150,000); subagents default to `DEFAULT_SUBAGENT_CEILING_TOKENS` (120,000). The one resolver is `fs::work_dir::resolve_context_ceiling_tokens(work_dir, stage_ceiling)`; `monitor/detection.rs` keeps its own copy of the same order because it holds a pre-read `ContextConfig` and must not touch the filesystem per tick.

`orchestrator/monitor/context.rs::context_health(tokens, ceiling)` bands a session as a fraction of its resolved ceiling: Green `<60%`, Yellow `60-90%`, Red `>=90%` (a ceiling of 0 yields Green). `BudgetExceeded` is the daemon's 1.25x-over-ceiling backstop (`DAEMON_CEILING_MULTIPLIER`), not the handoff trigger — and it now KILLS the session it hands off before re-queueing (see [context-ceiling.md](architecture/context-ceiling.md) for why an unconditional re-queue was a double-spawn bug). The actual handoff trigger is the `PostToolUse` hook reading the resident-token count out of the transcript.

Three independent enforcement thresholds fire off this one ceiling — 1.0x (the `PostToolUse` hook instruction), 1.25x (the daemon kill+re-queue), and 1.5x (Claude Code's own native `CLAUDE_CODE_AUTO_COMPACT_WINDOW`) — plus `hooks/pre-compact.sh`'s block-then-allow pattern as a threshold-independent last resort. Full detail: [context-ceiling.md](architecture/context-ceiling.md).

## Security Model

- **ID validation**: Alphanumeric + dash/underscore, max 128 chars, no path traversal (validation.rs)
- **Acceptance criteria**: Runs arbitrary shell commands (trusted model)
- **Socket**: Mode 0o600 (owner only), max 100 connections, 10MB message limit, Unix only
- **Self-update**: minisign signature verification for binaries; `agents.zip`, `skills.zip`, and `CLAUDE.md.template` ARE SHA256-verified against the release checksums asset (self-update refuses to install an asset with no checksum entry). The asset-name mismatch an earlier version of this bullet described is fixed: `commands/self_update/mod.rs::checksum_asset` fetches `SHA256SUMS.txt`, the name the release workflow publishes (corrected 2026-09-03; `PLAN-embed-assets-and-complete-self-update` removes the config-asset download path entirely)
- **Shell escaping**: escape_shell_single_quote(), escape_applescript_string() in emulator.rs
- **permission_mode field** (`SandboxConfig` / `StageSandboxConfig`): Resolves as stage > plan > stage-type default. Default by stage type: ALL four stage types → `auto` (Knowledge, KnowledgeDistill, Standard, IntegrationVerify) — loom stages run autonomously with no human to answer prompts, so the agent auto-accepts actions its heuristics deem safe; the sandbox deny/allow rules are the safety boundary. Override to a stricter mode (`accept-edits`, `plan`) at plan or stage level if needed. **Delivery:** the resolved mode is passed as the `--permission-mode` CLI flag by `build_claude_command` at spawn — NOT via `permissions.defaultMode` in the worktree's `settings.local.json`, which Claude Code v2.1.142+ ignores for `auto` (a repo cannot grant itself auto mode; only the CLI flag or user/managed settings are honored). Auto mode itself requires a supporting account/model (Opus 4.6+/Sonnet 4.6+); loom's job is only to request it correctly. See entry-points.md §2–3 and mistakes.md.

## Merge Lock (git/merge/lock.rs)

`MergeLock` serializes loom-driven merges with an exclusive OS advisory lock (`fs2::try_lock_exclusive`) on the stable `.work/merge.lock` inode. The file is created once and never unlinked; the holder's pid and timestamp are written into it for diagnosis only. `acquire` polls every 100 ms up to the caller's timeout (30 s from `merge_stage` and the probe). Release is by `Drop` or process exit, so there is no stale-lock reclamation and a pid left in the file after a merge is not a held lock. An earlier version of this section named `progressive_merge/lock.rs` and a five-minute stale sweep; neither exists.

## Merge Flow (post-completion auto-merge) [DETAILED]

The daemon writes `Completed` before any merge runs; the first merge attempt comes from the one-shot loop at the end of `sync_graph_with_stage_files`, not from the `StageCompleted` event, which arrives later in the same tick and only cleans up. Outcomes: conflicts spawn a resolver (`MergeConflict`); any other failure is forced to `MergeBlocked` with the git error in `failure_info`; an empty branch goes to `NeedsHumanReview`; only auto-merge-disabled stages rest at `Completed + !merged`. `loom stage merge <id>`, run from the worktree, accepts all three non-merged states.

→ [Merge Flow](architecture/merge-flow.md)

## Skills Module (loom/src/skills/)

Loads skill metadata from SKILL.md files across TWO roots — `~/.claude/skills/` (core skills) and `~/.claude/loom-skill-catalog/` (the other ~53 catalogued skills, split out of `~/.claude/skills` to keep the primary directory small and to avoid tripping `hooks/read-guard.sh`'s 400-line rules on oversized skills — full rationale in [skill-catalog.md](architecture/skill-catalog.md)) — builds an inverted index of trigger keywords, matches stage descriptions. Components: `types.rs` (SkillMetadata, SkillMatch), `matcher.rs` (keyword matching, phrase=2pts, word=1pt, threshold 2.0), `index.rs` (SkillIndex, load_from_directory, match_skills — visibility of `add_skill`/`parse_skill_file` widened to `pub(super)` for the catalog loader, otherwise unchanged), `index_catalog.rs` (the compiled-in core manifest via `include_str!` of `skills/core-skills.txt`, the two-root loader `load_with_catalog`, and `skill_invocation()` which renders the correct invocation form — bare `/loom-<name>` for core skills, `Skill(skill="loom-skills", args="<name>")` for catalogued ones), `install_layout.rs` (reads `~/.claude/loom-install.toml` and re-places skills after a self-update). Up to 5 skill recommendations embedded in agent signals.

A catalogued `SKILL.md` is loaded via the Read tool, not the Skill tool, so both skill roots need an exemption in every Read-class `PreToolUse` hook (`hooks/worktree-file-guard.sh` and `hooks/_read_discipline.sh`) or all catalogued skills become unreachable from a worktree stage session — see [skill-catalog.md](architecture/skill-catalog.md).

## Diagnosis Module (loom/src/diagnosis/)

Analyzes failed/blocked stages. DiagnosisContext collects crash_report, log_tail, git_status, git_diff. Generates diagnostic signal for Claude Code investigation. CLI: `loom diagnose <stage-id>`.

## Map Module (loom/src/map/)

Automated codebase analysis that populates knowledge files. Detectors: project type, dependencies, entry points, structure, conventions, concerns. Features: --deep (3-level depth + concerns), --focus (filter entry points), --overwrite. CLI: `loom map`.

## Handoff System

Fully functional handoff chain:

1. **loom handoff create** -- CLI command accepting --stage, --session, --trigger, --message flags
2. **pre-compact.sh** -- Two-phase block-then-allow pattern. Phase 1 blocks compaction (exit 2), creates handoff. Phase 2 allows compaction (exits 0). No longer creates a recovery marker file.
3. **session-end.sh** -- Uses glob `*-${LOOM_STAGE_ID}.md` for stage file lookup (handles depth prefixes)
4. **Signals** -- cache.rs append_common_footer() adds compaction recovery instructions to ALL signal types
5. **session-start.sh** -- On SessionStart with `.source == "compact"` or `"resume"`, emits hookSpecificOutput additionalContext re-anchor pointer so the agent finds its signal file after compaction

## macOS Terminal Detection Priority

1. LOOM_TERMINAL env var (explicit override)
2. TERMINAL env var (user preference)
3. Parent process detection (walks process tree up to 10 levels via ps)
4. Cross-platform binary check (ghostty, kitty, alacritty, wezterm via which)
5. macOS native apps (/Applications/Ghostty.app, /Applications/iTerm.app, Terminal.app fallback)

Note: $TERM_PROGRAM is NOT checked.

## find_claude_path() (src/claude.rs)

Shared binary resolution: `which::which("claude")` -> `~/.claude/local/claude` -> `~/.local/bin/claude` -> `~/.cargo/bin/claude` -> `/usr/local/bin/claude` -> `/opt/homebrew/bin/claude`.

## KnowledgeDir API (fs/knowledge/dir.rs)

KnowledgeFile enum: Architecture, EntryPoints, Patterns, Conventions, Mistakes, Stack, Concerns. Core methods: new(root), exists(), initialize(), read(file), read_all(), append(file, content), generate_summary(), list_files().

## Adding New Plan Fields Checklist

1. Add to StageDefinition (plan/schema/types.rs) with serde defaults
2. Add validation in validation.rs
3. Add to Stage model (models/stage/types.rs) with serde defaults
4. Copy in the canonical `Stage::from_definition()` builder (`models/stage/methods.rs`); init delegates to it
5. If goal-check: update has_any_goal_checks() in BOTH StageDefinition and Stage
6. If verification: add verify function in verify/goal_backward/ and call from run_goal_backward_verification()
7. Check ALL test files constructing Stage directly (src/ AND tests/ directories)
8. If the field reaches the agent: copy it onto `EmbeddedContext` (orchestrator/signals/types.rs) and
   emit it from BOTH format/sections.rs AND recovery_format.rs — the recovery signal embeds only the
   stable prefix, so a gated section missing there vanishes on any retry
9. Add the two backwards-compat tests (plan YAML without the key; legacy `.work/stages/*.md` without
   the key) — see [Additive Schema Fields](conventions.md); `#[serde(default)]` is the only migration
10. If it is user-facing, add a row to the README Stage Fields table

## Goal-Backward Verification (verify/goal_backward/)

Four verification layers for standard stages. **`truths` is NOT one of them** — it was removed from goal-backward and merged into acceptance. A duplicate section listing `truths` as a goal-backward layer was deleted from this file on 2026-07-30; if you see that claim anywhere else, it is stale.

- **artifacts** -- Files must exist with real implementation (stub detection: TODO, FIXME, unimplemented\!, todo\!)
- **wiring** -- Regex patterns verifying code connections in source files
- **wiring_tests** -- Runtime command-based integration verification
- **dead_code_check** -- Command + pattern detection for unused code

Acceptance criteria (verify/criteria/runner.rs) now handle both:

- **Simple** -- Plain shell command, 5min timeout, exit 0 = pass
- **Extended** -- TruthCheck struct with stdout_contains, stderr_empty, exit_code, 30s timeout

Returns: GoalBackwardResult::Passed | GapsFound | HumanNeeded. Storage: `.work/verifications/<stage-id>.json`.

Note: truths.rs module and verify_truth_checks() are retained for before_stage/after_stage verification (pre/post conditions), NOT for goal-backward.

## Per-Worktree Gitignore for settings.local.json

After worktree creation, `.claude/settings.local.json` is appended (idempotently) to `<worktree>/.git/info/exclude`. Uses per-worktree exclude to avoid polluting the repo's `.gitignore`.

- Standard/IntegrationVerify/KnowledgeDistill: append to `<worktree>/.git/info/exclude`
- Knowledge stages: append to main repo's `.git/info/exclude` (no worktree created)
- The per-worktree exclude file lives at `<worktree>/.git/info/exclude` — NOT at `<worktree-dir>/.git/info/exclude` (the latter is a FILE pointing at the real gitdir, not a directory; the real exclude is at `<repo>/.git/worktrees/<stage-id>/info/exclude`)

## Claude Code Worktree Isolation Disabled in Generated Settings

Loom owns the per-stage git worktree, so it disables Claude Code's _own_ worktree
isolation (`worktree.bgIsolation`) in every settings file it generates. Claude
Code's default (`"worktree"`) blocks Edit/Write in the checkout until
`EnterWorktree`, which would push subagents into nested worktrees on top of loom's
— leaving stray branches and tangled checkouts. Loom emits `"none"` so subagents
edit the loom worktree directly (Claude Code v2.1.143+; older versions ignore it).

Two write sites, both targeting `settings.local.json` (never the committed
`settings.json`, to avoid imposing on non-loom teammates):

- **Worktree stage sessions** — `sandbox/settings.rs:generate_settings_json()`
  emits a top-level `"worktree": { "bgIsolation": "none" }` block. Survives the
  `merge_existing_permissions()` step, which only touches `permissions.*`.
- **Main-repo sessions** (knowledge stages, interactive) —
  `fs/permissions/settings.rs:ensure_loom_hooks_local()` sets it idempotently
  alongside the agent-teams env var.

## Hook System Architecture (hooks/)

The `hooks/` scripts, how they are embedded and installed, the SessionStart
`hookSpecificOutput` contract, and the enforcement layers that keep subagents inside their
lane (`commit-filter.sh`, `subagent-verify-guard.sh`, the worktree guards).

→ [Hook System](architecture/hook-system.md)

## Monitor Subsystem (orchestrator/monitor/)

Full file list:

- `core.rs` — `Monitor` struct, `poll()` API, stage/session loading
- `config.rs` — `MonitorConfig` (work_dir, hung_timeout, etc.)
- `detection.rs` — `Detection` struct: `detect_stage_changes()`, `detect_session_changes()`, `detect_heartbeat_events()`
- `events.rs` — `MonitorEvent` enum (stage/session/heartbeat event variants)
- `failure_tracking.rs` — Consecutive failure escalation logic
- `handlers.rs` — `Handlers` struct: handoff/crash-report generation; holds optional `LivenessService`
- `heartbeat.rs` — `HeartbeatWatcher` with 300s hung timeout
- `context.rs` — `context_health(tokens, ceiling)` bands an absolute token count as a fraction of its resolved ceiling: Green `<60%`, Yellow `60-90%`, Red `>=90%` (see "Context Budget Enforcement" above)
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

## Status Data Model [DETAILED]

Where every field `loom status` can show comes from: static/compact reads `StatusData` off
`collect_status_data`, while `--live` subscribes to the daemon's push channel instead of polling
it, and the two paths carry different fields — the live `StageInfo` payload is strictly narrower
than the static `StageSummary` (no context tokens, no heartbeat/activity, no retry count). Covers
the 13 `StageStatus` variants, the `Stage`/`Session`/`Heartbeat` fields worth surfacing, merge and
retry state, and what does not exist on the status path today (subagent state, cost/request
counts, per-criterion acceptance results).

→ [Status Data Model](architecture/status-data-model.md)

## Status Command Architecture (commands/status/)

The status command is organized as a sub-module tree:

```text
commands/status.rs          # Entry: dispatches to 3 modes + validate/doctor
commands/status/
  data.rs                   # collect_status_data() → StatusData struct
  render/                   # Pure render functions (progress, graph, merge, compact)
  ui/                       # TUI backed by daemon IPC subscription
  diagnostics.rs            # Workspace integrity checks
  display.rs                # count_files() helper
  merge_status.rs           # Merge section data
  validation.rs             # Markdown + cross-reference validation
```

**Data flow (static mode):** `collect_status_data()` loads plan name, stage list (with status/context), session list, merge state, and progress counts into a single `StatusData`. Renderers receive `StatusData` and write to `impl Write`.

**TUI mode:** `ui::run_tui(work_path)` subscribes to the daemon's Unix socket (`orchestrator.sock`) and re-renders on each update. Requires daemon running; errors with hint if not.

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

## Stage State Machine — 13 Variants

`StageStatus` (`models/stage/types.rs`) has **13** variants. `NeedsAdjudication` shipped and is wired into `transitions.rs`, the graph renderer, and the daemon dispute handler — an earlier version of this section described it as a proposed 13th addition to a 12-variant enum.

```text
WaitingForDeps → Queued → Executing → Completed/Skipped (terminal)
                  |           |
                  v           +→ Blocked, NeedsHandoff, WaitingForInput,
               Skipped           MergeConflict, CompletedWithFailures,
                                 MergeBlocked, NeedsHumanReview,
                                 NeedsAdjudication
```

Transitions FROM `NeedsAdjudication` (`transitions.rs`) — note it can loop to itself:

- `Queued` — verdict applied, stage re-queued
- `NeedsAdjudication` — evidence loop (another round on the same dispute)
- `NeedsHumanReview` — by design, a **`Reject` verdict**: the adjudicator upheld the criterion and ruled the implementation wrong, while the agent disputed it as impossible. Neither side can move, so this is the one outcome a human is needed for. Also reached when a bound is exhausted: the evidence loop (`MAX_EVIDENCE_ROUNDS`, 5), the per-stage amendment cap (`max_amendments_per_stage`, default 10), or the adjudication respawn budget (`MAX_ADJUDICATION_ATTEMPTS`, 3). An earlier version named `ANTHROPIC_API_KEY not set` here; that gate no longer exists — see conventions.md § Adjudicator Transport Convention

`CompletedWithFailures` also transitions into `NeedsAdjudication` (dispute filed after a failed completion) and into `NeedsHumanReview` (budget escalation).

## Dispute Directory Structure (Shipped)

`.work/disputes/<stage_id>/<n>/` — per-dispute directory (numbered from 1):

| File             | Authority                            | Contents                                                                                                    |
| ---------------- | ------------------------------------ | ----------------------------------------------------------------------------------------------------------- |
| `request.md`     | Agent-writable (via daemon RPC)      | id, stage_id, criterion_index, reason, evidence_commit, failure_output, fix_attempts_at_dispute, created_at |
| `verdict.md`     | Daemon-only (worker thread writes)   | verdict, citations, reasoning, plan_patch, adjudicator_attempt_count, model                                 |
| `applied.marker` | Daemon-only (zero-byte, idempotency) | —                                                                                                           |
| `attempts`       | Daemon-only (respawn budget)         | count spent when an adjudication job is handed out; cap 3. Replaced the `.inflight` staleness marker, which is gone with the worker thread |

Request.md is written by the daemon handler on behalf of the agent's RPC call. Trust boundary: same pattern as `loom memory note`.

## Plan Versioning / Runtime Amendment (Shipped — `plan/amendment.rs`)

Runtime plan amendment exists and is reachable from an `Accept` adjudication verdict. `.work/plan_versions/` is its audit trail:

- `.work/plan_versions/.lock` — `flock` (serializes amendments)
- `.work/plan_versions/<n>.md` — snapshot of full plan content after amendment n
- `.work/plan_versions/audit.md` — O_APPEND atomic rows (amendment log)

**The write set is TWO files, not one — this is the trap.** Under the lock, an amendment writes the snapshot, appends the audit row, replaces the live plan file via `safe_replace_outside_workdir`, **and rewrites the target stage's `.work/stages/<n>-<id>.md`**. The last step is not optional: `plan/graph/loader.rs` prefers `.work/stages/` over the plan file, so amending only the plan would leave `sync_graph_with_stage_files` serving the old criteria forever. (An earlier version of this section described a 6-step flow ending at "atomic rename plan file", omitting the stage-file write.)

The proposed value is deserialized into the **real** `AcceptanceCriterion` / `WiringCheck` types before anything is written, so a malformed patch fails fast rather than corrupting the plan. A per-stage cap (default 3, `loom.adjudication.max_amendments_per_stage`) bounds runaway adjudication.

**Recovery** — `verify_plan_versions_consistency()`, called from orchestrator startup, handles three divergences:

| On disk                                 | Action                                                             |
| --------------------------------------- | ------------------------------------------------------------------ |
| Snapshot written, audit row missing     | Orphaned snapshot — removed so the next amendment can claim the id |
| Audit row appended, plan file still old | Re-apply the snapshot to plan + stage file (catch-up commit)       |
| Plan + audit in sync, stage file stale  | Re-apply just the stage-file update                                |

## Plan Immutability Invariant (Narrowed, Not Removed)

Plans are loaded ONCE at daemon startup via `build_execution_graph()` → `ExecutionGraph::build()`; there is no general reload mechanism, and the in-memory `graph: ExecutionGraph` on `Orchestrator` holds all state. Amendment is the one sanctioned mutation path and it is deliberately narrow: **only the `acceptance` and `wiring` arrays on a single stage**. Stage IDs, dependencies, `working_dir`, DAG topology, and plan structure are never amendable — so the graph the daemon loaded at startup stays topologically valid for the life of the run.

## Remote Control Module (loom/src/remote_control.rs)

Capability detection, preflight, and resolution for driving external agent binaries; the
permission-mode plumbing that decides how a spawned session is allowed to act.

→ [Remote Control Module](architecture/remote-control.md)

## Signal Generation Pipeline (orchestrator/signals/) [DETAILED]

How a stage signal is assembled: the stable-prefix cache keyed by stage type, the shared
`append_*` helpers that compose each block, per-stage-type prefixes, and the soft-signal
escalation path. This is the runtime channel for agent doctrine — change it and every
running stage changes.

→ [Signal Generation Pipeline](architecture/signal-generation.md)

## before_stage / after_stage / code_review Schema Fields — Execution Status

**Status as of 2026-06-15 (verified against stage_executor.rs:219-256, plan/schema/types.rs:261, and orchestrator/signals/generate.rs):**

| Field          | Schema Type                | Stored on Stage            | Executed   | Where                                             |
| -------------- | -------------------------- | -------------------------- | ---------- | ------------------------------------------------- |
| `before_stage` | `Vec<TruthCheck>`          | ✅ Yes (plan_setup.rs:280) | ✅ Yes     | stage_executor.rs:220-256 (pre-spawn)             |
| `after_stage`  | `Vec<TruthCheck>`          | ✅ Yes (plan_setup.rs:281) | ✅ Yes     | commands/stage/complete.rs:847-866                |
| `code_review`  | `Option<CodeReviewConfig>` | ✅ Yes                    | ✅ Signal  | signals/generate.rs renders it for IV signals     |

**`before_stage` execution (`stage_executor.rs::before_stage_gate_passed`):**

- Runs after worktree creation, BEFORE session spawn
- **Gated on a pristine workspace.** `verify::before_after::find_prior_stage_work(stage_branch, base_branch, repo_root, worktree_path)` runs first; if it finds commits on `loom/<stage-id>` beyond the resolved base, or non-scaffold changes in the worktree, the checks are SKIPPED (logged at `info`) and the spawn proceeds. `before_stage` is a delta-proof ("the feature does not exist yet"), which is only meaningful on the first attempt — re-running it on a re-spawn (orphan recovery, `loom stage retry`, crash retry) fails on the previous attempt's own work and blocks the stage before any session exists to finish it (unrecoverable loop; see mistakes.md 2026-07-27)
- Loom's own worktree scaffolding (`.work`, `.claude/`, root `CLAUDE.md`) is discounted via `git::worktree::is_worktree_scaffold_path` — it is present from the first spawn, and in repos that don't gitignore it, counting it would disable the gate entirely
- Calls `crate::verify::before_after::run_before_stage_checks(&stage.before_stage, &check_dir)`
- On failure gaps: stage → `Blocked` (FailureType::TestFailure), session NOT spawned. `TestFailure` is not auto-retryable (`should_auto_retry`), so the stage rests Blocked until an operator runs `loom stage retry`
- On errors (infrastructure): prints warning, continues anyway (advisory)
- TruthCheck timeout: 30 seconds (hardcoded in truths.rs:13)

**`after_stage` execution (commands/stage/complete.rs:847-866):**

- Runs during `loom stage complete`, AFTER acceptance criteria pass
- On failure: stage stays Executing, no merge, agent must fix and re-run

**`code_review` — PERSISTED AND WIRED FOR SIGNAL GENERATION:**

- Parsed by serde at schema level and copied by `Stage::from_definition()` onto the runtime `Stage`
- `orchestrator/signals/generate.rs` reads the persisted runtime field; it does not reparse the plan
- `render_review_dimensions()` emits a `## Review Dimensions` checkbox section in IV signals, honoring `require_all`
- It remains agent guidance rather than an acceptance or goal-backward verification primitive

## load_stage_definition_from_plan — Centralized Plan Lookup

Centralized in `plan/parser/mod.rs` (re-exported via `plan/mod.rs`). Previously lived in `commands/verify.rs`.

**Signature:** `load_stage_definition_from_plan(stage_id, work_dir) -> Result<Option<StageDefinition>>`

Reads `.work/config.toml` for plan path, calls `resolve_source_path()`, calls `parse_plan()`, finds stage by ID. Used by:

- `commands/stage/complete.rs` — after-stage execution

**Why plan/ layer:** completion needs the authoritative plan definition for checks that are intentionally evaluated from the active plan. Runtime policy copied onto `Stage` should not reparse the plan at its consumers.

## `loom pressure` Command (Plan Pressure-Testing Driver)

`loom pressure <plan> [--rounds N=2] [--claude-model M] [--codex-model M] [--address-model M] [--dry-run]` (loom/src/commands/pressure/mod.rs) is a standalone, **synchronous foreground** driver that hardens a plan by combining two external agents. It is a second execution model distinct from the daemon/worktree orchestrator: it runs in the user's repo — NOT a worktree, NOT a background daemon, NOT a terminal-spawn.

Per round (default 2): delete the codex report → run **Claude `/pressure` (foreground) and Codex `$pressure` (background) CONCURRENTLY** → once both finish, run Claude `/address <plan>` (folds Codex's written review back into the plan). One final report deletion after all rounds. Because the two pressure-tests run in parallel, Codex reviews the _pre-edit_ plan while Claude edits it — a more independent perspective; `/address` reconciles both afterward.

**Billing/TTY constraint (load-bearing):** Claude Code enters its non-interactive `-p` path — which can bill against pay-per-token API credits instead of the claude.ai subscription — whenever **stdout is not a TTY** (piped/redirected), even without `-p` (confirmed in `claude --help`). So Claude's stdout MUST stay the real terminal: `/pressure` and `/address` run in the **foreground** (interactive, subscription-billed, visible), and CANNOT be captured/backgrounded. Codex — which has separate auth and floods stdout with a verbose event stream — is the one backgrounded, with stdout+stderr captured to a temp log (`$TMPDIR/loom-pressure-codex-<pid>.log`); its tail is printed on non-clean exit.

**Auto-exit without `-p` (mirrors the daemon):** interactive Claude never exits on its own after a slash command, and EOF on stdin makes the REPL quit _before_ the work finishes. So the driver replicates how the daemon ends a session (`event_handler.rs` → `NativeBackend::kill_session` → SIGTERM once the stage completes): it injects a completion instruction via `--append-system-prompt` telling the agent to `touch <marker>` as its FINAL action, polls for that marker file, then SIGTERMs (escalating to SIGKILL after a grace period) the now-idle foreground session. If the marker never appears the user can still exit manually (graceful fallback = old behavior). Codex is non-interactive and exits on its own.

Children run with `current_dir(repo_root)` (resolved via `git rev-parse --show-toplevel`), so the plan argument handed to them is **repo-relative** (e.g. `doc/plans/PLAN-foo.md`), never cwd-relative. Claude argv: `--permission-mode auto --model <resolved> --append-system-prompt <marker-instruction> <slash>` with `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1` and stdin/stdout/stderr inherited. Codex argv: `exec --sandbox workspace-write -m <resolved> -c model_reasoning_effort=xhigh -C <repo_root> <skill>` with stdin `/dev/null` and stdout/stderr → the log file. **All three models are operator-selectable** and resolved per slot by `PressureModels::resolve` (`pressure/models.rs`): CLI flag → `~/.loom/config.toml`'s `pressure.{claude,codex,address}_model` → built-in default (`opus`/`gpt-5.6-sol`/`opus`, the `DEFAULT_PRESSURE_*_MODEL` consts in `claude.rs`/`codex.rs`). The accepted vocabularies are the `CLAUDE_MODELS`/`CODEX_MODELS` slices in those same two modules — one source of truth shared by the clap `PossibleValuesParser`, the user-config registry's `ValueKind::Enum`, and the shell completions. The `/pressure` and `/address` slots are independent, so the two Claude steps can run on different models. Reasoning effort stays pinned (`CODEX_REASONING_EFFORT` in pressure/spawn.rs — codex has no dedicated effort flag, hence the `-c` config override). Both the dry-run preview and the real run print the three resolved models in their header. NOTE: Codex has been observed printing a non-fatal `worker transport error / authorization required` warning at startup even while logged in and continuing to work — it is codex-side, not a loom bug; the captured log now keeps it off the terminal. Because codex is otherwise invisible (no output; the spinner shows only when codex outlives Claude; the report is deleted as final cleanup), the driver prints status lines: `→ codex review started in background (log: …)` at spawn, and `✓ codex review written → <report>` (or a warning if codex exited cleanly without writing the report) after it finishes — without these, a codex run that finished before Claude was repeatedly mistaken for never having started (see mistakes.md).

Supporting pieces:

- `loom/src/codex.rs` — `find_codex_path()` binary resolver, mirrors `claude::find_claude_path` (which::which, then candidate install paths favoring ~/.bun/bin; spawned children may not inherit PATH so resolve eagerly).
- Vendored agent assets (installed LOCALLY by install.sh): `commands/{pressure,address,distill}.md` → `~/.claude/commands/`; `codex/skills/pressure/SKILL.md` → `~/.codex/skills/pressure/`.
- Wiring: `Commands::Pressure(PressureArgs)` — a TUPLE variant in cli/types.rs whose args live in `cli/types_pressure.rs`, because types.rs sits at its 400-line ceiling — dispatched in cli/dispatch.rs; `pressure` registered in dynamic completions with `--rounds`/`--dry-run` plus the three model flags (`--claude-model`/`--address-model` → `complete_model_names`, `--codex-model` → `complete_codex_model_names`).

## Tiered Knowledge Base (`fs/knowledge/`, `commands/knowledge/`)

Two-tier curated knowledge: generated `INDEX.md`, tier-1 summary files, and tier-2 topics at
`<category>/<slug>.md`. Layout is `Hierarchical` **iff** `INDEX.md` exists. Covers module split,
target parsing, index generation, the `catalog::build` diagnostics (duplicate heading, generic
blurb, broken link, missing source ref), opt-in migration, and lock ordering.

→ [Knowledge Hierarchy](architecture/knowledge-hierarchy.md)

## Codex Plugin (openai-codex) [DETAILED]

The Codex Claude Code plugin: marketplace/install and scope rules, the `loom-codex-forwarder` lane
(loom's own shim — the plugin's `codex:codex-rescue` is never spawned directly since the 2026-08-07
rogue-wrapper incident) and its flag-forwarding contract, `codex-companion.mjs` effort values, the three plugin hooks,
and what loom shipped to drive it — the per-stage `implementers` LIST (licensed lanes in preference
order, default `["claude"]`; membership licenses a lane, order picks the routine-work default, and a
stage mixes lanes per subagent), the `## Codex Implementers` signal section gated on codex
MEMBERSHIP on both the normal and recovery paths, the two-key settings carry-forward that lets a
plugin install survive loom's per-worktree settings rebuild, and the availability fallback that
routes terra-/luna-tier work to sonnet with an advisory `loom run` startup warning (never a hard
failure) when the codex CLI or plugin is not installed.

→ [Codex Plugin](architecture/codex-plugin.md)

## Codex Concurrency [DETAILED]

Empirical spike on running several codex-companion tasks at once in one workspace: **foreground fan-out
over disjoint file sets is verified safe to 6** — edits and results were correct at every concurrency
tested. Only the plugin's unlocked shared `state.json` degrades, which costs observability
(`/codex:status`, `/codex:result`) and rules out _background_ fan-out. Note the cap of 6 is a doctrine
number carried as a literal in the signal prose; there is **no `CODEX_MAX_PARALLEL` constant in the
code**. The page also records what execution did NOT prove: no stage has yet run with codex listed
in `implementers`.

→ [Codex Concurrency](architecture/codex-concurrency.md)

## Terminal Backends: SessionBackend / TmuxBackend [DETAILED]

`SessionBackend` (`orchestrator/terminal/backend.rs`) is the one dispatcher every spawn, kill and
liveness call routes through, wrapping a `Native` lane (host terminal window, the default) and an
opt-in `Tmux` lane (detached tmux server, headless-capable). It selects `[terminal] backend` from
`.work/config.toml` per spawn, builds the native lane lazily in a `OnceLock` that memoizes even the
failure, and records the lane actually used on `Session.backend` so kill/liveness stay correct across
daemon restarts. The tmux lane runs **one server per session** (socket `loom-<session.id>`, keyed on
session id for the 104-byte `sun_path` limit) for crash containment, and its liveness deliberately
**never calls `tmux has-session`** — a server whose pane died still answers 0, which would mask the
crash. The topic covers the spawn-time `has-session` probe and the `new-session exits 0 on failure`
trap, the sticky `.work/terminal-backend-fallback` marker and every path that writes/reads/clears it,
`loom attach`'s overview and direct modes, and the positive-attribution rule for socket reaping.

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

Plan-authored commands (acceptance, setup, truth checks, wiring tests, dead-code
checks, change-impact) run through the single primitive `spawn_confined`, which
rebuilds the child environment from an allowlist. **That is the whole boundary:
environment scrubbing, not isolation.** No namespaces, seccomp, landlock, or
network restriction exists anywhere in loom.

What is and is not guaranteed, the allowlist, and the `Edit(path)`-vs-`Write(path)`
rule: [architecture/execution-containment.md](architecture/execution-containment.md).

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

One append-only JSON-lines file, `.work/telemetry/events.jsonl`, recording
whether a spawned session received a context brief (`ContextDelivered` /
`ContextUnavailable`). Best-effort by contract: `emit` may never fail a spawn and
`read_events` skips a malformed line rather than failing the file. Every count is
an item count, never a token saving.

Written only by `orchestrator/core/stage_telemetry.rs` (called from
`stage_executor.rs:570`), which derives its fields from the `DeliveryRecord`
signal generation already wrote — no second retrieval. `read_events` has **no
production caller** today, and `.work/` is removed when the plan finishes, so
events currently go unread; the intended reader is a future `loom status`/`loom map`
diagnostic.

## Quota Poller

A daemon thread (`loom/src/quota/poller.rs`) polls the Claude OAuth usage endpoint and `codex app-server` every 180 s and caches one `ProviderQuota` per provider under `.loom/work/quota/`; `StatusData.quota` is read from that cache, never polled, and rendered in the `--live` and `--web` footers. Sources, backoff, cache hygiene, and the token-handling rules: [architecture/quota-poller.md](architecture/quota-poller.md).

## Web Dashboard

`loom status --web [PORT]` serves an embedded React SPA over HTTP/WebSocket on `127.0.0.1`, streaming the same `StatusData` the live TUI renders and reusing its collection/derivation code rather than reimplementing it. Without `PORT`, binding starts at 7373 and advances past occupied ports; an explicit port is exact, while `0` delegates selection to the OS. Server shape, broadcaster fallback, the Rust/TS fixture contract, and the committed-dist/build.rs embedding: [architecture/web-dashboard.md](architecture/web-dashboard.md).
