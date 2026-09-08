# Hook System

> Hook embedding and install, the SessionStart hookSpecificOutput contract, and the two subagent enforcement hooks.

## Hook System Architecture (hooks/)

The `hooks/` module provides Claude Code hooks integration for session lifecycle management, plus a Codex-native subset installed through `fs/permissions/codex_hooks.rs`. It is a **top-level module** — currently imported by `orchestrator/` and `git/worktree/`, which is a known layering violation (both should import a stable hooks interface instead).

**Layering:** `hooks/` is used by `orchestrator/core/stage_executor.rs` (worktree hook setup) and `git/worktree/settings.rs` (settings injection). The intended fix is to extract hooks as a fully independent top-level module with no reverse imports.

**Global vs session hooks distinction:**

- **Global hooks** include commit filtering, Git-add protection, Bash isolation, the canonical five-tool file guard, plan-path protection, `prefer-modern-tools.sh`, and the forwarding guard. They are installed under `~/.claude/hooks/loom/` and registered by `fs/permissions/hooks.rs`, so they persist across sessions. `prefer-modern-tools.sh` lives here as a global `PreToolUse:Bash` hook (`fs/permissions/hooks/config.rs:25`) — there is no `PreferModernTools` `HookEvent` variant (deleted); it never was one of the session hooks below.
- **Session hooks** (session-start.sh, post-tool-use.sh, pre-compact.sh, session-end.sh, learning-validator.sh, subagent-start.sh, subagent-stop.sh): generated fresh per-session by `hooks/generator.rs:generate_hooks_settings()` from the **7** `HookEvent`s that `HooksConfig::to_settings_hooks()` (`hooks/config.rs:183`) emits, derived by iterating `HookEvent::all()` rather than seven hand-written blocks. Merged into worktree's `settings.local.json` with duplicate detection.

`LOOM_HOOKS` (the full inventory: session hooks, global `PreToolUse` guards, compatibility bridges, and sourced-library hooks like `_common.sh`/`_read_discipline.sh`/`_read_ledger.sh`) is 33 rows; Claude's global `PreToolUse` registration alone is 47 entries, including `spawn-guard.sh` (Task+Agent), `read-guard.sh` (Read), and `poll-guard.sh` (Bash). Installers iterate `LOOM_HOOKS`; trigger configuration and tests remain separate registration surfaces — see [Registration Sites for a New Hook](../entry-points/hooks.md).

### Codex-native subset

`install_codex_hooks_to` installs the same embedded asset inventory under
`~/.codex/hooks/loom/` and non-destructively merges Loom-owned rules into
`~/.codex/hooks.json`. User rules and top-level metadata survive; malformed JSON is refused rather
than replaced. Codex registers `SessionStart:knowledge-orient`,
`UserPromptSubmit:user-prompt-context`, the Bash-compatible guards, and an `apply_patch` bridge.
The bridge presents each patch target to the canonical file guards as a synthetic `Write` or
`Edit`, then records successful patch paths with `loom context record-edit` so the stage source
overlay can be reconciled on the next prompt.

The stage lifecycle hooks are intentionally absent from the Codex-global config. A Codex worker
forwarded from a Loom stage inherits the parent `LOOM_*` variables; registering `session-start`,
`pre-compact`, or `session-end` there would let the worker claim, hand off, or end its parent's
session. Codex requires explicit trust for changed non-managed hooks, surfaced through `/hooks`.

## Hook System — Session-Start Behavior and hookSpecificOutput Pattern

### session-start.sh Behavior (Updated 2026-06-15)

- Captures stdin into a variable (not drained) using cross-platform gtimeout/timeout/cat, 1s timeout
- Validates LOOM_STAGE_ID, LOOM_SESSION_ID, LOOM_WORK_DIR — silently exits if missing
- Writes initial heartbeat: `.work/heartbeat/<LOOM_STAGE_ID>.json`, through the shared
  ownership-checked lock and atomic-replacement protocol
- Logs SessionStart event to `.work/hooks/events.jsonl`
- **Parses `.source` field from stdin JSON**: when `.source == "compact"` or `"resume"`, emits `hookSpecificOutput.additionalContext` JSON with a re-anchor pointer (signal file path), redirecting the agent back to its signal after context compaction or resume
- Stdin must be captured (not drained with `>/dev/null`) so the source field can be parsed — same pattern as `post-tool-use.sh`

**Compaction recovery flow (current):**

```text
pre-compact.sh phase 1 → blocks compaction + creates handoff
pre-compact.sh phase 2 → allows compaction
Claude Code emits SessionStart with source="compact"
session-start.sh → parses source → emits hookSpecificOutput additionalContext re-anchor
```

