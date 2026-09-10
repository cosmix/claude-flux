---
---
# Cli Process And Conventions

> CLI registration, TUI, error handling, process mgmt, config, HTTP client.

## TUI Patterns

Two modes: **static** (one-time print) and **live** (real-time via daemon socket). Live uses ratatui with vertical layout: header(3), progress(3), main(min 10, two 50/50 columns), footer(3). Left: Executing(60%)+Pending(40%). Right: Completed(60%)+Blocked(40%). Three-layer cleanup: panic hook, Ctrl+C signal handler, Drop with `cleaned_up` flag.

## Knowledge Systems Pattern

Three systems, in ascending order of permanence:

| System            | Location                                                                  | Lifetime                          | Written by                                                                   |
| ----------------- | ------------------------------------------------------------------------- | ---------------------------------- | ----------------------------------------------------------------------------- |
| **Memory**        | `.work/memory/{session}.md`                                               | The run                           | `loom memory note\|decision\|change\|question`                               |
| **Stage outputs** | `outputs: Vec<StageOutput>` on the stage file (key / value / description) | The run; read by dependent stages | `loom stage output set`                                                      |
| **Knowledge**     | `doc/loom/knowledge/` (tiered)                                            | Permanent                         | `loom knowledge update` (stage execution) or direct Write/Edit (interactive) |

Memory is placed in the signal's recitation section for maximum LLM attention. The promotion path from memory to knowledge is the **`knowledge-distill` stage**, which reads `loom memory show --all` and curates — there is no `loom memory promote` command.

`loom knowledge update` appends; `loom knowledge replace-section <file> <heading> [content]` overwrites a `## <heading>` section's body in place — the correction path for stale knowledge — and falls back to appending, with a distinct message, when the heading is not found. There is still no verb that deletes a section outright, or renames its heading (see concerns.md). Knowledge commands resolve through `WorkDir::project_root()` (cwd-relative), so a worktree agent writes to its own worktree rather than the main repo.

**Corrected 2026-07-30:** an earlier version of this section claimed a `.work/facts.toml` cross-stage KV store, a `loom memory promote` command, and `<!-- .loom-protected -->` file markers. None of the three exist in the codebase. Cross-stage KV is `loom stage output`; "Discovered Facts" survives only as a HandoffV2 field and a signal sub-section.

## Error Handling Pattern

Application and orchestration boundaries use `anyhow::Result<T>` with `.context()` or
`.with_context()`. Domain operations use typed errors when a caller must distinguish outcomes, such
as `BaseBranchError`, `MergeProbeError`, and `ProcessTimeoutError`; adapters such as Clap validators
may return strings because their interface requires display text. Do not stringify a domain error
before a caller has finished matching it, and do not add a second general-purpose error framework.
Graceful degradation is explicit and limited to operations whose callers do not require recovery
semantics, such as optional skill discovery or best-effort notification.

## Process Management Pattern

**Wrapper script** (`pid_tracking.rs`): creates `.work/wrappers/<stage_id>-wrapper.sh`, starts from `env -i`, reconstructs a minimal locale/terminal allowlist plus explicit Loom variables, records PID and process start time, then `exec`s the agent. **Liveness/signaling** uses `process::ProcessIdentity`; start-time mismatch is dead and missing identity is unverifiable. Raw PID fallback is forbidden. **Zombie prevention:** `spawn_reaper_thread()` calls `wait()`.

## Directory Hierarchy Pattern

Three-level: **Project Root**, **Worktree** (`.worktrees/<stage-id>/`), **working_dir** (YAML field). Path resolution: `EXECUTION_PATH = worktree_root + working_dir`. All acceptance/artifact/wiring paths relative to working_dir. Common mistake: `cargo test` failing because working_dir not set to Cargo.toml directory.

## Three-Layer Guidance Reinforcement

New agent guidance should be reinforced at: (1) Skill file (depth), (2) CLAUDE.md.template (authority), (3) cache.rs signals (runtime enforcement). Ensures guidance reaches agents regardless of entry point. Agent definitions (`agents/*.md`) serve as a supplementary fourth surface for role-specific guidance (e.g., coordinator/worker roles in subagent hierarchies).

### Mini Adversarial Code Review (multi-surface, 2026-06-25)

