# Terminal Backends

> The native and tmux session backends behind one dispatcher, lane resolution, and session-recorded dispatch.

## Two Lanes Behind One Dispatcher

`SessionBackend` (`orchestrator/terminal/backend.rs`) is the single type every spawn/kill/liveness
call goes through. It wraps two concrete lanes:

| Lane               | Type                                 | Where sessions run                      |
| ------------------ | ------------------------------------ | --------------------------------------- |
| `Native` (default) | `NativeBackend` (`terminal/native/`) | a host terminal emulator window         |
| `Tmux` (opt-in)    | `TmuxBackend` (`terminal/tmux/`)     | a detached tmux server, no GUI required |

`SessionBackend::from_config(work_dir)` **always succeeds** as long as the config parses — it does not
construct either lane eagerly. That matters: `NativeBackend::new` runs `detect_terminal()` subprocess
probes and _fails_ on a headless box, which is exactly where the tmux lane is wanted. Each native
detection/process-discovery probe has a two-second deadline, so terminal discovery cannot stall the
single scheduler loop.

The native lane is therefore built lazily and memoized in a `OnceLock<Result<NativeBackend, String>>`
— **including the failure**. `OnceLock` (not `RefCell`/`Mutex`) because the orchestrator holds
`SessionBackend` behind an `Arc` and the monitor thread calls it via `&self`, so it must stay
`Send + Sync`. The error is stored as `String` because `anyhow::Error` is not `Clone` and callers only
render it as text. Terminal availability is fixed for the life of the daemon, so memoizing the failure
is free — before this, a post-fallback `is_session_alive` re-ran `detect_terminal()` once per session
per 5s monitor tick.

## Configuration and Lane Resolution

- **Config:** `[terminal]` / `backend = "native" | "tmux"` in `.work/config.toml`.
  `TerminalConfig` (`models/session/types.rs:85-89`) holds one `SessionBackendKind`
  (`types.rs:62-70`, `#[serde(rename_all = "lowercase")]`, `#[default] Native`).
  Helpers `read_terminal_config` / `write_terminal_config` (`fs/work_dir.rs:509-516`); a missing
  section falls through to `~/.loom/config.toml`'s `terminal.backend`, THEN the built-in default —
  absence is the inheritance channel, not a synonym for native. Written at init by
  `commands/init/plan_setup.rs`, and only when someone chose explicitly.
- **CLI:** `--backend <native|tmux>` on both `loom init` (skips the interactive prompt) and
  `loom run` / `loom run --foreground` (persists to `[terminal]`). `loom run`'s
  `resolve_backend_flag` (`commands/run/mod.rs:118-158`) is shared by both run paths.
- **`loom init`'s prompt inherits** (`commands/init/backend.rs`): pressing Enter returns `None`, so
  no `[terminal]` section is written and the repo follows the user config live; typing `native` or
  `tmux` pins that value into the workspace. Before this, the prompt hardcoded `native` in three
  places — its label, its empty-input arm and its EOF arm — so an operator with
  `terminal.backend = "tmux"` in `~/.loom/config.toml` was shown `(native)`, pressed Enter, and got
  native written into the workspace, shadowing their global preference for the life of the repo. The
  user-scope key was dead for every interactively-initialised repo. The tmux-missing-from-PATH
  warning keys off `effective_backend` (explicit choice, else the user config) precisely because an
  inherited tmux backend now arrives as `None`.
- **Per-spawn resolution** — `SessionBackend::resolve_lane` (`backend.rs:125-128`) picks `Native` if
  the fallback marker exists, else the configured kind if `which tmux` succeeds, else `Native`.

## Session-Recorded Backend Dispatch

`Session.backend: SessionBackendKind` (`#[serde(default)]`, `models/session/types.rs:119-123`) records
the lane **actually used**, and is persisted to `.work/sessions/<id>.md`.

This is the load-bearing part: sessions are reconstructed from disk after a daemon restart, so
kill/liveness must route on the _session's_ recorded backend, never on the currently-configured one.
`SessionBackend::is_session_alive` and `kill_session` dispatch on `session.backend`, so a run that
flipped config (or fell back) still kills and monitors older sessions through the lane that spawned
them.

## One tmux Server Per Session — Crash Containment

The tmux lane deliberately does **not** use one shared server with many windows. Each session gets its
own server on its own socket:

- socket name — `format!("loom-{}", session.id)` (`tmux/mod.rs:41-43`)
- socket dir — `$TMUX_TMPDIR` else literal `/tmp`, joined with `tmux-<uid>` (`tmux/socket.rs:23-30`)
- tmux session name inside that server — `session.tracking_key` (`models/session/methods.rs:75-78`),
  so merge/knowledge/base-conflict prefixes resolve correctly

