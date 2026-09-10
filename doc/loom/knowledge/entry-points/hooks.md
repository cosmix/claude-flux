# Hooks

> Every hook script and the event it binds to, _common.sh's command-matching and subagent-detection helpers, and the registration sites a new hook needs.

## Hooks

- `hooks/*.sh` - the hook scripts (commit-guard.sh, commit-filter.sh, etc.); `skill-trigger.sh` is Python despite its extension
- `fs/permissions/hooks.rs` - `install_loom_hooks()`, `configure_loom_hooks()`; `loom_hooks_config_for_dir` builds the global registration table from `fs/permissions/hooks/config.rs`
- `fs/permissions/codex_hooks.rs` - installs assets to `~/.codex/hooks/loom/` and merges `~/.codex/hooks.json`
- `fs/permissions/settings.rs` - `ensure_loom_permissions()`, `scrub_session_identity_env()`; `create_worktree_settings()` lives in `git/worktree/settings.rs`
- `fs/permissions/constants.rs` - embedded hook scripts via `include_str!()` (`LOOM_HOOKS`)
- `hooks/config.rs` - `HookEvent` enum and `HooksConfig`; the session-hook module is the top-level `loom/src/hooks/`, not the `orchestrator/hooks/` an earlier version of this list named
- `hooks/generator.rs` - `setup_hooks_for_worktree()`, `generate_hooks_settings()`, `find_hooks_dir()`
- `commands/hook/` - the `loom hook` delegates the shell hooks call: `user_prompt.rs` (`loom hook user-prompt`, behind `user-prompt-context.sh`), `pre_compact.rs`, `reconcile_graph.rs`, `context_ceilings.rs`, and `project_types.rs` (behind `skill-trigger.sh`)
- `commands/hook/target.rs` - `HookTarget`, the one stage-or-checkout scope resolution the user-prompt, pre-compact and reconcile-graph delegates share
- `telemetry/mod.rs` - `TelemetryEvent`; the prompt hook appends `prompt-brief` and `prompt-abstained` events, which `loom knowledge telemetry` summarizes

Skill-trigger scoring, the prompt-hook wrapper and `HookTarget` are described in [Hook System](../architecture/hook-system.md).

## Shared Hook Utility

- `hooks/_common.sh` - Source guard + `strip_embedded_content()` + the `loom_tokenize_command` / `loom_tokens_*` command-matching helpers + `loom_is_subagent` — sourced by all PreToolUse hooks. MUST be installed alongside hooks (in `~/.claude/hooks/loom/`). Registered in `constants.rs` as `HOOK_COMMON`.

## Hook System (loom/src/hooks/)

- `hooks/mod.rs` - Module root; re-exports `HookEvent`, `HooksConfig`, `generate_hooks_settings`, `setup_hooks_for_worktree`, `find_hooks_dir`, and the `events` types
- `hooks/config.rs` - `HookEvent` enum (7 variants: `SessionStart`, `PostToolUse`, `PreCompact`, `SessionEnd`, `Stop`, `SubagentStart`, `SubagentStop`) + `HooksConfig` struct + `to_settings_hooks()`
- `hooks/generator.rs` - `generate_hooks_settings()` (merge session hooks into settings), `setup_hooks_for_worktree()`, `find_hooks_dir()`
- `hooks/events.rs` - `log_hook_event()`, `read_recent_events()`, event log CRUD
- `hooks/validators/` - Rust implementations of the rules `worktree-isolation.sh` enforces (`bash.rs`, `file_path.rs`), used for tests, pre-validation and detailed error messages; they are not hook scripts
- `hooks/codex-apply-patch.sh` - (repository `hooks/`, not `loom/src/hooks/`) translates Codex `apply_patch` targets into canonical file-guard payloads and records successful edits

**7 emitted session-hook events** (`HooksConfig::to_settings_hooks()`, derives the map by iterating `HookEvent::all()` rather than seven hand-written blocks, so the list and the map can no longer diverge):