Every code-producing stage must end with a MANDATORY mini adversarial code review across six fixed dimensions: **code quality & architecture (SOLID), idiomatic code, security, wiring, dead & unnecessary code, no duplication (DRY across the whole codebase)**. The doctrine is reinforced across six surfaces that MUST stay consistent when the dimensions change:

| Surface                    | Where                                                                                                                                                                                 |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Runtime signal (canonical) | `orchestrator/signals/cache.rs::append_adversarial_review()` — injected into Standard + IntegrationVerify stable prefixes (via `stable_prefix_for`, so the recovery path gets it too) |
| Authority                  | `CLAUDE.md.template` — Stage Completion Checklist "MINI ADVERSARIAL CODE REVIEW" block                                                                                                |
| Implementer agents         | `agents/loom-software-engineer.md`, `agents/loom-senior-software-engineer.md` — "Self-Review Before Returning"                                                                        |
| Reviewer agent             | `agents/loom-code-reviewer.md` — Capabilities aligned to the six dimensions                                                                                                           |
| Plan authoring             | `skills/loom-plan-writer/SKILL.md` — note under stage_type table (auto-injected; don't restate in descriptions)                                                                       |

Scope rule: code stages ONLY. Documentation stages (`knowledge`, `knowledge-distill`) emit only markdown and deliberately omit it; cache + recovery tests negative-assert its absence there. Silent-failure detection is a SEPARATE concern (Standard has its own block; IV has `SILENT FAILURE DETECTION`) — not part of the six dimensions.

## Stage Necessity Test

Before creating ANY stage beyond the bookends, it must answer YES to one of four questions
(`skills/loom-plan-writer/SKILL.md:388`) and the plan prose must NAME which one:

- **Q1** — does another stage need this stage's code _merged_ before it can start? Only a
  MERGE-ORDER dependency counts. "B imports A" is compile-order → foundation step in ONE stage.
- **Q2** — does another stage write files this stage also writes? (file conflict)
- **Q3** — does later work need a verification checkpoint on this first? Name what would go
  undetected without it; "it would be tidy" is not a checkpoint.
- **Q4** — would the combined work blow a single session's context budget?

All NO → merge into ONE stage with parallel subagents over disjoint files. See
[Stage Fragmentation](../mistakes.md) for the detection rule and the cost of getting this wrong.

## macOS GUI App Launch Pattern (2026-04-27)

macOS apps installed in `/Applications/X.app` may ship a CLI binary inside `Contents/MacOS/` that is NOT added to PATH. To launch with arguments without requiring a manual PATH shim, use `open -na <AppName> --args <flags...>` from `Command::new("open")`. The CLI flags following `--args` are passed through to the new process exactly as if invoked directly — Ghostty's `--working-directory=`, `--title=`, and `-e CMD` all work this way (per Ghostty maintainer in ghostty-org/ghostty#9221).

**`-na` vs `-a`:** Always use `-na` (force new instance) when each invocation needs its own per-window args. With `-a`, an already-running singleton may ignore `--args` and just focus the existing window — `--working-directory` and `-e` would silently no-op. Trade-off: process accumulation, acceptable when each window corresponds to a finite stage.

**Where applied:** `emulator.rs` `Self::Ghostty` arm uses this on macOS while keeping the direct `ghostty <args>` invocation on Linux via `#[cfg(not(target_os = "macos"))]`. The arm-level cfg-gating pattern (rather than per-emulator-variant duplication) keeps cross-platform terminals together. Same approach applies to any future `.app`-distributed terminal emulator added to loom.

**When NOT to use:** Mac-only emulators (`TerminalApp`, `ITerm2`) already use AppleScript via `osascript`, which is itself PATH-independent — no `open` needed. Use `open -na ... --args` only when the underlying tool accepts CLI flags directly.

## CLI Subcommand Registration Pattern

Adding any new top-level command (e.g. `loom plan`) requires touching exactly **three files**:

1. **`loom/src/cli/types.rs`** — Add variant to `Commands` enum (with `#[command(subcommand)]` if nested):

   ```rust
   /// Validate a plan without side effects
   Plan {
       #[command(subcommand)]
       command: PlanCommands,
   },
   ```

2. **`loom/src/cli/dispatch.rs`** — Add match arm in `dispatch()`:

   ```rust
   Commands::Plan { command } => match command {
       PlanCommands::Verify { path, strict } => plan::verify(path, strict),
   },
   ```

   Also add the module import at the top: `use loom::commands::plan;`

3. **`loom/src/commands/newcmd.rs`** (or `commands/newcmd/mod.rs`) — Implement the execute function.
   Then expose it from `loom/src/commands/mod.rs`: `pub mod newcmd;`

**Verification**: `cargo build` must pass. `loom <newcmd> --help` must show the command.

**Nested subcommands**: define a second `#[derive(Subcommand)]` enum in `cli/types.rs` (e.g. `PlanCommands`), mirror the outer pattern. See `types_stage.rs` / `types_memory.rs` for examples of extracted sub-enum files.

### Gotcha: Clap is only HALF the registration — dynamic completions are a separate site

The three files above make a command **compile, dispatch, and show in `--help`**, but loom ships a **second, hand-maintained completion engine** that does NOT read Clap's metadata. A command registered only via the three-file pattern is **invisible to shell tab-completion** (and its flags won't complete). The completion tables are hardcoded string lists in `loom/src/completions/dynamic/`:

- `commands.rs` — `TOP_LEVEL_COMMANDS` (the top-level name list; keep it alphabetical), `complete_flags` (per-command-path flag arms, e.g. `["pressure"] => &["--rounds", "--dry-run"]`), `complete_subcommands` + `has_subcommands` (only for commands with nested sub-enums).
- `mod.rs` — `complete_after_command` routes the positional arg of a single-level command. A command whose positional is a **file path** (like `init`) returns `Ok(Vec::new())` so the shell falls back to native path completion; a command taking a stage ID calls `complete_stage_ids`.
- `tests/tests_commands.rs` — add a test asserting the new command appears in top-level completion and that its flags complete.

**Rule:** "register a CLI command" in this repo = Clap (3 files) **AND** the dynamic-completion tables + their tests. Before assuming Clap is the whole story, `rg TOP_LEVEL_COMMANDS loom/src/completions`. This is easy to miss because the command works end-to-end in manual testing and `--help` — only tab-completion silently lacks it.

## Centralized Config File Ownership (toml_edit)

All writes to `.work/config.toml` go through `fs/work_dir.rs` using `toml_edit` for round-trip-safe writes. `toml` is for typed reads. Never mix: `toml_edit Item -> serde` silently drops nested sub-tables.

`read_section::<T>` re-parses the whole file with `toml::Value` then `try_into` on the section — preserves nested config sub-tables.

## Plan Validation Tier Separation (loom init contract)

`loom init` runs validation in two distinct tiers that `loom plan verify` must mirror:

**Tier 1 — Fatal (blocks init):**

- `plan/schema/validation.rs::validate(&metadata)` — called inside `parse_and_validate()` inside `parse_plan_content()`
- Returns `Err(Vec<ValidationError>)` on failure; parse aborts, init fails immediately
- Checks: unsupported version, duplicate stage IDs, unknown deps, path traversal, empty acceptance, artifact path safety, wiring regex validity, bug_fix/regression_test consistency

**Tier 2 — Advisory (printed, never block):**

- `validate_structural_preflight(&stages, repo_root)` — warnings for double-path prefixes, weak wiring patterns, missing build config files, before/after check imbalance
- `check_knowledge_recommendations(&stages)` — warns if plan has no knowledge-bootstrap stage
- `check_sandbox_recommendations(&metadata)` — rejects command-prefix exclusions and flags other unsafe sandbox expansion such as `allow_unsandboxed_escape`
- All return `Vec<String>`; init prints them and continues

**`loom plan verify` contract:** run `parse_plan()` first (auto-runs Tier 1); if it returns `Err`, report fatal errors and exit non-zero. If it succeeds, run the three Tier 2 functions, print their warnings, exit 0 (advisory only).

**Known gap (2026-05-14):** `loom plan verify` does NOT validate `sandbox.permission_mode=bypass-permissions`. That check lives only in `sandbox::config::validate_config`, called from `commands/init/plan_setup.rs` (init path) and at spawn time. `plan verify` skips it, so a plan with `bypass-permissions` reports 0 errors from `plan verify` but fails at `loom init`. Fix: thread `validate_config` into the `plan verify` flow.

**Call site:** `loom/src/commands/init/plan_setup.rs` — shows the canonical order and how warnings are surfaced to the user.