**Why per-session:** a wedged or killed server takes down exactly one stage. A shared server is a
single point of failure for every parallel stage at once.

**Why keyed on `session.id`, not `stage_id`:** `sun_path` is capped at 104 bytes on macOS; plan stage
ids run up to 128 chars and would silently blow the limit. See `mistakes/tmux-backend.md`.

## Liveness Uses Verified Process Identity, Not tmux

`TmuxBackend::is_session_alive` and the native/headless lanes consult the shared
`process::ProcessIdentity { pid, start_time }` service. There is deliberately no `tmux has-session`
fallback for liveness and no raw `session.pid` fallback for signaling.

A tmux server whose pane process has died but which has not yet reaped itself still answers
`has-session` with exit 0. Consulting it would make the monitor report a dead `claude` as alive, and
the crash would never be filed or retried — defeating the containment property the backend exists for.
The recorded start time is mandatory identity evidence. A mismatch means the recorded process is
dead; missing or unreadable evidence is `Unverifiable`. Neither outcome may signal the numeric PID.
The shared identity service gives native, tmux, headless, and daemon integrations the same fail-closed rule.

## Spawn Path and the Silent-Failure Guard

`spawn_in_tmux` cannot trust `new-session`'s exit code — tmux can print an error to stderr and still
exit 0. Two checks follow the spawn:

1. `evaluate_new_session(socket, status_success, stderr)` — a **pure** decision fn (no tmux, no
   filesystem) so the rule is unit-testable; it treats any stderr with exit 0 as failure.
2. an authoritative `tmux has-session` probe against the socket.

Every error path _after_ the server may exist routes through teardown — killing the socket server and
calling `native::cleanup_stage_files` to drop the PID/wrapper files — before returning `Err`. Skipping
either leaks a live agent (see `mistakes/tmux-backend.md`).

After a successful spawn, `PRESENTATION_OPTIONS` is applied best-effort: `status off`, `mouse off`,
and `terminal-overrides[99]` = `*:kmous@`. The last one is load-bearing: it deletes the `kmous`
capability for every client TERM so the server can never put an attached terminal into mouse mode —
otherwise claude's own all-motion mouse tracking is mirrored out to the operator's terminal, drags
are forwarded back into the agent, and claude's clipboard copy (`tmux load-buffer -w -`) crashes
tmux 3.6a. Full chain in `mistakes/tmux-backend.md`.

## Fallback Marker: `.work/terminal-backend-fallback`

A **sticky** marker that forces every subsequent spawn onto the native lane.

- **Written** (`backend.rs:192` after a tmux spawn failure; `backend.rs:218` when configured tmux is
  unavailable) — but **only when a native lane is actually constructible**. With no native lane loom
  writes nothing and surfaces the original tmux error, rather than degrading to a retry that cannot
  succeed and a marker that permanently disables the one backend that works headless.
- **Read** by `resolve_lane` (`backend.rs:125`) — present ⇒ `Native`, unconditionally.
- **Cleared** by `clear_fallback_marker` (`backend.rs:67-69`), whose only production caller is
  `loom run --backend tmux` (`run/mod.rs:141`). `loom clean --state` / `--all` clears it only as a side
  effect of deleting `.work/` wholesale. Nothing else clears it — it survives daemon restarts.

Fallback is announced with an `eprintln!` warning naming the re-enable command; in daemon mode that
goes to the daemon's stderr, not the user's shell.

## `loom attach` — Overview and Direct

`commands/attach/` discovers sessions with `backend == Tmux`, status `Running | Spawning`, a live PID
and a resolvable tmux session name, sorted oldest-first by `(created_at, id)`. `mod.rs` owns
discovery and direct attach; `overview.rs` owns the viewer server.

Both paths then apply one further precondition: `endpoint_ready`
(`orchestrator/terminal/tmux/viewer.rs`) — the session's socket exists AND `has-session` succeeds on
it. Attach and the viewer reconciler (below) are the only legitimate `has-session` callers, and it
is not a contradiction of the liveness rule above: PID liveness answers "is the agent alive",
attaching needs "is the server accepting clients", and the two disagree in both directions (a
`Spawning` session has a live wrapper PID before its server is up; a torn-down server can outlive
its `claude` PID by a moment). Attach's call sites report "still spawning, or just ended — re-run in
a moment" rather than letting tmux's error surface.

Viewer identity (socket name, `loom-overview` session name, the nested-attach pane command) and
attachability discovery live in `orchestrator/terminal/tmux/viewer.rs`, shared by attach's one-shot
build and the reconciler so the two can never disagree; `commands/attach/overview.rs` keeps only the
build sequence (`build_overview_argv`, `VIEWER_HARDENING`).