| Event           | Script                   | Purpose                                                                  |
| --------------- | ------------------------ | ------------------------------------------------------------------------- |
| `SessionStart`  | `session-start.sh`       | Initial heartbeat                                                        |
| `PostToolUse`   | `post-tool-use.sh`       | Heartbeat update plus canonical context-ceiling enforcement after every tool call |
| `PreCompact`    | `pre-compact.sh`         | Trigger handoff before context compaction                                |
| `SessionEnd`    | `session-end.sh`         | Cleanup on normal exit                                                   |
| `Stop`          | `learning-validator.sh`  | Memory usage check on stop                                               |
| `SubagentStart` | `subagent-start.sh`      | Records `{agent_id, agent_type, stage_id, parent_session_id, loom_session_id, ts}` to `.work/subagents/<stage>/starts.jsonl`; usage joins by agent + Claude parent transcript UUID, while Loom ownership stays distinct |
| `SubagentStop`  | `subagent-stop.sh`       | Completion signal + heartbeat refresh when a Task-tool subagent finishes |

There is no `PreferModernTools` `HookEvent` variant any more — it was deleted. `prefer-modern-tools.sh` still runs, but through a separate path entirely: it is registered as a **global** `PreToolUse:Bash` hook in `fs/permissions/hooks/config.rs`, alongside `commit-filter.sh`/`git-add-guard.sh`/etc., never through `HookEvent`/`to_settings_hooks()`.

**Settings placement:** Session hooks → `<worktree>/.claude/settings.local.json` (`hooks/generator.rs`). Global hooks (commit-filter, git-add-guard, worktree-isolation and the rest) are configured by `fs/permissions/hooks.rs::configure_loom_hooks()` from the table in `fs/permissions/hooks/config.rs`; an earlier version of this line named a `fs/permissions.rs` module, which is now the `fs/permissions/` directory.

**Env vars injected via settings env block:**

- `LOOM_WORK_DIR` — path to `.work/` directory (the ONLY loom var persisted; stable per repo)

**Per-session identity (LOOM_MAIN_AGENT_PID, LOOM_STAGE_ID, LOOM_SESSION_ID):** Explicitly REMOVED from all settings env blocks (`scrub_session_identity_env` in `fs/permissions/settings.rs`). Set ONLY by the wrapper script exports so they always reflect the running session — settings env overrides process env, so persisted values from an earlier session would shadow the fresh exports (see mistakes.md 2026-07-22). Because Claude Code applies the MAIN repo's settings env to worktree sessions, the main-repo files are also healed in the run path: `scrub_main_repo_settings_identity` at `loom run` startup and inside the `sync.rs` fold-back (see mistakes.md 2026-07-23).

**Hooks discovery:** `find_hooks_dir()` checks `$LOOM_HOOKS_DIR` first (used when that path exists), then `~/.claude/hooks/loom/`. Returns `None` if neither exists.

**Permissions:** Absolute paths use `//` prefix in allow entries (e.g., `Read(//home/user/.work/signals/**)`). Single `/` means project-relative — wrong for `.work/` which resolves outside the worktree due to symlink.

## Hook Scripts — What Each Does

