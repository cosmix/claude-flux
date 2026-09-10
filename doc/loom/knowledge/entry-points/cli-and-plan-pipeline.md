---
---
# Cli And Plan Pipeline

> CLI dispatch, plan parsing/validation/graph, verification, configs

## CLI Entry Point

- `loom/src/main.rs` - CLI entry (clap `#[derive(Parser)]`), `Commands` enum dispatch
- `loom/src/lib.rs` - Module exports (14 public modules)

## Command Dispatch (cli/types.rs)

| Command       | Entry File                    | Purpose                                        |
| ------------- | ------------------------------ | ----------------------------------------------- |
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
| `memory`      | `commands/memory/handlers/mod.rs` | Session memory journal                     |
| `review`      | `commands/review/mod.rs`      | Generate review docs from memories             |
| `self-update` | `commands/self_update/mod.rs` | Update loom binary                             |
| `clean`       | `commands/clean/mod.rs`       | Clean up resources                             |
| `repair`      | `commands/repair.rs`          | Fix workspace issues                           |
| `map`         | `commands/map.rs`             | Codebase structure analysis                    |
| `pressure`    | `commands/pressure/mod.rs`    | Plan pressure-testing driver (Claude + Codex)  |
| `diagnose`    | `commands/diagnose.rs`        | Stage failure diagnosis                        |
| `plan verify` | `commands/plan/verify.rs`     | Validate plan file without side effects        |
| `check`       | `commands/verify.rs`          | Goal-backward verification (`verify::execute`) |
| `skill-index` | `commands/skill_index.rs`     | Build skill keyword index for skill-trigger    |
| `completions` | `completions/mod.rs`          | Shell completions (custom scripts + dynamic)   |
| `complete`    | Hidden (dynamic completions)  | Backend for shell tab completions              |

Total: 23 visible commands + 1 hidden (`complete`, for dynamic completions). Dispatch: `cli/dispatch.rs` match-based, two-level for nested commands. `completions` dispatches from `cli/dispatch.rs` into the top-level `completions/` module, not a `commands/completions/` directory.

**Three commands that do NOT exist** (an earlier version of this table listed all three — verify against `cli/dispatch.rs` before citing one):

- `loom hooks` — there is no `commands/hooks.rs`. Hook install lives in `fs/permissions/hooks.rs` and runs as part of `loom init` / `loom repair --fix`.
- `loom sandbox` — there is no `commands/sandbox/`. Sandbox config generation lives in `sandbox/config.rs` + `sandbox/settings.rs`, driven by the plan.
- `loom verify` — there is no `commands/check.rs`; loom has no top-level `verify` command. `commands/verify.rs::execute` is reached via `loom check`; `loom plan verify` is the only separate verify subcommand. The unsafe `loom stage verify` completion pipeline was removed.

## CLI Subcommand Registration Pattern

Three files to add a new subcommand:

1. `cli/types_memory.rs` - Define variant in KnowledgeCommands/MemoryCommands enum
2. `cli/dispatch.rs` - Add dispatch match arm
3. `commands/<module>/` - Implement handler

## Schema-to-Runtime Conversion

- `plan/schema/types.rs` - StageDefinition (YAML input); SandboxConfig + StageSandboxConfig with `permission_mode: Option<PermissionMode>`
- `models/stage/types.rs` - Stage (runtime model)
- `models/stage/methods.rs` - canonical `Stage::from_definition()` conversion
- `commands/init/plan_setup.rs` - delegates stage creation to the canonical conversion

## Plan Parsing Pipeline

- `plan/parser.rs` - Markdown plan parser (extracts YAML from `<!-- loom METADATA -->`)
- `plan/schema/types.rs` - LoomMetadata, StageDefinition structs
- `plan/schema/validation.rs` - Stage validation (goal-backward required for Standard only)
- `plan/graph/mod.rs` - Execution DAG with cycle detection

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
| ----------------------------------------------------- | ------------------------------------- | ------------------------------------------------- |
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

## New CLI Surface (2026-08-17)

**Documented from the clap definitions, not from `--help`.** The installed PATH binary can
lag `main` mid-plan: at the end of this plan `loom knowledge context` worked while
`loom map --outline`, `loom context` and `loom hook` all reported "unrecognized subcommand"
from the same binary. Verify a flag against `loom/src/cli/` before documenting it.

| Command                              | Defined in                      | Options                                                                                                                                                                                                                                  |
| ------------------------------------- | ---------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
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

## Key Config Files

- `.work/config.toml` - Active plan reference and settings
- `.work/stages/{depth}-{stage-id}.md` - Stage state (YAML frontmatter)
- `.work/sessions/{session-id}.md` - Session tracking
- `.work/signals/{session-id}.md` - Agent instruction signals
- `doc/plans/PLAN-*.md` - Plan definition files