- **No argument** ⇒ a tiled **overview**: a per-repo _viewer_ server on socket
  `loom-view-<sha256(canonical repo root)[..8]>`, session `loom-overview`, created detached at
  220x50, one pane per attachable session, re-tiled after **each** split.
  - `VIEWER_HARDENING` runs **before** `new-session`, as one `;`-separated sequence:
    `start-server ; set -g exit-empty off ; set -gw remain-on-exit on ; set -g mouse off ;
    set -g terminal-overrides[99] '*:kmous@' ; set -g remain-on-exit-format`. The indexed
    `kmous@` entry is idempotent on purpose — this sequence re-runs against the same long-lived
    viewer server on every `loom attach`.
    Pane 0 is born already running an attach client, so only a GLOBAL option can protect it — the
    targeted `remain-on-exit` step after `new-session` is a belt-and-braces re-assertion. The
    sequence is best-effort: a tmux rejecting part of it degrades to the pre-hardening behaviour
    rather than failing the attach. See `mistakes/tmux-backend.md`.
- **With a stage id** ⇒ direct `exec` of `tmux -L loom-<session.id> attach-session -t <tracking_key>`,
  newest session wins if several match.

## Live Overview Reconciliation (Daemon-Side)

The overview is no longer a one-shot snapshot. `Monitor::poll` calls
`tmux::refresh_attached_viewer` (`orchestrator/terminal/tmux/reconcile.rs`) once per scheduler tick,
best-effort: a failed pass is logged at `warn` (it was `debug` until 2026-08-26, which hid every
later kill/add behind one bad step) and never fails the poll. The daemon never CREATES the viewer —
only `loom attach` does — it maintains one the operator already built. Gate order keeps the common
case free: viewer-socket `stat` (no subprocess, no session reads, no log when nobody is attached) →
bounded `has-session` (skip, logged at `debug` with the refusal/error, while absent or
mid-`loom attach` rebuild) → `list-panes -F` → pure diff (`reconcile_steps`, in
`reconcile/steps.rs` — the executor stays in `reconcile.rs`) → apply.

Both processes must resolve the same tmux socket directory: the orchestrator records its
`TMUX_TMPDIR` in `.work/tmux-tmpdir` at start (`fs/tmux_tmpdir.rs`, removed at exit) and
`loom attach` adopts it while a daemon is alive, so a shell with a different `TMUX_TMPDIR` cannot
make the reconciler stat the wrong directory. The daemon's `work_dir` is absolute (`loom run`
passes its cwd, never `.`) because `viewer_socket_name` hashes the canonical repo root and a
relative path silently diverges on any `canonicalize` failure.

Panes are attributed by parsing the inner socket out of `#{pane_start_command}` (survives for
`new-session` and `split-window` panes; verified tmux 3.6a) with a `loom-` prefix requirement, so an
operator's own splits — including a manual `tmux attach-session` — are `None`-attributed and never
touched. Diff rules: missing attachable session ⇒ `split-window` re-tiled after EVERY split (the 6+
pane trap); dead pane whose server still lives ⇒ `respawn-pane`; dead pane whose session is gone ⇒
`kill-pane`, but never the last pane (a killed last pane collapses window and session despite
`exit-empty off`); duplicate panes for one socket (attach-rebuild race) collapse to one keeper; all
adds, then all respawns, then all kills, then ONE `select-layout tiled` iff any pane was killed.
`select-layout` steps are cosmetic: a failed one is logged at `debug` and the pass continues; every
other verb stops the pass on first failure. Every tmux call is bounded (`run_tmux_control` + probe
timeout) because it runs on the single scheduler loop. `tests/e2e/tmux_reconcile.rs` drives
`reconcile_viewer` against a real tmux (add, kill, floor); it skips loudly where AF_UNIX bind is
denied — every Claude Code sandbox — so it only truly runs on a developer host or CI.

Each overview pane nests into an inner server by running `unset TMUX; exec tmux -L <sock>
attach-session -t <key>` — passed as **separate `sh -c` argv words**, not one string, so the shell is
guaranteed POSIX (see `mistakes/tmux-backend.md`). `build_overview_argv` is a pure builder taking the
viewer socket and `(session_socket, tracking_key)` pairs as parameters, so it is testable without
tmux; socket derivation stays in `tmux::socket_name` and `viewer_socket_name`.

`loom attach` requires a TTY (`require_tty`), so its `exec` paths are unreachable from a non-TTY
harness — but the empty-set, unknown-stage and multi-match messages are emitted _before_ the TTY check
and are testable.

