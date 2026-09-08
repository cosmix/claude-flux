# Loom

Loom is an agent orchestration system for Claude Code. You write a plan; loom executes it — stages run in parallel across isolated git worktrees, completion is gated by checks loom runs itself rather than by the agent's own account of its work, and what each session learns is captured and distilled into a knowledge base the next session reads first.

## What Loom Solves

Autonomous agent work fails in a small number of predictable ways. Loom answers each with a mechanism, not a paragraph of prompt.

| Failure mode                            | What actually happens                                                                | Loom's answer                                                                                                                                                                                          |
| --------------------------------------- | ------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **False completion**                    | Tests were never run, the module was written but never imported, the fix is a `TODO` | Loom runs the acceptance criteria itself, then checks artifacts for stubs, wiring for real integration, and dead-code patterns for orphaned work. The bypass flags need a token the agent cannot read. |
| **Instruction drift**                   | Rules decay the moment they scroll out of attention                                  | Shell hooks enforce the load-bearing rules deterministically — commit discipline, staging scope, worktree boundaries, subagent limits — outside the model's control.                                   |
| **Amnesia**                             | Every session rediscovers the same architecture and repeats the same mistakes        | A per-stage memory journal feeds a distillation stage that curates permanent, tiered knowledge; later sessions read it before touching code.                                                           |
| **Cost scaling with tokens, not value** | Expensive models doing cheap work; re-reading everything, every time                 | Judgment stays on an orchestrator; bulk implementation is delegated to cheap subagents. Signals are laid out for KV-cache reuse and knowledge is tiered, so agents load only what they need.           |
| **Context exhaustion**                  | The session degrades into an expensive compaction loop                               | Context budgets are monitored per stage; a handoff is written *before* compaction and the resumed session is re-anchored to its assignment.                                                            |
| **Lost runs**                           | A crashed or hung session takes the work with it                                     | All state is files under `.work/`. The daemon detects dead and hung sessions, classifies the failure, and retries or escalates.                                                                        |
| **Serialization**                       | Multi-stage work runs one-at-a-time, or collides on the same files                   | A dependency DAG schedules independent stages concurrently in separate worktrees, with progressive auto-merge and dedicated conflict-resolution sessions.                                              |

## Key Capabilities

### Deterministic guardrails

The rules that matter are not left to the model. Loom installs Claude Code hooks, a Codex-native hook subset, and a git `pre-commit` hook that fire regardless of what an agent intends:

- `commit-guard.sh` blocks a session from ending with uncommitted work or a stage still `Executing`
- `git-add-guard.sh` blocks `git add -A` / `git add .`; `git-pre-commit-hook.sh` blocks commits containing `.work` or `.worktrees`
- `worktree-isolation.sh` / `worktree-file-guard.sh` block cross-worktree writes, reads, and path traversal
- `commit-filter.sh` blocks subagent git operations (a subagent commit loses the main agent's work) and blocks AI attribution in commit messages
- `subagent-verify-guard.sh` blocks subagents from running project-wide build/test/lint suites, so verification stays with the one agent that can see the whole tree — with `integration-verify` stages carved out, and no opt-out environment variable
- `pre-compact.sh` blocks compaction, writes a handoff, then allows it; `session-start.sh` re-anchors the resumed agent to its signal file
- `plans-path-guard.sh` keeps plans in `doc/plans/` where loom and git can see them

Subagent detection is a live process-tree ancestry check, not a PPID comparison. See [Verification Is the Main Agent's Job](#verification-is-the-main-agents-job).

### Verification that outlives the agent's opinion

`loom stage complete` is not a self-report. Loom executes the stage's acceptance criteria in-process and refuses completion on failure, leaving the stage `Executing` so the agent must fix and retry. On top of that, goal-backward verification asks whether the *outcome* exists:

- **`artifacts`** — files exist and contain real implementation (stub detection rejects `TODO`, `FIXME`, `unimplemented!`, `todo!`, `pass`, `NotImplementedError`)
- **`wiring`** — regex proof that new code is actually referenced: module registered, route mounted, component rendered
- **`wiring_tests`** — runtime commands proving the integration behaves
- **`dead_code_check`** — command output patterns catching code that exists but is never called
- **`before_stage` / `after_stage`** — pre-spawn and post-acceptance gates; a failed pre-check blocks the stage before a session is even spawned

The escape hatches (`--no-verify`, `--force-unsafe`, `--assume-merged`) require a one-time operator proof bound to the project, stage, action, and exact flag set. The operator supplies the daemon secret only while minting the proof; the target command cannot fetch that credential for its caller or reuse the proof for another action.

### Knowledge capture and distillation

Loom treats what agents learn as a first-class artifact with a pipeline, not a scratch file.

1. **Capture** — during execution, agents record to a per-stage journal: `loom memory note` (gotchas, mistakes-with-prevention), `decision` (with rationale), `change`, `question`. The journal is injected into the *recitation* section at the end of the next signal, where model attention is highest.
2. **Distill** — a `knowledge-distill` stage runs at the end of a plan, reads every stage memory, and curates it into permanent knowledge — mistakes rewritten as actionable prevention rules, decisions with their rationale, reusable patterns and conventions.
3. **Retrieve** — the result is a **tiered** base under `doc/loom/knowledge/`: a generated `INDEX.md`, seven tier-1 summaries, and tier-2 topic files. Inside a stage, the per-stage Knowledge Brief comes first. Otherwise, agents read `INDEX.md` for orientation, then the tier-1 summary for their area, then only the topics they touch; a specific question is pulled with `loom knowledge context --query`, which returns the matching sections quoted — so the base can grow without every session paying to load it.

Knowledge lives in `doc/loom/knowledge/`; agents write it through loom, and `loom knowledge sync` rebuilds the derived retrieval artifacts after the tree changes, including the one-time flat-to-hierarchical upgrade. Details: [Knowledge System](#knowledge-system).

### Cost control by construction

Loom's savings come from **delegation, not downgrade**:

- **Orchestration is always Opus at `xhigh` effort.** Every stage's main agent plans, decomposes, verifies, and commits — the judgment-heavy work that is worst to economize on.
- **Implementation is always delegated**, spawned by agent type so the choice is explicit rather than inherited: Fable for major bugs, visual/UI design, and extremely challenging algorithmic design (no agent type pins it — the model override is stated explicitly at spawn); Opus for mainstream architecture and algorithm implementation; Sonnet or Codex GPT-5.6 Terra for common implementation and integration tests; Codex GPT-5.6 Luna for boilerplate, scaffolding, and simple unit tests. The codex tiers are licensed only on stages listing codex in `implementers`, and additionally require the `codex` CLI and its plugin to be installed — when either is missing, `loom run` prints an advisory warning at startup (it never aborts) and terra-/luna-tier work falls back to Sonnet.
- **Signals are built for cache reuse.** Each signal is a four-section layout with a per-stage-type stable prefix that is byte-identical across sessions, so the large doctrine block is a cache hit rather than a re-read.
- **Context budgets prevent compaction**, which is the expensive failure: an uncached re-read that costs more and produces worse work.
- **Tiered knowledge and a skill index** keep the working set small — at most 5 matched skills are injected per stage, out of 61 installed.
- **Orchestrated sessions are interactive**, billing against your Claude subscription. The handful of headless `claude -p` paths are opt-in flags, off by default (see the Billing note below).

Per-stage `model`, `reasoning_effort`, and `ultracode` fields let you override any of this explicitly.

### Parallel execution and progressive merge

Stages form a dependency DAG; everything independent runs at once, each in its own worktree (`.worktrees/<stage-id>`, branch `loom/<stage-id>`). Completed stages merge back progressively under a file lock, and a real conflict spawns a dedicated resolution session rather than stalling the run.

### Crash recovery and liveness

All orchestration state is plain files in `.work/`, so nothing is lost when a process dies. The daemon polls every 5s, tracks PID liveness and per-session heartbeats, flags hung sessions after 300s, and classifies failures across ten types into retryable (exponential backoff) and needs-diagnosis. Tool-call telemetry drives a stuck-session signal when a session's recent calls are overwhelmingly failures. Orphaned sessions are recovered on daemon restart.

### Sandboxing and plan hardening

Plan-level defaults and per-stage overrides control filesystem reads/writes, network domains, and permission mode for the agent session, and commands loom runs from your plan get a rebuilt, allowlisted environment so they cannot read ambient credentials ([Sandbox Configuration](#sandbox-configuration)). Before you spend anything, `loom plan verify` validates a plan with no side effects — running the same sandbox validation that would otherwise only fail at `loom init` — and `loom pressure` hardens it through adversarial review rounds run by two different model families.

### Human-in-the-loop where it matters

Thirteen stage states make "needs a person" a first-class outcome rather than a hang: `WaitingForInput` (raised automatically when an agent asks a question), `NeedsHumanReview`, `Blocked`, `MergeConflict`. Operators get `loom stage hold/release/skip/retry/human-review`, and an agent that believes a criterion is wrong can escalate with `loom stage dispute-criteria` instead of quietly weakening it.

## Platform Support

- Linux x86_64: primary development and full CI test runs; signed release binary
- macOS (Apple Silicon): supported for build/terminal integration, CI does build-only verification; signed release binary
- macOS (Intel): builds from source; no release binary is published
- Windows via WSL2: supported — WSL2 runs the Linux x86_64 binary unmodified. Use the tmux backend, since a stock WSL install has no GUI terminal emulator for the native backend to find — see [Running under WSL](#running-under-wsl). Native Windows, outside WSL, is unsupported.
- Linux ARM64: builds from source; no release binary is published yet
- Headless (SSH, no terminal emulator): supported via the tmux backend — see [Terminal Backends](#terminal-backends)

## Quick Start

Loom is under active development. Signed binaries are published for Linux x86_64 and macOS Apple Silicon; every other platform builds from source with the Rust toolchain installed.

### Prerequisites

| Tool                       | Needed for                                                        | Required?                                                                               |
| -------------------------- | ----------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| Rust toolchain (`cargo`)   | building the `loom` binary                                        | only when building from source                                                          |
| `git`                      | worktrees, merges, crash reports                                  | yes                                                                                     |
| `claude` (Claude Code CLI) | every orchestrated session                                        | yes                                                                                     |
| `jq`                       | every loom hook parses the Claude Code hook payload with it       | yes; `install.sh` and `loom run` refuse to proceed without it, `loom repair` reports it |
| `rg` (ripgrep) and `fd`    | the installed CLAUDE.md steers agents to these over `grep`/`find` | recommended; `install.sh` and `loom run` warn when missing                              |
| `tmux`                     | the tmux terminal backend                                         | optional                                                                                |
| `sccache`                  | shared dependency compiles across stage worktrees                 | optional                                                                                |
| `codex` CLI                | the codex implementer lane and `loom pressure`                    | optional                                                                                |

### 1. Install Loom

```bash
curl -fsSL https://raw.githubusercontent.com/cosmix/loom/main/install.sh | bash
```

This downloads the signed release binary for your platform to `~/.local/bin/loom`, then installs Loom's agents, skills, commands, hooks, and orchestration rules into `~/.claude/` and `~/.codex/` from the assets embedded in that binary. Codex asks you to review new or changed non-managed hooks with `/hooks` before they run.

To build from source instead — required on Linux ARM64, and what you want when working on loom itself:

```bash
git clone https://github.com/cosmix/loom.git
cd loom
bash ./dev-install.sh
```

`dev-install.sh` builds the release binary (`cargo build --release`) and runs `install.sh`, which installs `loom-*` prefixed agents and skills (non-destructively, preserving user customizations), hooks, and configuration into `~/.claude/` and the CLI binary to `~/.local/bin/loom`. Orchestration rules are written directly to `~/.claude/CLAUDE.md` (existing file is backed up).

`install.sh` takes an optional `--skills core|all` flag (default `core`): `core` installs a small set of always-loaded core skills to `~/.claude/skills/` and catalogs the rest under `~/.claude/loom-skill-catalog/`, loaded on demand; `all` installs every loom skill directly to `~/.claude/skills/`.

### 2. Write a Plan

Plans are how loom knows what to build. Open Claude Code in your target project and use the `/loom-plan-writer` skill to create one:

```bash
cd /path/to/project
claude  # start Claude Code CLI
```

Inside the Claude Code session:

1. Enter plan mode (`/plan`)
2. Load the plan-writing skill by typing `/loom-plan-writer`
3. Describe what you want to build and discuss with Claude
4. Claude will write the plan to `doc/plans/PLAN-<name>.md`

To validate the draft before running it:

```bash
loom plan verify doc/plans/PLAN-<name>.md
```

### 3. Run Loom

Once your plan is written:

```bash
loom init doc/plans/PLAN-<name>.md
loom run
loom status --live
loom stop
```

`loom init` parses the plan, creates stage state, and installs/configures project hook wiring automatically. For an existing repo that is missing Claude Code hook setup, run `loom repair --fix`.

### What Gets Installed

| Location                     | Contents                                                  |
| ---------------------------- | --------------------------------------------------------- |
| `~/.claude/agents/loom-*.md` | 4 specialized subagents (per-item, non-destructive)       |
| `~/.claude/skills/loom-*/`   | 9 core domain knowledge modules, always loaded (per-item, non-destructive) |
| `~/.claude/loom-skill-catalog/loom-*/` | 53 more domain knowledge modules, loaded on demand (`--skills core`, the default) |
| `~/.claude/commands/*.md`    | Loom slash commands (`/pressure`, `/address`, `/distill`) |
| `~/.claude/hooks/loom/`      | Embedded lifecycle and guardrail hooks + shared libraries |
| `~/.claude/CLAUDE.md`        | Orchestration rules                                       |
| `~/.codex/skills/pressure/`  | Codex pressure-testing skill (`$pressure`)                |
| `~/.codex/hooks/loom/`       | Loom hook assets used by Codex-native registrations      |
| `~/.codex/hooks.json`        | Non-destructively merged Codex hook registrations        |
| `~/.codex/AGENTS.md`         | Codex navigation and execution doctrine                   |
| `~/.local/bin/loom`          | Loom CLI                                                  |

## Core Workflow

1. Open Claude Code, enter plan mode (`/plan`), and use `/loom-plan-writer` to write a plan to `doc/plans/`.
2. Run `loom init <plan-path>` to parse metadata and create stage state.
3. Run `loom run` to start daemon + orchestrator.
4. Track progress with `loom status --live`.
5. Recover, verify, merge, or retry stages as needed.

### Stage Lifecycle

```text
WaitingForDeps → Queued → Executing → Completed
```

Everything else is an explicit, inspectable outcome rather than a hang:

| State                   | Meaning                                                                |
| ----------------------- | ---------------------------------------------------------------------- |
| `Blocked`               | A `before_stage` check or an explicit block stopped the stage          |
| `NeedsHandoff`          | Context ceiling reached; a handoff was written                         |
| `WaitingForInput`       | The agent asked a question (raised automatically by the AskUser hooks) |
| `MergeConflict`         | Auto-merge hit a real conflict; a resolution session is spawned        |
| `MergeBlocked`          | Merge cannot proceed (e.g. another merge is in progress)               |
| `CompletedWithFailures` | Work finished but acceptance did not pass                              |
| `NeedsHumanReview`      | Escalated to a person                                                  |
| `NeedsAdjudication`     | A disputed acceptance criterion is awaiting a verdict                  |
| `Skipped`               | Explicitly skipped                                                     |

## CLI Reference

### Primary Commands

```bash
loom init <plan-path> [--clean] [--backend native|tmux]
loom run [--manual] [--max-parallel N] [--foreground] [--watch] [--no-merge] [--backend native|tmux]
loom status [--live] [--compact] [--verbose] [--web [PORT]]
loom stop
loom resume <stage-id>
loom check <stage-id> [--suggest]
loom diagnose <stage-id>
loom pressure <plan-path> [--rounds N] [--dry-run]
```

`loom pressure` hardens a plan before you run it by combining two external agents over `--rounds` rounds (default 2). Each round runs both pressure-tests in parallel: Claude `/pressure` edits the plan in place in the foreground (you watch it live), while Codex `$pressure` writes an independent review next to it (`codex-<plan>.md`) in the background (its output is captured to a temp log to keep the terminal clean). Once both finish, Claude `/address` folds the review back in. Claude stays interactive (subscription billing) and auto-closes when done; Codex runs from the repo root. Requires both the `claude` and `codex` CLIs on PATH. `--dry-run` prints the exact commands without spawning anything.

`loom status --live` renders a live ledger dashboard: one row per stage across eight columns (STATE, STAGE, DEPENDS ON, MODELS, ACTIVITY, CONTEXT, TIME, MERGE). MODELS lists the orchestrator's own model first, then the models any subagents it spawned ran on. Columns drop in priority order as the terminal narrows; below a 64x16 (columns x rows) terminal a notice replaces the dashboard entirely. Press `?` to toggle a legend overlay explaining every state icon.

`loom status --web [PORT]` starts a read-only web dashboard bound to `127.0.0.1` and serves the same live ledger over a WebSocket in the browser. Without `PORT`, it starts at port 7373 and automatically tries the next available port when a candidate is occupied. Supplying a nonzero `PORT` requests that exact port; `PORT` 0 asks the OS for any free port. It works without the daemon by polling `.work/` files directly when the daemon socket is unreachable.

### Plan Commands

```bash
loom plan verify <plan-path> [--strict] [--json] [--no-color]
```

`loom plan verify` validates a plan file without touching `.work/` or requiring a git repo. It runs the same fatal validation as `loom init` (schema errors, unknown or retired fields at every nested policy layer, duplicate IDs, unknown dependencies, path safety) plus advisory warnings (structural issues, missing knowledge-bootstrap stage, sandbox gaps). A retired top-level `truths` block is rejected; move behavioral commands to `acceptance`. Exits 0 on success, non-zero on fatal errors; `--strict` promotes warnings to errors.

### Stage Commands

```bash
loom stage complete <stage-id> [--session <id>] [--no-verify] [--force-unsafe --assume-merged]
loom stage block <stage-id> <reason>
loom stage reset <stage-id> [--hard] [--kill-session]
loom stage waiting <stage-id>
loom stage resume <stage-id>
loom stage hold <stage-id>
loom stage release <stage-id>
loom stage skip <stage-id> [--reason <text>]
loom stage retry <stage-id> [--force] [--context <message>]
loom stage merge [stage-id] [--resolved]
loom stage human-review <stage-id> [--approve|--force-complete|--reject <reason>]
loom stage dispute-criteria <stage-id> --criterion-index N --reason <text> [--evidence-commit <sha>] [--failure-output <path>]
```

`loom stage dispute-criteria` is the sanctioned way for an agent to challenge a criterion it believes is wrong or impossible, instead of quietly weakening it. The daemon writes `request.md` and moves the stage to `NeedsAdjudication`; the verdict is daemon-written and never authored by the agent.

### Stage Outputs

```bash
loom stage output set <stage-id> <key> <value> [--description <text>]
loom stage output get <stage-id> <key>
loom stage output list <stage-id>
loom stage output remove <stage-id> <key>
```

### Knowledge / Memory

```bash
loom knowledge sync [--structural-only] [--json]              # Rebuild derived retrieval artifacts after editing knowledge
loom knowledge check [--strict] [--json]                      # Report knowledge-base diagnostics (read-only; never opens the context store)

loom memory note <text> [--stage <id>]
loom memory decision <text> [--context <why>] [--stage <id>]
loom memory change <text> [--stage <id>]
loom memory question <text> [--stage <id>]
loom memory query <search> [--stage <id>]
loom memory list [--stage <id>] [--entry-type <type>]
loom memory show [--stage <id>] [--all]
```

See [Knowledge System](#knowledge-system) for how these fit together.

### Other Commands

```bash
loom review [--ai-summary]                                                   # Generate a code-review doc from stage memories; --ai-summary uses headless `claude -p` (see Billing note)
loom usage [--since <duration|date>] [--project <path>] [--all] [--stage <id>] [--plan <name>] [--json]  # Report what agent sessions actually consumed, in tokens
loom attach [stage-id]                                                       # tmux backend only; omit the id for a tiled overview
loom sessions list
loom sessions kill <session-id...> | --stage <stage-id>
loom worktree list
loom worktree remove <stage-id>
loom graph
loom context record-edit --stage <id> --path <path> [--path <path>...]       # Keep a stage's context overlay current
loom hook user-prompt                                                        # UserPromptSubmit entry point; invoked by loom's hooks
loom repair [--fix]
loom clean [--all|--worktrees|--sessions|--state]
loom update
loom install-assets [--claude-dir <path>] [--codex-dir <path>] [--skills core|all]  # Install loom's agents, skills, commands, hooks and doctrine files
loom completions [<shell>] [--install] [--migrate]
```

### ⚠️ Billing: headless `claude -p` flags

Loom runs every orchestrated stage as a normal **interactive** Claude Code session, which bills against your Claude subscription exactly like launching `claude` yourself. One **opt-in** flag instead invokes Claude in headless print mode (`claude -p`):

| Command       | Flag           | Behavior without the flag                                       |
| ------------- | -------------- | --------------------------------------------------------------- |
| `loom review` | `--ai-summary` | Uses the plan's first paragraph as the summary (no Claude call) |

Headless `claude -p` usage may be billed **separately from (and in addition to) your Claude subscription** as API/extra charges, depending on your account and auth setup. This flag is **off by default** so loom never silently incurs those charges — only pass it when you knowingly accept the headless billing.

## Plan Format

Plans live in `doc/plans/` with metadata in fenced YAML between loom markers.

````markdown
# PLAN-0001: Feature Name

<!-- loom METADATA -->

```yaml
loom:
  version: 1
  sandbox:
    enabled: true
  stages:
    - id: implement-api
      name: Implement API
      description: Add endpoint + tests
      working_dir: "."
      stage_type: standard
      dependencies: []
      acceptance:
        - "cargo test"
        - command: "cargo test api_integration::returns_200"
          stdout_contains: ["test result: ok"]
      files:
        - "loom/src/**/*.rs"
      artifacts:
        - "loom/src/api/*.rs"
      wiring:
        - source: "loom/src/main.rs"
          pattern: "mod api;"
          description: "API module registered"
      execution_mode: team

    - id: integration-verify
      name: Integration Verify
      working_dir: "."
      stage_type: integration-verify
      dependencies: ["implement-api"]
      acceptance:
        - "cargo test --all-targets"
        - command: "cargo test api_integration::returns_200"
          stdout_contains: ["test result: ok"]
```

<!-- END loom METADATA -->
````

### Plan-Level Context Fields

Set in the `loom:` block alongside `version` and `stages`, these supply the
default ceiling for every stage that does not declare its own. Both are
absolute resident-token counts with a minimum of 60000, and both are persisted
into `.work/config.toml`'s `[context]` section at `loom init`.

| Field                     | Required | Notes                                                                                      |
| ------------------------- | -------- | ------------------------------------------------------------------------------------------ |
| `context_ceiling_tokens`  | No       | Default ceiling for a stage's main agent session (default 150000)                           |
| `subagent_ceiling_tokens` | No       | Ceiling for subagents spawned by a stage session (default 120000); never read from a stage  |

### Stage Fields

| Field                              | Required               | Notes                                                                                                                                      |
| ---------------------------------- | ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `id`                               | Yes                    | Stage identifier                                                                                                                           |
| `name`                             | Yes                    | Human-readable title                                                                                                                       |
| `working_dir`                      | Yes                    | Relative execution directory (`.` allowed)                                                                                                 |
| `description`                      | No                     | Optional summary                                                                                                                           |
| `dependencies`                     | No                     | Upstream stage IDs                                                                                                                         |
| `acceptance`                       | Conditionally required | Shell criteria (strings or extended objects with stdout_contains etc.)                                                                     |
| `setup`                            | No                     | Setup commands                                                                                                                             |
| `files`                            | No                     | File glob scope                                                                                                                            |
| `stage_type`                       | No                     | `standard` (default), `knowledge`, `integration-verify`, `knowledge-distill`                                                               |
| `artifacts` / `wiring`             | Conditionally required | Required for `standard` and `integration-verify` (acceptance OR goal-backward)                                                             |
| `wiring_tests` / `dead_code_check` | No                     | Extended verification                                                                                                                      |
| `before_stage`                     | No                     | Pre-spawn checks (TruthCheck list); stage → Blocked if any fail                                                                            |
| `after_stage`                      | No                     | Post-acceptance checks (TruthCheck list); completion fails if any fail                                                                     |
| `code_review`                      | No                     | `integration-verify` only: `dimensions` (string list) and `require_all` (bool); rendered as checklist in agent signal                      |
| `model`                            | No                     | Model for this stage's main agent (default `opus` for every stage type)                                                                    |
| `reasoning_effort`                 | No                     | `low`, `medium`, `high`, `xhigh`, `max` (default `high` for every stage type and model)                                                    |
| `implementers`                     | No                     | Licensed agent lanes as a list, first = preferred for routine work: `["codex", "claude"]`. Default `["claude"]`. Listing a lane makes it available, not mandatory — a stage mixes lanes per subagent |
| `ultracode`                        | No                     | License this stage for large multi-agent fan-out; per-stage opt-in (default `false`)                                                       |
| `subagent_timeout_secs`            | No                     | Seconds of tool silence before the monitor warns `appears hung` (default 300); the advisory idle budget death is judged against — never the `--timeout` passed to `loom subagents watch` (that stays long, 3600) |
| `context_ceiling_tokens`           | No                     | Absolute resident-token ceiling for this stage's session (minimum 60000). Resolved stage value → plan-level `context_ceiling_tokens` → 150000. The session hook warns at 80% and blocks at 100%; the daemon forces a handoff at 125% |
| `plan_overview`                    | No                     | Set `false` to suppress the embedded plan overview in this stage's signal                                                                  |
| `sandbox`                          | No                     | Per-stage sandbox override                                                                                                                 |
| `sandbox.permission_mode`          | No                     | `auto` (default), `accept-edits`, `plan`, `default` — resolves stage > plan > stage-type default; `bypass-permissions` is rejected at init |
| `execution_mode`                   | No                     | `single` (default) or `team` hint                                                                                                          |

### Stage Type Behavior

- `knowledge`: knowledge/bootstrap work, different verification expectations
- `standard`: implementation stage; must define goal-backward checks
- `integration-verify`: final quality gate combining code review and functional verification; must define goal-backward checks. Define `code_review.dimensions` to render a checklist of review dimensions in the agent's signal.
- `knowledge-distill`: final stage; curates stage memories into permanent knowledge files

## Verification Model

`loom check <stage-id>` validates outcomes, not just compilation/tests:

- `acceptance`: shell criteria (simple strings or extended objects with `stdout_contains`, `exit_code`, etc.)
- `artifacts`: real implementation files exist
- `wiring`: critical integration links exist
- `wiring_tests`: runtime integration checks
- `dead_code_check`: detect unused code via command output patterns

For `standard` and `integration-verify` stages, acceptance criteria or at least one goal-backward check must be defined.

### Verification Is Enforced, Not Self-Reported

`loom stage complete` is the only way a stage finishes, and it runs the acceptance criteria itself before doing anything else. If they fail, the stage stays `Executing` — the agent must fix the work and re-run, and `fix_attempts` is incremented so repeated failures surface rather than accumulate silently. `after_stage` checks then run post-acceptance, and goal-backward verification runs before the progressive merge.

Artifact verification treats a stub as a failure: a file that exists but contains `TODO`, `FIXME`, `unimplemented!`, `todo!`, a bare `pass`, or `raise NotImplementedError` does not count as delivered.

Normal stage completion crosses a narrow control boundary. The stage runs one exact pinned
`loom stage complete <stage-id>` command; a PostToolUse bridge accepts only the matching stage and
session, requires Loom's verification marker, and sends a non-extensible `CompleteStage` request to
the daemon. The request cannot carry commands, paths, or bypass flags.

The three bypass flags — `--no-verify`, `--force-unsafe`, `--assume-merged` — are the operator's, and
cost the operator nothing: `loom stage complete <stage> --no-verify` just works from your shell. It
authorizes itself against `.work/admin.token`, which you can already read and a sandboxed agent
cannot (the sandbox binds the whole process tree, so a `loom` an agent spawns is denied the same
read). The proof is still bound to the project, stage, action, and exact flag set, and consumed on
first use — you simply never handle it.

The same applies to `loom stop`. Nothing asks a human to mint a credential they already hold; making
them carry an HMAC between two commands added ceremony, not security.

`loom stage admin-proof` remains for the case it was actually built for: a trusted broker minting a
narrowly-scoped capability for another process. It takes the secret through `LOOM_ADMIN_TOKEN` and
never reads the token file, so a caller that can invoke loom but cannot read that file gains nothing.

An agent that genuinely believes a criterion is wrong or impossible has a sanctioned path — `loom stage dispute-criteria` — rather than an incentive to weaken it.

### Verification Is the Main Agent's Job

Subagents do not verify. A subagent may run **at most one narrowly-scoped check** covering the files it just changed; project-wide builds, full test suites, and repo-wide lint or typecheck runs belong to the main agent — the only party that can see the whole tree and act on the result.

This is enforced, not just advised: `hooks/subagent-verify-guard.sh` (a `PreToolUse:Bash` hook) blocks project-wide runners — `cargo build`, `cargo test`, `make test`, `tsc`, `go build` and friends — when the caller is detected as a subagent. Scoped invocations pass, quoted mentions are ignored, and unrecognised commands are always allowed: a false block would strand a subagent mid-task.

Two things worth knowing:

- **`integration-verify` stages are carved out.** That stage type exists to run the complete suite, so its subagents may. The carve-out is read from the stage file and fails safe — an ambiguous or missing stage file means no relaxation.
- **There is deliberately no opt-out environment variable.** The main agent is never affected, so an escape hatch would only serve to defeat the rule.

## Knowledge System

Loom's answer to "every session starts from zero" is a three-stage pipeline: capture during execution, distill at the end of a plan, retrieve cheaply forever after.

### 1. Capture — session memory

While a stage runs, its agent journals to `.work/memory/<session>.md`:

```bash
loom memory note "gotcha: worktree exclude lives at <worktree>/.git/info/exclude, not <dir>/.git/..."
loom memory decision "centralized plan lookup in plan/parser" --context "avoids an orchestrator→commands layering violation"
```

Entries are typed (`note`, `decision`, `change`, `question`). The most recent are embedded in the *recitation* section at the end of the next signal — the position with the highest model attention — so a later stage inherits an earlier stage's hard-won detail instead of rediscovering it.

Memory is deliberately cheap and disposable. It is a working journal, not the deliverable.

### 2. Distill — memories become knowledge

A `knowledge-distill` stage runs at the end of a plan and performs the reduce step: it reads every stage memory, dedupes, and curates the survivors into permanent knowledge. Mistakes are rewritten as actionable prevention rules rather than anecdotes:

```markdown
## [Short description]

**What happened:** ...
**Why:** [root cause]
**Prevention:** [how to detect it earlier]
**Fix:** [what to do instead]
```

Procedural noise ("spawned agents", "ran tests") and anything recoverable from git history is dropped. `loom review` turns the same memories into a human-readable code-review document.

### 3. Retrieve — a tiered base agents can afford to read

Knowledge lives in `doc/loom/knowledge/` and is **tiered**: a generated `INDEX.md` (tier 0) maps the seven curated summary files (tier 1), which link out to per-category topic files (tier 2, e.g. `architecture/merge-flow.md`). Tier-1 files stay navigable summaries; detail lives in topics. The index is regenerated automatically on every knowledge write.

**Reading protocol** — agents read `INDEX.md` first for orientation, then the tier-1 summary for the area they are working in, then only the tier-2 topics they actually touch; a specific question is pulled with `loom knowledge context --query`, which returns the matching sections quoted. Inside a stage, the per-stage Knowledge Brief comes first. Loading the whole base defeats the point of tiering. Every session that starts inside a repository holding `doc/loom/knowledge/INDEX.md` also receives a one-paragraph pointer to it from the `knowledge-orient.sh` SessionStart hook, installed globally by `loom init`/`loom repair --fix`.

**Writing protocol** — when a tier-1 section grows past roughly 40 lines, move its body into a topic with `loom knowledge update <category>/<slug>` and leave a 2-4 line summary plus a relative link behind. Write the link as `[Title](category/slug.md)` in a tier-1 file: that is the tree's convention for references.

There is **no aggregate line budget** across the knowledge base. What matters is per-file size — roughly 250 lines for a tier-1 summary and 500 for a tier-2 topic — because structure is what degrades retrieval, not size.

**Retrieval is deterministic and offline.** `loom knowledge context --query <text>` returns a token-budgeted *context pack*: the tool chunks the curated prose, scores each chunk, fuses the per-channel rankings, and takes whole chunks in order until the budget is spent, always reporting what it left out. There is **no embedding model, no network call and no randomness** — a pack is a pure function of the bytes on disk and the query string, so the same query returns the same pack.

```bash
loom knowledge context --query "how does merge cleanup order work" --budget-tokens 3000
loom knowledge context --query "source graph coverage" --explain   # per-item scores and why each was selected
loom knowledge context --query "sandbox rules" --json              # machine-readable
```

Stage sessions do not have to ask. Signal generation embeds a per-stage **Knowledge Brief** built through the same single entry point, so what a stage receives at spawn and what you get from the CLI are produced identically. Loom records what each recipient was given, so a second retrieval in the same session skips what the first already quoted rather than repeating it.

`--scope` selects which channels to search: `knowledge` searches the curated prose, `source` searches the derived source graph of symbols extracted from the code, and `all` (the default) fuses both. The two are ranked separately — prose over chunk text, symbols over their scope and signature — and then fused, so one pack can mix curated prose with the exact symbols a query names. A symbol whose file the parser could not fully read is still returned, but without a high-confidence claim.

**The source graph maintains itself.** `loom init` and `loom run` publish a base layer for the current revision before anything else starts, and each stage's working-tree overlay is refreshed immediately before its signal is written — so a stage's brief describes the code as that stage will actually find it. Publication is advisory: when it cannot run, loom prints one line and carries on, because a missing graph must degrade retrieval rather than block a run. A base layer is keyed to a revision and so is never published from a dirty tree; there, `loom knowledge sync` builds a working-tree overlay instead and tells you which layer it wrote.

Codex's native `PostToolUse:apply_patch` hook records every patched path into the stage overlay, and its `UserPromptSubmit` hook uses the same retrieval entry point as Claude. Shell-based edits remain outside file-tool hook visibility and are reconciled by the next explicit graph refresh.

Run `loom knowledge sync` after editing knowledge outside the CLI; it reports whether both derived layers are current, naming the source-graph layer it produced — `base`, `local-overlay` or `skipped`, with the reason.

### Bootstrapping and maintenance

Agents populate knowledge during orchestration: a knowledge-bootstrap stage writes CONTENT, while `loom init` scaffolds the directory automatically. `loom knowledge sync` rebuilds the derived retrieval artifacts.

Knowledge directories created before the hierarchy existed stay **flat** and keep working unchanged — neither reading nor updating migrates them behind your back. `loom knowledge sync` performs the opt-in upgrade, creating `INDEX.md` the first time it runs on a flat directory.

Knowledge writes are protected by the sandbox defaults: agents update knowledge through `loom knowledge ...`, never by editing the files directly.

## Model Allocation

Every stage's main agent is an **orchestrator**, and orchestration is never economized:

| Stage type           | Default model | Default effort |
| -------------------- | ------------- | -------------- |
| `standard`           | `opus`        | `xhigh`        |
| `knowledge`          | `opus`        | `xhigh`        |
| `integration-verify` | `opus`        | `xhigh`        |
| `knowledge-distill`  | `opus`        | `xhigh`        |

The orchestrator decomposes the work, hands each subagent full context, then verifies and commits. It does not implement. Implementation is delegated to as few subagents as the work allows, each spawned **by agent type** so the model choice is explicit:

| Agent                            | Model                          | Use for                                                                                                                                |
| -------------------------------- | ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `loom-software-engineer`         | Sonnet                          | Common implementation and integration tests to detailed instructions                                                                   |
| `loom-codex-forwarder`           | Codex GPT-5.6 Terra or Luna     | Codex lane, licensed only on stages listing codex in `implementers`: Terra for common implementation/integration tests, Luna for boilerplate, scaffolding, and simple unit tests |
| `loom-senior-software-engineer`  | Opus                            | Mainstream architecture and algorithm implementation, complex debugging, security-sensitive or cross-cutting work                       |
| `loom-code-reviewer`             | Opus                            | Read-only code, security, and architecture review                                                                                       |
| `loom-advisor`                   | Fable                           | Diagnosis after a repeated failure — advice returned, nothing written                                                                   |

Fable-tier implementation — major bugs, visual/UI design, extremely challenging algorithmic design — has no dedicated agent type; it is spawned with an explicit model override rather than relying on inheritance.

The `loom-codex-forwarder` row additionally depends on the `codex` CLI and its plugin's companion runtime being installed. `loom run` checks this at startup and prints an advisory warning if either is missing — it never blocks the run — and terra-/luna-tier work falls back to Sonnet for the duration; the stage signal states the fallback explicitly and does not spawn `loom-codex-forwarder`.

This is why savings come from delegation rather than downgrade: an untyped subagent silently inherits the stage's Opus model, making every worker expensive. Two failures on the same task should produce a `loom-advisor` diagnosis, not a blind retry at a larger model.

Override per stage with `model` and `reasoning_effort` (`low`, `medium`, `high`, `xhigh`, `max`). `ultracode: true` licenses a stage for large multi-agent fan-out; it is per-stage opt-in so the cost decision stays explicit.

## Sandbox Configuration

Loom supports plan-level defaults plus stage-level overrides.

```yaml
loom:
  version: 1
  sandbox:
    enabled: true
    auto_allow: true
    filesystem:
      deny_read:
        - "~/.ssh/**"
        - "~/.aws/**"
        - "../../**"
        - "../.worktrees/**"
      deny_write:
        - "../../**"
        - "doc/loom/knowledge/**"
      allow_write:
        - "src/**"
    network:
      allowed_domains: ["github.com", "crates.io"]
      additional_domains: []
      allow_local_binding: false
      allow_unix_sockets: []
```

Note: knowledge file writes are intentionally protected by sandbox defaults; knowledge updates should be done via `loom knowledge ...` commands. Plan-configured `excluded_commands` are rejected because broad executable exemptions bypass the host sandbox. When sandboxing is enabled, generated settings use host `denyRead` rules for sensitive paths and `failIfUnavailable: true`; failure to write those settings blocks session spawn. Unit tests pin the generated policy and blocked-spawn behavior. A credentialed Claude host-runtime canary across Bash, interpreters, build scripts, symlinks, and file tools remains a manual release check.

### Command Confinement

The `sandbox:` block above bounds the **agent session**. A separate control bounds the commands **loom itself runs from your plan** — every acceptance criterion, setup command, truth check, wiring test, dead-code check and change-impact command:

```yaml
loom:
  version: 1
  sandbox:
    command_confinement: confined # plan-level default; `inherit` to opt out

  stages:
    - id: build
      sandbox:
        command_confinement: inherit # per-stage override
```

| Level | Behavior |
| ---------- | ------------------------------------------------------------------------------------ |
| `confined` | **Default.** The child process environment is cleared and rebuilt from a fixed allowlist |
| `inherit`  | The child inherits loom's ambient environment                                          |

Plans are trusted artifacts, but trusted is not privileged: under `confined`, a plan line cannot read `GITHUB_TOKEN`, `AWS_*` or `ANTHROPIC_API_KEY` merely because you started loom from a shell that had them. The allowlist carries what a build toolchain needs to find itself — `HOME`, `PATH`, `CARGO_HOME`, `RUSTUP_HOME`, locale and terminal variables, `TMPDIR`, the proxy variables and the CA-bundle *locations* (`SSL_CERT_FILE`, `SSL_CERT_DIR`, `NIX_SSL_CERT_FILE`). `SSH_AUTH_SOCK` is deliberately withheld, so an acceptance criterion that needs SSH auth fails by design rather than silently borrowing your agent.

> **What confinement is not.** It is environment scrubbing — least-privilege hygiene, not a security boundary. Loom applies **no** namespace, seccomp, landlock, cgroup or network isolation to the commands it spawns: a confined command shares your network namespace, can read and write any path your user can, and can reach any Unix socket on the host. The `network:` settings above are emitted into the *agent session's* sandbox and do not restrict plan-authored commands. Use `confined` to keep ambient credentials out of plan commands; do not use it to run code you would not run yourself.

### Permission Mode

All stages default to `auto` (agents auto-accept any action their heuristics deem safe, since loom stages run autonomously with no human to answer prompts; the sandbox deny/allow rules are the safety boundary). Override per-plan or per-stage to tighten control:

```yaml
loom:
  version: 1
  sandbox:
    permission_mode: accept-edits # plan-level override

  stages:
    - id: my-stage
      sandbox:
        permission_mode: plan # stage-level override (takes precedence)
```

Valid values: `auto` (default), `accept-edits`, `plan`, `default`. `bypass-permissions` is rejected at init time.

### Remote Control

Claude Code's `--remote-control` flag lets the loom orchestrator drive spawned Claude sessions programmatically. Loom enables it automatically when prerequisites are met — no configuration required.

**Prerequisites (preflight check):**

- **Claude version** ≥ 2.1.51
- **Auth**: claude.ai login — loom accepts **either** credential store: `~/.claude/.credentials.json`, **or** (on macOS) a `Claude Code-credentials` entry in the **Keychain**, which is where Claude Code stores credentials on macOS instead of the file. Additionally, none of these env vars may be set: `ANTHROPIC_API_KEY`, `CLAUDE_CODE_OAUTH_TOKEN`, `CLAUDE_CODE_USE_BEDROCK`, `CLAUDE_CODE_USE_VERTEX`, `CLAUDE_CODE_USE_FOUNDRY`

The flag exits non-zero when its prerequisites are not met, so loom never passes it blindly. When preflight fails, loom falls back silently to standard mode and prints a one-line advisory at orchestrator startup (e.g. `⚠ Remote Control disabled: <reason>`).

**Configuration** — the `[remote_control]` section of `.work/config.toml` carries a single switch:

```toml
# .work/config.toml
[remote_control]
mode = "auto"   # default: enable whenever preflight passes
# mode = "off"  # never enable, regardless of preflight
```

Toggling `mode` takes effect on the next session spawn — no daemon restart needed.

**Mid-run fallback** — if a session crashes within 15 seconds of spawn while Remote Control is active, loom writes a `.work/remote_control-unsupported` marker, then respawns and omits the flag for the rest of the run.

**Session naming** — every spawned session is named after its stage in the Remote Control UI: the stage name for stage sessions, and `Merge: <stage name>`, `Base conflict: <stage name>`, `Knowledge: <stage name>` for merge, base-conflict, and knowledge sessions respectively. Claude binaries whose `--remote-control` flag doesn't accept a name argument automatically fall back to the bare flag — detected via a one-time `claude --help` capability check, no configuration needed.

## Terminal Backends

Loom spawns each stage's Claude Code session through a terminal backend. Two are available.

| Backend            | Default | Sessions run in                     | Needs a GUI? |
| ------------------ | ------- | ----------------------------------- | ------------ |
| `native`           | ✅ yes  | a host terminal emulator window     | yes          |
| `tmux`             | opt-in  | a detached tmux server (no window)  | no           |

The `native` backend opens a real terminal window per session — you watch stages run in your own
terminal emulator. It requires a detectable emulator, so it cannot run headless.

The `tmux` backend spawns each session into a detached tmux server instead, which makes loom usable
over SSH, on a headless Linux box, or anywhere no terminal emulator exists. **tmux must be installed
and on `PATH`.**

### Selecting a backend

```toml
# .work/config.toml
[terminal]
backend = "tmux"   # or "native" (default)
```

Or from the CLI:

```bash
loom init <plan> --backend tmux   # skips the interactive backend prompt
loom run --backend tmux           # persists the choice to [terminal]
```

`loom init` prompts for a backend when run interactively; with no TTY it defaults to `native`.
Changing the backend while the daemon is running is refused with a hint — `loom stop` first, then
re-run with `--backend`. Selecting a backend takes effect on the next spawn.

If tmux is selected but not installed, loom prints an advisory warning at `loom init` and at
`loom run` startup and **never aborts** — sessions fall back to the native lane.

### Running under WSL

WSL2 runs the published `loom-linux-x86_64` binary unmodified — it is an ordinary glibc ELF, and git
worktrees, the `.work/` Unix socket and PID liveness checks all behave as they do on native Linux.

What does not carry over is the `native` backend. It opens a real terminal-emulator window per
session, and a stock WSL install ships no Linux GUI stack — without WSLg or an X server there is no
emulator to detect. Select tmux explicitly:

```bash
sudo apt install tmux jq          # both required; ripgrep and fd are recommended
loom init doc/plans/PLAN-<name>.md --backend tmux
loom run --backend tmux
```

Sessions then run in detached tmux servers, and you watch them from a Windows Terminal tab:

```bash
loom status --live       # live ledger dashboard
loom status --web        # browser dashboard: starts at 7373, then the next free port
loom attach              # tiled overview of every live session
loom attach <stage-id>   # attach to one stage
```

On Windows 11 with WSLg, an emulator installed inside WSL is detectable and the native backend does
work, but tmux remains the better fit: a run survives closing the window, and `loom attach`
reconnects to one already in progress.

Two WSL-specific notes:

- **Keep the repository on the Linux filesystem** (`~/src/…`), not under `/mnt/c`. Loom creates a
  worktree per parallel stage and the daemon polls stage files every 5s; 9p latency across the
  Windows drive makes both crawl.
- **ARM64 Windows** has no published Linux ARM64 binary. Install the Rust toolchain inside WSL and
  build from source.

### One tmux server per session

Loom does **not** put every stage in one shared tmux server. Each session gets its own server on its
own socket, named `loom-<session-id>` under `$TMUX_TMPDIR` (else `/tmp`).

This is deliberate: a wedged or killed server takes down exactly one stage, instead of every stage
running in parallel. Liveness is tracked from PID files rather than by asking tmux, because a tmux
server whose agent process has died still reports the session as existing — which would hide the
crash from loom's monitor and prevent the retry.

### Attaching

```bash
loom attach              # tiled overview of every live session
loom attach <stage-id>   # attach directly to one stage's session
```

With no argument, `loom attach` builds a per-repo viewer window with one pane per live session, tiled.
With a stage id it attaches straight to that stage. Both require a real terminal (a TTY), and both
work only for sessions spawned by the tmux backend — with the native backend it tells you so.

Panes in the overview are **live, writable terminals**, not read-only views: keystrokes go to the
agent, and `C-b x` closes that stage's pane. Detach the normal way with `C-b d`.

> **⚠️ Mouse interaction with a pane could kill the agent (fixed; mechanism below).**
>
> Two independent paths led from the mouse to a dead stage, both ending in
> `[server exited unexpectedly]` over `Pane is dead (status 1)`, a filed crash, and a retry:
>
> 1. **With `mouse on`** (inherited from `~/.tmux.conf`), tmux's **default** root-table bindings are
>    armed — no custom binding needed. `MouseDown3Pane` opens a menu whose entries include
>    `Kill → kill-pane`; each overview pane hosts a stage's own server running one agent, so that
>    kill ends the stage.
> 2. **Even with `mouse off`**, the agent itself enables all-motion mouse tracking, tmux mirrors that
>    mode out to your terminal, and forwards the resulting drag events back into the agent. The agent
>    treats the drag as a TUI text selection and copies it by running `tmux load-buffer -w -` against
>    its stage server — and tmux 3.6a **crashes** serving `load-buffer -w` with a client attached
>    (reproducible with `printf x | tmux load-buffer -w -` inside any pane while attached). The
>    server dies, the agent gets SIGHUP, and the stage reads as crashing on its own. This is also why
>    mouse selection appeared not to work at all: your drag was consumed as app mouse events.
>
> Loom now forces `mouse off` **and** deletes the `kmous` capability
> (`terminal-overrides[99]` = `*:kmous@`) on every server it creates. The first disarms tmux's own
> mouse bindings; the second stops any loom server from ever putting your terminal into mouse mode,
> so drags stay ordinary terminal-emulator selection and no mouse event reaches the agent.
> **Servers started by an older loom keep the old behaviour** — fix them in place:
>
> ```bash
> for s in "${TMUX_TMPDIR:-/tmp}"/tmux-$(id -u)/loom-*; do
>   tmux -S "$s" has-session 2>/dev/null &&
>     tmux -S "$s" set -g mouse off \; set -g 'terminal-overrides[99]' '*:kmous@'
> done
> ```
>
> (Then detach and re-attach: a terminal already switched into mouse mode stays there until the
> client reconnects.)
>
> Two related traps. `set-clipboard off` in your config means a tmux copy lands in a tmux buffer and
> never reaches your system clipboard — use `tmux -S <socket> capture-pane -p -S -` to get text out
> instead. And before hand-killing any session with `kill-server`, install a loom containing
> `89c4f350`: older daemons read a manual kill as a stage crash and spend the stage's retry budget
> on it.

### Fallback marker

If a tmux spawn fails — or tmux is configured but unavailable — loom retries on the native backend and
writes a marker file:

```text
.work/terminal-backend-fallback
```

While that marker exists, **every** subsequent spawn uses the native backend, and it survives daemon
restarts. Loom only writes it when the native backend is actually usable; on a headless box, where the
native lane cannot be built, loom keeps tmux selected and reports the real tmux error instead of
silently disabling the one backend that works there.

Clear it by explicitly re-selecting tmux:

```bash
loom run --backend tmux
```

`loom clean --state` (or `--all`) also removes it, as a side effect of deleting `.work/` entirely.

## Agent Teams (Experimental)

Loom enables agent teams in spawned sessions (`CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`) and injects team-usage guidance into stage signals.

Use teams when work needs coordination/discussion across agents (multi-dimension review, exploratory analysis). Use subagents for independent, concrete file-level tasks.

## State Layout

```text
project/
├── .work/
│   ├── config.toml
│   ├── stages/
│   ├── sessions/
│   ├── signals/
│   └── handoffs/
├── .worktrees/
├── doc/plans/
└── doc/loom/knowledge/
    ├── INDEX.md            # generated tier-0 map
    ├── architecture.md     # tier-1 summaries
    ├── patterns.md
    ├── ...
    └── architecture/       # tier-2 topics, one directory per category
        └── merge-flow.md
```

## Shell Completions

Loom provides context-aware tab completions for all commands, subcommands, flags, and dynamic values (stage IDs, plan files, session IDs, knowledge files).

### Quick Install

```bash
loom completions --install
```

Auto-detects your shell from `$SHELL` and writes completions to the standard location:

| Shell | Install Path                                      |
| ----- | ------------------------------------------------- |
| Bash  | `~/.local/share/bash-completion/completions/loom` |
| Zsh   | `~/.zfunc/_loom`                                  |
| Fish  | `~/.config/fish/completions/loom.fish`            |

Follow the printed post-install instructions to activate (e.g., for zsh, ensure `fpath=(~/.zfunc $fpath)` appears before `compinit` in `~/.zshrc`).

### Manual Setup

You can also write the completion script to a file yourself:

```bash
# bash
loom completions bash > ~/.local/share/bash-completion/completions/loom

# zsh — ensure ~/.zfunc is in fpath (add before compinit in ~/.zshrc):
#   fpath=(~/.zfunc $fpath)
#   autoload -Uz compinit && compinit
mkdir -p ~/.zfunc
loom completions zsh > ~/.zfunc/_loom

# fish
loom completions fish > ~/.config/fish/completions/loom.fish
```

### Migrating from Older Versions

Older versions of loom used `clap_complete` and required an `eval` line in your shell RC file that ran a subprocess on every shell startup. The new system writes a static script to disk and only calls `loom` at actual tab-completion time, which means faster shell startup and completions that work even before `loom` is in your `PATH`.

To check whether you need to migrate:

```bash
loom completions --migrate
```

This scans for two things:

1. **`eval` lines** in RC files (`.bashrc`, `.zshrc`, etc.) like `eval "$(loom completions zsh)"` — these should be removed
2. **Stale completion files** containing old `clap_complete` markers — these need to be regenerated

If issues are found, follow the printed instructions. Typically: remove the `eval` line from your RC file, then run `loom completions --install` to write the new file-based completion script.

### What's Completed

- Commands and subcommands (`loom stage <TAB>` shows all stage subcommands)
- Flags (`loom run --<TAB>` shows available flags)
- Stage IDs with smart filtering (`loom stage complete <TAB>` shows only executing stages)
- Plan files, session IDs, knowledge files (including aliases like `deps`, `tech`)
- Model names, trigger types, and more

## License

MIT