| Script                     | Hook Type                                  | Key Behavior                                                                                                                                                                                                                                              |
| -------------------------- | ------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `session-start.sh`         | SessionStart                               | Writes initial heartbeat; captures stdin and parses `.source` field; on `source == "compact"` or `"resume"` emits `hookSpecificOutput.additionalContext` JSON re-anchor pointer                                                                           |
| `knowledge-orient.sh`      | SessionStart (global)                      | Points a fresh non-stage session at `doc/loom/knowledge/INDEX.md`; exits silently inside a stage, on `compact`/`resume`, and when no `INDEX.md` exists inside the git root                                                                                |
| `post-tool-use.sh`         | PostToolUse                                | Updates private heartbeat metadata, caches `loom hook context-ceilings` output, and enforces the selected threshold; never persists tool commands or output                                                                                                |
| `pre-compact.sh`           | PreCompact                                 | Block-then-allow: first call exits 2 (blocks) + creates pending flag + calls `loom handoff`; second call exits 0 (allows); does NOT create a recovery marker file                                                                                         |
| `session-end.sh`           | SessionEnd                                 | Creates handoff if stage not completed                                                                                                                                                                                                                    |
| `learning-validator.sh`    | Stop                                       | Advisory check for session memory usage                                                                                                                                                                                                                   |
| `commit-guard.sh`          | Stop (global)                              | **Advisory only — always exits 0.** Warns about uncommitted changes or a stage still Executing. It stopped blocking because Claude Code fires Stop hooks during Task-tool waits, where a block killed the session before the agent could commit                                                                                                                                                                                               |
| `prefer-modern-tools.sh`   | PreToolUse:Bash                            | Warns (never blocks) when a command-position token invokes `grep`/`find`; token scan, raw-regex fallback. Emits `hookSpecificOutput.additionalContext`                                                                                                                                                                        |
| `commit-filter.sh`         | PreToolUse:Bash                            | Token scan (raw-regex fallback). Blocks subagent git commits via `loom_is_subagent()` (payload-first, process-tree fallback); attribution checks read the ORIGINAL command, since trailers live in the message body                                                                                                                                                        |
| `subagent-verify-guard.sh` | PreToolUse:Bash                            | **Still regexes raw strings** (last hook not converted; see concerns.md). Blocks **subagents** from running project-wide build/test/lint/typecheck suites; at most one narrowly-scoped check allowed; `integration-verify` stages carved out; unmatched commands are allowed (a false block strands a subagent); no opt-out env var |
| `git-add-guard.sh`         | PreToolUse:Bash                            | Token scan (raw-regex fallback). Blocks the all-files staging forms and staging of `.work`                                                                                                                                                                                                 |
| `worktree-isolation.sh`    | PreToolUse:Bash                            | Token scan (raw-regex fallback). Blocks git-dir overrides, `eval`, path traversal and cross-worktree paths; path checks consider word-shaped tokens only                                                                                                                                                                                               |
| `worktree-file-guard.sh`   | PreToolUse:Read/Write/Edit/Glob/Grep       | Canonical component-aware file boundary; blocks host paths, credentials, leaf symlinks, prefix siblings, and direct protected-state writes                                                                                                                |
| `plans-path-guard.sh`      | PreToolUse:Edit/Write                      | **Unconditional** (fires in interactive sessions too) — blocks plan writes under `.claude/plans/` or `.claude/projects/*/plans/`, redirecting to `doc/plans/PLAN-*.md`                                                                                    |
| `codex-forward-guard.sh`   | PreToolUse:Bash/Edit/Write/Read/Task/Agent | Pins forwarding agents to one exact, shell-parsed invocation of `codex-forward.sh`; rejects operators and missing classification metadata                                                                                                                 |
| `codex-forward.sh`         | Trusted forwarding executable              | Resolves the installed companion and invokes it with fixed argv, validated model/effort, and the task as one argument; when the outer sandbox refuses a nested Seatbelt (macOS) it runs `codex exec --sandbox danger-full-access` directly                |
| `git-pre-commit-hook.sh`   | git `pre-commit`                           | Blocks commits containing `.work` or `.worktrees`; appended to `.git/hooks/pre-commit` by `loom init`, not installed to `~/.claude/hooks/loom/`                                                                                                           |
| `skill-trigger.sh`         | UserPromptSubmit                           | Scores keywords, emits skill suggestions as `hookSpecificOutput.additionalContext`                                                                                                                                                                        |
| `ask-user-pre.sh`          | PreToolUse:AskUserQuestion                 | Marks stage WaitingForInput                                                                                                                                                                                                                               |
| `ask-user-post.sh`         | PostToolUse:AskUserQuestion                | Resumes stage                                                                                                                                                                                                                                             |
| `_common.sh`               | Utility (sourced, not registered)          | Exports the command-matching and subagent-detection helpers — see below                                                                                                                                                                                                                             |

### `hooks/_common.sh` Helpers

Public helpers hooks may call. Everything prefixed `_loom_*`, plus
`loom_tokens_command_word_index`, is INTERNAL — call the predicates below, not those.