## Socket Reaping at init/clean

`cleanup_orphaned_sessions` reaps only sockets **positively attributed to this work dir** — never
"every `loom-*` socket with no matching session file", because the socket dir is per-user and would
match another checkout's live servers.

`SessionReapMode` distinguishes the two callers: `OrphansOnly` (normal path, skips live sessions) vs
`IncludeLiveBeforeClean`. `loom init --clean` deletes `.work/` immediately afterwards, and `.work/` is
the _only_ thing that makes attribution possible — so a live session left running through `--clean`
would become permanently unattributable and leak forever. `--clean` therefore reaps attributed sockets
even when alive; the normal path stays conservative.

`list_loom_sockets` skips names starting `loom-view-` so the overview viewer is not reported as
unattributable. Nothing currently reaps that viewer socket (see `concerns.md`).

## Entry Points (2026-08-08)

**Dispatcher and lanes**

- `loom/src/orchestrator/terminal/backend.rs` — `SessionBackend`: `from_config`, `resolve_lane`
  (`:125-128`), `dispatch_spawn`, the fallback-marker helpers (`:45-69`) and the lazy `OnceLock` native
  lane. Tests in `terminal/backend/tests.rs`.
- `loom/src/orchestrator/terminal/tmux/mod.rs` — `TmuxBackend`, `socket_name` (`:41-43`),
  `spawn_in_tmux`, `evaluate_new_session`, `kill_session`.
- `loom/src/orchestrator/terminal/tmux/socket.rs` — `loom_socket_dir` (`:23-30`), `socket_path_for`, `list_loom_sockets`, `socket_session_is_alive`, `kill_socket_server`.
- `loom/src/orchestrator/terminal/native/pid_guard.rs` — the deduped PID-file liveness layers shared by
  both lanes (six former copies).
- `loom/tests/e2e/tmux_backend.rs` — external e2e; `TmuxTmpDirGuard` works around the sandbox. Note
  `tests/` can only reach `pub` items, never `pub(crate)`.

**`[terminal]` config**

- `loom/src/models/session/types.rs` — `SessionBackendKind` (`:62-70`, `native`/`tmux`, default
  `native`), `TerminalConfig` (`:85-89`), `Session.backend` (`:119-123`).
- `loom/src/fs/work_dir.rs` — `TERMINAL_SECTION` (`:347`), `read_terminal_config` /
  `write_terminal_config` (`:509-516`).
- `loom/src/commands/init/plan_setup.rs:179-182` — writes the section at init.

**`--backend` flag**

- `loom/src/cli/types.rs` — on `loom init` (`:41-43`) and `loom run` (`:68-70`),
  `value_parser = ["native", "tmux"]`.
- `loom/src/commands/run/mod.rs:118-158` — `resolve_backend_flag`, shared by `run/mod.rs:46` and
  `run/foreground.rs:32`. Rejects unknown values, refuses to persist a change while the daemon is
  running (prints a restart hint), clears the fallback marker when selecting tmux, and runs the
  advisory `which tmux` preflight that never aborts.
- `loom/src/commands/init/execute.rs:184-226` — `resolve_backend_choice`; flag wins, else prompts when
  both stdin and stdout are TTYs, else defaults to native.

**`loom attach`**

- `loom/src/commands/attach/mod.rs` — `execute` (`:67-79`), live-session discovery (`:115-141`),
  `viewer_socket_name` (`:161-177`), `build_overview_argv` (pure), `run_overview`, `attach_direct`
  (`:346-400`), `require_tty` (`:414-419`). Tests in `attach/tests.rs`.
- `loom/src/cli/types.rs:101-106` — `Attach { stage_id: Option<String> }`; dispatch at
  `cli/dispatch.rs:48`. Shell completions complete live stage ids after `loom attach`.

## macOS Terminal Detection Priority

1. `LOOM_TERMINAL` env var (explicit override)
2. `TERMINAL` env var (user preference)
3. Parent process detection (walks process tree up to 10 levels via `ps`)
4. Cross-platform binary check (ghostty, kitty, alacritty, wezterm via `which`)
5. macOS native apps (`/Applications/Ghostty.app`, `/Applications/iTerm.app`, `Terminal.app` fallback)

Note: `$TERM_PROGRAM` is NOT checked.

## find_claude_path() (src/claude.rs)

Shared binary resolution: `which::which("claude")` -> `~/.claude/local/claude` -> `~/.local/bin/claude` -> `~/.cargo/bin/claude` -> `/usr/local/bin/claude` -> `/opt/homebrew/bin/claude`.
