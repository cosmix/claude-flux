# Session Identity Env

> The wrapper script's `LOOM_*` exports are a contract read by hooks, the CLI and the daemon. Two long-standing defects in that contract made knowledge stages impossible to complete.

## An OS-Resource Name Is Not an Identifier (2026-08-10)

**What happened:** a knowledge stage finished with 26/26 acceptance criteria green and could not
transition. Its session environment carried `LOOM_STAGE_ID=knowledge-knowledge-bootstrap`; the real
stage is `knowledge-bootstrap`.

**Why:** `Session::derive_tracking_key` builds `loom-[<kind>-]<stage-id>` to namespace **OS
resources** — terminal window titles, tmux session names, PID files — so a merge session and its
stage session never collide. `prepare_session_launch` stripped only the `loom-` and handed the rest
to the wrapper as `LOOM_STAGE_ID`. For `Stage` sessions the two forms coincide, which is why this
survived: the bug is invisible on the only kind anyone routinely exercises. Pre-existing since long
before the spawn unification (`fc6b1b0c`), whose comment preserved the value deliberately "so hook
behavior is unchanged".

**Blast radius — every consumer resolves that value as a real stage id:**

| Consumer | Failure with the prefixed form |
| --- | --- |
| `hooks/loom-control-complete.sh` (PreToolUse) | pins completion to a nonexistent stage id and **blocks** the correct command |
| `sandbox_control_session` | `env_stage != stage_id` → "completion request does not match the active wrapper stage/session" |
| `hooks/session-start.sh` → `HeartbeatWatcher` | heartbeat written as `<kind>-<stage>.json`; `detect_heartbeat_events` looks up `session.stage_id`, gets `NoHeartbeat` **forever** — hung detection silently disabled for knowledge/merge/base-conflict |
| `loom memory note\|decision` | entries filed under a phantom stage |
| `loom handoff`, `session-end.sh`'s `*-${LOOM_STAGE_ID}.md` glob, `ask-user-pre.sh` | resolve the wrong stage |

**Prevention:** `LOOM_STAGE_ID` is the plain plan stage id for **every** session kind. Anything that
varies by kind travels as the kind, not as a prefix on an id — `create_wrapper_script` now takes
`SessionType` and derives `LOOM_MERGE_SESSION` from it. That flag used to be
`stage_id.starts_with("merge-")`, a sniff that both breaks under this fix and would fire on a plan
stage legitimately named `merge-anything`. When a value serves two audiences (OS resource naming vs.
identity), give each its own field rather than deriving one from the other by string surgery.

## Presence of an Env Var Is Not Membership (2026-08-10)

**What happened:** with the stage id corrected by hand, the same stage then failed with
`sandboxed completion supports worktree stages only`. Knowledge stages run in the **main repo with
no worktree**, yet `sandbox_control_session` classified the session as a sandboxed worktree agent
and refused it. Both walls had to fall for the stage to complete; fixing either alone leaves it
stuck.

**Why:** `create_wrapper_script` derived `LOOM_WORKTREE_PATH` from its `working_dir` parameter,
which every session kind passes (it is also the `cd` target — one parameter serving two roles).
Merge, knowledge and base-conflict sessions `cd` into the repo root, so all three exported the var
pointing at the main repo. `sandbox_control_session` (added 2026-08-09 by `888e190e`) then read bare
presence of the three `LOOM_*` vars as "this is a sandboxed worktree agent". The latent export
became load-bearing the day it was made a gate.

**The asymmetry that should have caught it:** `NativeBackend::spawn`/`TmuxBackend::spawn` already
carried a `set_worktree_path: bool` that is **false** for those three kinds — the `Session` record
correctly omitted the worktree path. Only the env var ignored the distinction.