| Function                                                                              | Role                                                                                                                                                                                                                      |
| ------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `strip_embedded_content()`                                                            | **Pre-step, runs before tokenizing.** Strips heredoc bodies and `-m`/`--message` text — a heredoc body is unquoted, so its words would otherwise tokenize as real command words. **Known limit:** cannot strip a multi-line `-m` body |
| `loom_tokenize_command()`                                                             | Walks a command with quote/escape state into `LOOM_TOKENS` (argv words plus a `%%SEP%%` sentinel per command boundary). Splices `sh -c` payloads. Returns non-zero on an unterminated quote or an exhausted splice budget — callers must then use their raw-regex fallback |
| `loom_token_is_word()`                                                                | True for a real word-shaped token (not the sentinel, no whitespace). The discriminator for path checks: a real path argument is whitespace-free, a prose payload is not                                                   |
| `loom_tokens_invoke()`                                                                | True when some command segment INVOKES the named command (basename match), seeing through `VAR=value` prefixes, wrappers (`sudo`/`env`/`xargs`/`timeout`…), shell keywords and command-prefix builtins                    |
| `loom_tokens_cmd_has_arg()`, `..._cmd_has_arg_pair()`, `..._cmd_argv()`               | True when a segment invoking a command carries a given argument / adjacent pair / positional argv[n]                                                                                                                      |
| `loom_tokens_word_matches()`                                                          | True when any word-shaped token matches an ERE (unanchored). Used for path traversal, `.worktrees/`, and `VAR=` env assignments                                                                                           |
| `loom_is_subagent()`                                                                  | **The subagent gate.** True only when `LOOM_MAIN_AGENT_PID` is a _live ancestor_, then payload-first via `loom_payload_agent_verdict` (falls back to an intervening-Claude-process walk only when the payload is missing or unrecognized). Returns false inside agent-team teammates (not in the main agent's tree) |
| `loom_current_worktree()`                                                             | Worktree detection by directory, NOT just the env var                                                                                                                                                                     |
| `loom_debug()`                                                                        | Gated debug logging                                                                                                                                                                                                       |
| `is_ancestor()`, `find_nearest_claude_ancestor()`, `count_claude_processes_between()`, `loom_payload_agent_verdict()` | Internal helpers for `loom_is_subagent` — documented as internal; hooks should call `loom_is_subagent`, never these                                                                                                       |

### Registration Sites for a New Hook

A hook that Claude Code itself invokes (a `PreToolUse` guard, a global `UserPromptSubmit` or `SessionStart` hook, or a session-lifecycle `HookEvent`) needs FOUR integration surfaces; the installer is not one of them. `install.sh` carries no hook inventory any more: it delegates to `loom install-assets`, which installs every hook embedded through `LOOM_HOOKS`, and `dev-install.sh` builds the binary and delegates to `install.sh`. (This section used to list two `all_hooks` arrays in `install.sh` as a fifth surface; those arrays are gone.) A SOURCED LIBRARY (like `hooks/_common.sh`, `hooks/_read_discipline.sh`, `hooks/_read_ledger.sh` — embedded and installed, but never invoked directly by the harness) needs every applicable surface below except a trigger:

1. The executable or sourced file under `hooks/`.
2. An `include_str!` const plus a `LOOM_HOOKS` entry in `fs/permissions/constants.rs`.
3. Its trigger: for a global hook, an entry in the table `fs/permissions/hooks/config.rs` builds (reached through `fs/permissions/hooks.rs::loom_hooks_config_for_dir`); for a session-lifecycle hook, a `HookEvent` variant in `hooks/config.rs` (`to_settings_hooks()` derives the emitted map from `HookEvent::all()`, so adding the variant is enough — no hand-written block to update). A hook Codex should also run needs its entry in `fs/permissions/codex_hooks.rs`. A sourced library has no trigger.
4. Tests: `fs/permissions/tests/hooks_tests.rs::test_hooks_config_structure` asserts the exact `PreToolUse` array length and per-index order (currently 47 entries) — or, for a session hook, `fs/permissions/tests/hooks_tests.rs::test_hook_event_surface_has_seven_events`, while `hooks/tests.rs` asserts the emitted map has one entry per `HookEvent::all()` — plus a `hooks/tests/` case registered in `hooks/tests/run-all.sh`, and the `setup_hook()` of every integration test harness that sources the new file (e.g. `hooks_read_guard.rs`, `hooks_poll_guard.rs`) if it is a sourced library.

`settings_checks.rs` renders `LOOM_HOOKS.len()` dynamically (`commands/repair/settings_checks.rs`), so it needs no edit when a hook is added — only the count assertions above do.

**Worktree detection gotcha:** `_common.sh:loom_current_worktree()` decides membership by LOCATION, never by `LOOM_STAGE_ID`, which leaks into plain sessions from prior runs. A session counts as inside a worktree when EITHER the current directory is inside `.worktrees/<stage>/`, OR `LOOM_WORKTREE_PATH` points into `.worktrees/` and that directory still exists on disk (the on-disk check rejects a stale, leaked value). An earlier version of this note said both conditions were required.