### hookSpecificOutput JSON Pattern

Used by hooks to inject context into Claude's next turn:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "<EventType>",
    "additionalContext": "<string content>"
  }
}
```

**Examples:**

- `prefer-modern-tools.sh`: PreToolUse warning when a command-position token invokes `grep`/`find`
- `skill-trigger.sh`: UserPromptSubmit skill suggestions
- `session-start.sh`: SessionStart re-anchor pointer on compact/resume source

**Why JSON over plain text:** Claude Code has reliability issues with plain-text stdout from certain hook types (see issue claude-code#13912); JSON additionalContext is more reliable for context injection.

**Construction:** Always use `jq -nc --arg ctx "..."  '{hookSpecificOutput: {hookEventName: "...", additionalContext: $ctx}}'` — never manually escape JSON strings.

**jq availability:** a hook must never fail open just because jq is missing. Blocking guards
(header documents exit 2) call `loom_require_jq` at the top of the script, right after sourcing
`_common.sh`; advisory hooks call `loom_warn_no_jq` instead — both defined in `hooks/_common.sh`.
Lifecycle hooks (`session-start.sh`, `post-tool-use.sh`, `subagent-start.sh`, `subagent-stop.sh`)
keep their own explicit `command -v jq` skip rather than either helper. See
[stack.md](../stack.md#hook-runtime-dependencies-jq-rg-fd) for the Rust-side preflight/repair checks.

### PostToolUse context-ceiling boundary

`hooks/post-tool-use.sh` owns transcript-tail usage measurement and threshold messaging, but it owns no
configuration parsing. It calls the hidden, deterministic `loom hook context-ceilings` command,
which uses Rust's stage loader and `ContextConfig` TOML deserializer and returns
`<main>:<subagent>`. The hook validates and caches that complete pair per Loom session before
selecting the branch for the classified caller. This keeps teammates/subagents on the plan-wide
subagent ceiling while the parent receives the stage-aware main ceiling, with one canonical config
read and no shell TOML/YAML grammar to drift from Rust. Missing, failed, malformed, or out-of-range
command output uses the shell fallback constants; a valid main zero is instead Rust's explicit
disabled sentinel for a stage record it could not verify.

### LOOM_* Env Vars Available to All Hooks

Set by wrapper script (pid_tracking.rs:463-479) before `exec claude`:

| Variable               | Purpose                                             |
| ---------------------- | --------------------------------------------------- |
| `LOOM_SESSION_ID`      | Current session ID                                  |
| `LOOM_STAGE_ID`        | Current stage ID                                    |
| `LOOM_WORK_DIR`        | Absolute path to `.work/`                           |
| `LOOM_MAIN_AGENT_PID`  | Process PID (set dynamically, NOT in settings.json) |
| `LOOM_WORKTREE_PATH`   | Absolute worktree path (worktree sessions only)     |
| `LOOM_MERGE_SESSION=1` | Set for merge resolution sessions only              |

**Per-session identity gotcha (LOOM_MAIN_AGENT_PID, LOOM_STAGE_ID, LOOM_SESSION_ID):** Must NOT be in ANY settings-file env block — settings `env` overrides the process environment, so a persisted value from an earlier session shadows the wrapper's fresh exports (wrong-stage `loom memory` entries, heartbeats for the wrong session, commit-filter misidentifying the main agent). The wrapper script is the ONLY writer; `fs/permissions/settings.rs::scrub_session_identity_env()` strips these keys wherever settings are generated, copied, or merged (`generate_hooks_settings`, `create_worktree_settings`, worktree settings.local.json copy, `refresh_worktree_settings_local`, `ensure_loom_hooks_local`). Only the stable `LOOM_WORK_DIR` is persisted in settings env. `refresh_worktree_settings_local` merges main-repo permissions INTO the worktree's own settings (worktree base wins for env/hooks/defaultMode). **Claude Code applies the MAIN repo's settings env to sessions in linked worktrees** (observed v2.1.217), so worktree-side scrubbing alone is insufficient — the run path heals the main files too: `scrub_main_repo_settings_identity()` at `loom run` startup (`prepare_repo_for_run`) and `scrub_session_identity_env()` inside the sync fold-back (`merge_permissions_with_lock`), which rewrites the main settings.local.json on every stage completion (see mistakes.md 2026-07-23).

### Hook Embedding (constants.rs)

`LOOM_HOOKS` (`fs/permissions/constants.rs`) holds **32 entries**, each embedded via `include_str!()` at compile time. `install_loom_hooks()` writes them to `~/.claude/hooks/loom/` with mode 0o755. Hooks are NOT read from disk by loom at runtime.

**Do not read "32 entries" as "32 hooks."** The arithmetic, verified against `fd -t f -e sh . hooks --max-depth 1 | wc -l` and `rg -c '^    ("' loom/src/fs/permissions/constants.rs`:

```text
33 top-level scripts in hooks/
 −1  git-pre-commit-hook.sh    (excluded from LOOM_HOOKS; appended to .git/hooks/pre-commit by loom init)
 ───
 32  LOOM_HOOKS entries installed to ~/.claude/hooks/loom/
 −3  _common.sh, _read_discipline.sh, _read_ledger.sh   (sourced libraries, not registered hooks)
 −1  codex-forward.sh          (wrapper the codex forwarding lane invokes directly; not a registered Claude Code hook)
 ───
 28  actual Claude Code hooks, registered in fs/permissions/hooks/config.rs