## reqwest::blocking HTTP Client Pattern

Template from `commands/self_update/client.rs`, for loom's actual HTTP consumers. NOT for the adjudicator, which this line used to point at: it spawns a `claude -p` session and makes no HTTP call.

```rust
use reqwest::blocking::Client;

fn create_http_client() -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))  // includes all transfer time
        .user_agent("loom-adjudicator")     // change per consumer
        .build()
        .context("Failed to create HTTP client")
}

fn validate_response_status(response: &reqwest::blocking::Response, context: &str) -> Result<()> {
    if !response.status().is_success() {
        bail!("HTTP {} {}: {}", response.status().as_u16(),
              response.status().canonical_reason().unwrap_or("Unknown"), context);
    }
    Ok(())
}
```

`reqwest::blocking::Client` is already a dependency (used by self_update); no new Cargo.toml entry needed for the adjudicator.

## Synchronous Foreground Agent Driver (`loom pressure`)

A second execution model distinct from the daemon/worktree orchestrator: `loom pressure` (commands/pressure/mod.rs) spawns external agents synchronously in the foreground. The reusable sub-patterns:

- **Foreground spawn, inherited stdio:** children run via `Command::status()` (blocking) with `Stdio::inherit()` for stdin/stdout/stderr, in `current_dir(repo_root)`. No terminal backend, no session tracking, no `.work/`. Use this shape when a command orchestrates interactive tools the user must watch live, rather than background stages.
- **Single-source argv builders:** `claude_args()`/`codex_args()` are the ONLY place argv is assembled, consumed by BOTH the real spawn (`spawn_*`) and `render_dry_run`. `--dry-run` can therefore never drift from what actually runs. Apply whenever a command has a preview/plan mode.
- **Sibling-report naming + pre-delete guard:** the Codex review is written to `codex-<basename>` next to the plan. The report is deleted at the START of every round so that if Codex fails to write a fresh review, the following `/address` cannot silently read the previous round's stale report; a final delete cleans up after the last round.
- **Repo-relative invocation, not cwd-relative:** `resolve_plan_path` derives the agent argument via `fs_path.strip_prefix(repo_root)` (repo-relative when under the repo, else absolute) because children run with `current_dir(repo_root)`, not the user's shell cwd. It gates on `is_file()` (not `exists()`, which is true for dirs) and falls back to `doc/plans/<arg>` only when the raw path is absent AND does not already start with `doc/plans/` (double-prefix guard).
- **Visible exit classification:** `classify_code` maps `0`→continue, `130`/`2`/`None`→abort, other→warn — but the abort/warn handlers PRINT the child label + exit code, so a headless failure (e.g. a codex clap usage error exiting 2) is surfaced rather than mistaken for a clean Ctrl+C interrupt.

## Confidence Ceilings as Named Constants

When a component's view is structurally narrower than the claim it is asked to make, encode
the gap as a numeric ceiling in a named constant whose docstring carries the reasoning —
not as a comment at the call site. The source graph does this with
`MAX_INFERRED_CONFIDENCE = 0.5` (an extractor sees one file), `MAX_RESOLVED_INFERRED_CONFIDENCE = 0.9`
(whole-graph uniqueness is evidence, but not a parse) and `1.0` reserved for `Parser`
provenance alone.

Two properties make it work: a widening is a **new constructor** encoding the wider bound
(`SourceEdge::resolve_to`), never a raw field write; and path-level aggregation takes the
**MINIMUM** edge confidence along a path, never a product — a product punishes long
fully-parsed chains for their length (`1.0 × 1.0 × 1.0` stays `1.0`, but `0.9^5` reads as a
guess). `resolve.rs`'s `Trust::extend` only lowers the running minimum, carrying the weakest
edge's provenance and kind with it.

## Best-Effort By Contract, Stated In the Docstring

`telemetry` and `context::delivery` are both declared optimisations that may never fail the
operation they observe: `emit` discards its own error, `read_events` skips a malformed line
rather than failing the file, a missing delivery directory reads as "nothing delivered".
Writing that contract into the module docstring is what stops a later author "fixing" the
swallowed error into a propagated one.

The corollary is the failure budget rule: **the durable result and the derived artifact have
different budgets — never let the cheaper one veto the expensive one.** A reconcile failure
marks derived state stale and leaves a good merge intact.