**Prevention:** decide worktree membership **structurally**, never by presence. A loom worktree is
`<repo>/.worktrees/<stage-id>`; the main repo root is not. `hooks/_common.sh`'s
`loom_current_worktree()` has always required this and carries the reasoning ("that variable leaks
into plain Claude Code sessions"); the Rust gate and `loom-control-complete.sh` did not, and both
now do (`is_loom_worktree_path`, and a `=~ /\.worktrees/[^/]+` guard in the hook). When one surface
already encodes a rule with a comment explaining why, a new surface implementing the same concept
must adopt the rule, not re-derive a weaker one.

**Detection rule:** any new consumer of `LOOM_WORKTREE_PATH`, `LOOM_STAGE_ID` or `LOOM_SESSION_ID`
must be exercised against a **knowledge** session, not only a standard one. Standard stages are the
one kind where the prefixed id and the plain id coincide and where the worktree path is real, so
they cannot fail either way.

## Answering "Should Knowledge Stages Complete Via the Daemon Broker?" — No (2026-08-10)

Investigating the above, the obvious-looking alternative was to route knowledge completion through
the same daemon broker worktree stages use. Three independent blockers, recorded so it is not
re-proposed:

1. `validate_active_identity` (`daemon/server/control_complete.rs`) requires
   `session_type == SessionType::Stage`; knowledge sessions are `SessionType::Knowledge`.
2. `handle_complete_stage` calls `try_complete(None)` and never sets `merged = true`. Knowledge
   stages have no branch, so nothing else ever would — the plan could never reach `DONE-`.
3. Nothing in the daemon calls `trigger_dependents`; `complete_knowledge_stage` does it itself.

The broker exists because a *sandboxed worktree* agent must not mutate trusted `.work` state. A
knowledge session is not sandboxed — `spawn_knowledge_stage` installs hooks into the main repo but
generates no sandbox deny/allow settings — so the in-process path is correct for it, and the gate
simply had to stop claiming it.

## Per-session identity persisted in settings env blocks goes stale and shadows the wrapper env (2026-07-22)

**What happened:** Inside worktree sessions, `$LOOM_STAGE_ID`/`$LOOM_SESSION_ID` reported the IDs of the plan's FIRST stage (the knowledge stage) instead of the executing one — six stages later. Unqualified `loom memory` calls filed entries into the wrong stage's journal (hit across 2+ plans, 4+ stages), and hooks heartbeat the wrong session.
**Why:** Three writers persisted per-session identity into settings files: (1) knowledge-stage spawns wrote `LOOM_STAGE_ID`/`LOOM_SESSION_ID` into the MAIN repo's `.claude/settings.local.json` env block and nothing ever cleared it; (2) worktree creation copied that file wholesale into new worktrees; (3) `refresh_worktree_settings_local` (triggered by permission propagation on every `loom stage complete`/crash sync) rebuilt each worktree's settings.local.json FROM THE MAIN REPO'S COPY as base, clobbering the worktree's fresh env/hooks/defaultMode mid-session. Claude Code applies settings `env` OVER the process environment, so the stale settings values silently shadowed the wrapper script's correct exports. (`LOOM_MAIN_AGENT_PID` had already hit this exact failure and been removed from settings env — the lesson wasn't generalized to the other identity vars.)
**Prevention:** INVARIANT: per-session identity (`LOOM_MAIN_AGENT_PID`, `LOOM_STAGE_ID`, `LOOM_SESSION_ID`) is exported ONLY by the wrapper script (`pid_tracking.rs` template) and must NEVER be written into any settings file. Any settings-file `env` write of a value that varies per session is a staleness bug by construction — settings env overrides process env. When merging/copying settings across checkouts, the destination's session-specific config (env, hooks, defaultMode) must win; only permissions are unioned.
**Fix:** `fs/permissions/settings.rs::scrub_session_identity_env()` (shared `SESSION_IDENTITY_ENV_KEYS` scrubber) applied in `generate_hooks_settings`, `create_worktree_settings`, the worktree settings.local.json copy, `refresh_worktree_settings_local` (which now uses the worktree's own settings as merge base), and `ensure_loom_hooks_local` (self-heals existing installs on `loom init`/`repair`). `HooksConfig` no longer carries stage/session IDs at all.