```

So: **28 Claude Code hooks + 3 shared libraries + 1 codex wrapper + 1 git-side hook = 33 scripts**
in `hooks/` (plus `hooks/tests/`).

One of those 28 registered hooks is `knowledge-orient.sh`, the only hook registered **globally** on `SessionStart` (`fs/permissions/hooks/config.rs`) rather than per-session — it points a fresh non-stage session at `doc/loom/knowledge/INDEX.md`. It exits silently inside a stage, on `compact`/`resume`, and when no `INDEX.md` exists inside the git root.

Re-derive these with the two commands above rather than trusting the numbers here — they have gone stale before.

## Subagent Isolation

Three-layer defense: documentation (CLAUDE.md Rule 5), signal injection (cache.rs prefix), and hook enforcement — which is now **two** hooks, not one:

| Hook                       | Enforces                                                                                                                                                                                                                |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `commit-filter.sh`         | subagents may not run git operations; blocks AI attribution in commit messages. Matches argv TOKENS (raw-regex fallback on an unterminated quote), so quoted prose about git is not a git invocation                     |
| `subagent-verify-guard.sh` | **still matches raw command strings** (see concerns.md). subagents may not run project-wide build/test/lint/typecheck suites — at most one narrowly-scoped check. `integration-verify` stages are carved out and may run the full suite. Deliberately has **no opt-out env var** |

**Detection is payload-first, not a PPID comparison.** Both hooks gate on `loom_is_subagent()` in
`hooks/_common.sh`, which first requires `LOOM_MAIN_AGENT_PID` to be a **live ancestor** of the
current process — this scopes the globally-installed hooks to a loom stage session. Once that
passes, it classifies the caller from the hook's stdin JSON payload via `loom_payload_agent_verdict`
(`.agent_type` / `.transcript_path` / `.session_id`, which the caller cannot forge): a Task-spawned
subagent always carries a non-empty `.agent_type` and is classified SUBAGENT immediately, and a
main-session payload is recognized by its main-shaped `.transcript_path` and classified MAIN
immediately. Only a payload-less caller, or one whose verdict is "unknown", falls back to the
process-tree walk (`find_nearest_claude_ancestor` / `count_claude_processes_between`). In that
FALLBACK ONLY, a 2-level claude chain is classified MAIN AGENT and a 3-level chain is a SUBAGENT
(verified empirically with `COMMIT_FILTER_DEBUG=1`).

**Consequence worth knowing:** agent-team _teammates_ are not in the main agent's process tree,
so `LOOM_MAIN_AGENT_PID` is set but is not a live ancestor and `loom_is_subagent` returns false
for them before either check runs. Globally installed enforcement hooks gated on it therefore do
**not** fire inside teammates. The per-session `hooks/post-tool-use.sh` is intentionally different: its
validated `LOOM_*` identity already scopes it to the stage, so a positive payload verdict marks a
teammate as a subagent before ancestry. That keeps the parent heartbeat intact and applies the
subagent ceiling. A Task-tool subagent, by contrast, runs **in-process** (the same claude process
as the main agent), so the process-tree walk alone finds no intervening Claude process between it
and `LOOM_MAIN_AGENT_PID` — payload identity is what classifies both shapes correctly.

### Heartbeat writer protocol

`hooks/session-start.sh`, `hooks/post-tool-use.sh`, and `hooks/subagent-stop.sh` all write the same stage-keyed
heartbeat. Every writer acquires a portable `mkdir` lock, validates the stage's current
`session:` owner while holding it, and replaces the JSON via a same-directory temp-file rename.
Late hooks from an old session therefore cannot overwrite a successor, concurrent subagent
refreshes cannot roll the parent's token count backward, and the Rust watcher never observes a
partially truncated JSON document. Lock metadata permits conservative recovery after a dead
writer leaves an abandoned lock; live or uncertain owners are never stolen.
