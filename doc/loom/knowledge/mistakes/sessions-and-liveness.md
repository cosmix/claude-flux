# Sessions And Liveness

> Session identity, liveness routing, spawn-site coverage, and the blast radius of adding a session field.

## Session Identity: Backend Metadata Must Be Persisted

**Mistake:** Relying on transient session state to route kill/liveness calls after a daemon restart.

**Why:** Sessions are reconstructed from `.work/sessions/<id>.md` on daemon restart. Any field not in the session file is lost.

**Prevention:** Add `#[serde(default)]` to backend-related session fields and ensure they are set before the session is written to disk.

## Liveness: Monitor Must Route Through LivenessService

**Mistake:** Monitoring thread reads the PID from the session file and calls `kill -0 <pid>` directly.

**Prevention:** Always route session liveness through `LivenessService::is_alive(session)` and destructive actions through verified `ProcessIdentity`. Numeric PID liveness alone is not identity and must never authorize a signal.

**Fix:** `LivenessService` wraps the shared `Arc<SessionBackend>`, while `process::identity` verifies PID plus recorded start time. A mismatch is definitive death; missing evidence is unverifiable and fails closed.

## Run-Path Coverage: All Spawn Sites Must Use the Shared Backend

**Mistake:** Wiring a session-spawning change into the main orchestrator loop but forgetting the other spawn paths: foreground mode, daemon startup, merge resolver spawner, continuation (handoff) spawner, auto-merge spawner.

**Why:** Sessions are spawned from multiple entry points beyond the main orchestrator. Each missed path drifts from the shared `Arc<SessionBackend>` the orchestrator holds.

**Prevention:** When changing session spawning, `rg` for all `spawn_session\|spawn_merge_session\|spawn_knowledge_session` call sites before considering the work done. Typically 5+ sites: orchestrator main loop, foreground spawner, merge_handler, continuation, auto_merge.

## Session Liveness: Use tracking_key, Not stage_id

**What happened:** `kill_session` and `is_session_alive` in `orchestrator/terminal/native/mod.rs` used `format!("loom-{stage_id}")` for window titles and bare `stage_id` for PID key lookups. This worked for standard stages but silently missed merge sessions, knowledge sessions, and base-conflict sessions whose spawns use prefixed tracking keys.

**Why:** Standard stages dominate the mental model; their PID key and stage_id happen to align. But `Session.tracking_key` is the canonical OS-level resource identifier — it encodes the prefix/suffix needed for non-standard session types.

**Prevention:** Any OS-resource lookup keyed on a session (window title, PID file, process name) MUST use `session.tracking_key`, not `stage_id` or `format!("loom-{stage_id}")`. Verify by running a merge-resolver or knowledge session and checking that kill/liveness correctly targets it.

**Fix:** `native/mod.rs` updated to use `session.tracking_key` in all OS lookups.

## Adding Session Fields: ~15-20 Struct Literal Breakages

**Mistake:** Adding a field to `Session` struct and expecting `cargo build` to guide you to all the breakages. Test files in `tests/` are not compiled by default and may not show breakages until `cargo test`.

**Why:** Rust requires all struct fields to be initialized in struct literals (unless `..Default::default()` spread is used). `Session` is constructed explicitly in ~15-20 locations across `src/` and `tests/`.

**Prevention:** Use `..Session::default()` spread in all struct literals. When adding fields to Session/Stage/LoomConfig, run `cargo test --all` (not just `cargo build`) to catch `tests/` breakages. Alternatively, write a context-aware patch script.

## Timing: Missing Accumulation on Exit Transitions

**Mistake:** `accumulate_attempt_time` not called on `NeedsHandoff`/`BudgetExceeded`, permanently losing execution time.
**Fix:** Call `accumulate_attempt_time` on ALL exit transitions, not just `Completed`.

## Test Environment Race Condition

**Mistake:** `test_loom_terminal_env_var_takes_precedence` uses `std::env::set_var` without `serial_test`.
**Fix:** Use `#[serial]` attribute on tests that modify environment variables.

## CORRECTION (2026-08-08): "Adding Session Fields: ~15-20 Struct Literal Breakages" Is Now Wrong

**Supersedes the "~15-20 Struct Literal Breakages" section above — treat that count as obsolete.**

Adding `Session.backend` during the tmux-backend plan broke exactly **3** struct-literal sites, not
15-20: `src/commands/handoff/create.rs:98`, and `src/commands/stage/tests/session.rs` at `:52` and
`:146`. `Session` construction has migrated almost entirely to `Session::new()` / `new_merge()` /
`new_knowledge()` plus field mutation, so bare struct literals are now rare.

**What still holds, and is the part worth keeping:** `cargo build` alone does not surface breakages in
`tests/` — use `cargo test --all-targets --no-run`.

**Meta-lesson:** a knowledge entry that carries a _count_ is a decaying asset. Two stages sized their
work off this number before anyone checked it. Blast-radius numbers should be re-measured, not
inherited — and when you measure one, correct the entry in the same stage.

## `kill(pid, 0)` Reports an Unreaped Zombie as Alive (2026-08-10)

**What happened:** `tmux_liveness_ignores_running_server_when_pid_is_dead` failed in CI after
SIGKILLing a pane process, at a `wait_until(!is_process_alive(pid), 5s)` precondition. Not a slow
runner: the process had already exited, and the kernel keeps a zombie's PID entry (and its
`/proc/<pid>/stat` start-time) until the parent reaps it, so both halves of loom's liveness evidence
kept answering as though it were running.

**Why it mattered beyond the test:** `is_session_alive` → `pid_only_is_alive` →
`verify_process_identity` → `is_process_alive`. For a zombie the null signal succeeded AND the
recorded start-time still matched, producing `VerifiedAlive` — so a session whose process had died
read as healthy for as long as its parent went unreaped. Under tmux `remain-on-exit` (or any
terminal emulator slow to reap) that is indefinite: a dead session the orchestrator waits on forever.

**Prevention:** liveness must ask "can this process still run?", not "does this PID exist?". Signal
existence alone is not liveness. `is_process_alive` now consults `process_is_zombie` — `/proc/<pid>/stat`
state `Z` on Linux, `pbi_status == SZOMB` via `proc_pidinfo` on macOS — and answers false for a zombie.

**Detection rule:** a test that kills a process it did not fork and then polls for the death to be
observable is testing the _parent's_ reaping behaviour, not the kill. If the parent is tmux, a
terminal, or anything you do not control, expect the zombie window and assert on state rather than
existence. Demonstrated with a 20-line C harness: `kill(pid,0)` returns ALIVE with `/proc` state `Z`
between the SIGKILL and the parent's `waitpid`.

## In-Memory Dedup Makes Every Daemon Restart Replay Old Crashes (2026-08-10)

**What happened:** a live run in a sibling repo ended with TWO agents writing the same worktree. The
daemon was restarted (a `dev-install.sh` install does `pkill -x loom`); on its first poll it declared
a session that had died 25 minutes earlier crashed, charged that to the stage's retry budget, blocked
the stage, and auto-retried it — spawning a second agent into `.worktrees/weather-cache` while the
stage's real session was still alive and working there. The same restart also wrote a crash report
for `knowledge-bootstrap`, a stage already `completed` + `merged: true`.

**Why:** two `Detection`/`Orchestrator` fields that read as "we already handled this" are
**in-memory only** — `last_session_states` and `reported_crashes`. On startup both are empty, so
every session file on disk is a _first observation_. `detect_session_changes` tested
`previous_status != Some(current_status)`; with `previous_status == None`, a file already persisted
as `Crashed` looks exactly like a fresh Running→Crashed transition. Nothing downstream caught it:
`handle_session_crashed` loaded the stage named by the crashed session and acted on it **without
ever checking that the crashed session was the stage's current session**.

Session files accumulate forever — a stage that crashed and retried keeps every previous session on
disk with `stage_id` still set — so "this session names stage X" is a much weaker claim than "stage X
is being executed by this session". Only the second licenses a session to speak for its stage.

**Prevention:**

- A session may only speak for its stage when `stage.session == Some(session.id)`. Both the emitter
  (`monitor/session_events.rs`) and the authority that mutates the stage
  (`core/crash_handler.rs::stage_answerable_for_crash`) now enforce it — the emitter because the
  crash _report_ is written there, before any handler guard runs.
- Distinguish "first observation" from "transition". `previous.is_none()` is not a change. The
  correct rule is not "seed silently" either: a first observation that IS the stage's active session
  must still fire, or a stage stranded by a daemon that died between the crash and handling it sits
  `Executing` forever. Membership decides, not recency.
- **Detection rule:** any `HashSet`/`HashMap` on `Detection` or `Orchestrator` used as "already
  reported" is reconstructed empty on restart. Before relying on one, ask what the first poll after a
  restart observes; if the on-disk state alone can be misread as an event, the guard must be a fact
  about the data (identity), not a memory of having seen it.

**Symptom to recognise:** `loom attach` showing more panes than `loom status` shows stages, or a
stage `Blocked` while its tmux pane looks alive. The blocked-but-alive pane is usually the _reverse_
case — an un-reaped tmux server holding a dead pane (see the `kill(pid, 0)` note above and
`architecture/terminal-backends.md`); confirm with `/proc/<pid>`, never with `tmux has-session`.

## The Session Record Was Written After the Agent It Records (2026-08-29)

**What happened:** a daemon killed mid-spawn left a `claude` process running in tmux that loom
could not see. The stage sat `Executing` behind an hourglass, `loom attach` answered "No live tmux
sessions", orphan recovery found nothing to recover, and `loom stage reset` then queued a SECOND
agent into the same worktree. The second agent noticed the first, refused to work, and its
non-completing exit was read as a crash and retried.

**Why:** `stage_executor.rs` marked the stage `Executing`, spawned the agent, and only afterwards
wrote `.work/sessions/<id>.md` and linked `stage.session`. Every discovery path in the system reads
that one artifact — `viewer::live_tmux_sessions` (the sole input to `loom attach`),
`recover_orphaned_sessions`, and `status`'s `load_all_sessions`. Two compounding errors: a record
written AFTER the thing it records cannot describe a crash in between, and keying every consumer on
a single artifact turns its absence into total blindness rather than a degraded view. The agent's
OS-level evidence — the wrapper's pid file, the tmux socket named for the session — was on disk the
whole time and nothing looked at it.

**Prevention:** write the record that makes a thing discoverable BEFORE creating the thing, and
order the two writes so that a crash between them leaves the harmless state (an inert record with
no process) rather than the harmful one (a live process nothing references). When adding a consumer
of session state, ask what it does when the record is missing but the process is not: if the answer
is "reports absence", it is asserting something it cannot know.

**Fix:** the record is saved as `Spawning` and `stage.session` assigned in the same locked update
that marks the stage `Executing`, all before the spawn; every pre-spawn failure deletes it again.
`orchestrator/session_registry.rs` reads the pid-file and socket evidence back and rebuilds a
missing record, so an already-orphaned agent is adopted instead of duplicated, and spawning refuses
to start a second agent for a stage that has a live one.

**Scan direction matters.** Evidence recovery runs from the STAGE side, deriving each candidate
tracking key from the stage id, not from the pid-file side. A pid filename is
`<tracking_key>-<session_id>` and neither half is delimited, so parsing one backwards lets stage
`a`'s prefix match stage `a-b`'s file and invent a session id that never existed.

## Re-Queueing a Stage Without a CONFIRMED Kill Double-Spawns Into a Live Worktree (2026-08-30)

`handle_budget_exceeded` removed the over-ceiling session from `active_sessions` and re-queued its
stage WITHOUT killing the process first — the next daemon poll then spawned a SECOND agent into a
worktree the first one was still writing to. The sibling handoff path, `on_needs_handoff`, had
always done kill → remove-signal → re-queue in the correct order; the fix was to route the budget
path through the same shared helper. That fix was still broken on first landing: the kill sat
inside `if let Some(session) = active_sessions.get(stage_id)`, while the re-queue ran
UNCONDITIONALLY — and `active_sessions` is in-memory only, never rebuilt on a daemon restart. After
a restart the map is empty, so a live over-ceiling session is re-queued with NO kill and the exact
same double-spawn recurs through a different door. **Rule: any code path that re-queues a stage
must gate the re-queue on CONFIRMED death of the prior session — `is_session_alive` plus the
on-disk session record — never on whether an in-memory map happens to have an entry for it.**

Two more traps sit directly downstream of the fix itself:

- **Survivorship after a kill must be decided by RE-PROBING liveness, not by the kill call's
  return value.** An errored kill may still have worked; a successful one may not have taken
  effect yet. `TmuxBackend::kill_session` always returns `Ok`, and `NativeBackend::kill_session`
  returns `Ok` even when it refuses to signal an identity it cannot verify — `Ok` is never
  evidence of a kill.
- **`SIGTERM` returns immediately; the process does not die immediately.** A liveness probe run
  right after `kill_session` reports even a correctly-killed agent ALIVE for a moment, which would
  call every killed agent a "survivor" and wedge every handed-off stage. `confirm_session_gone`
  polls for up to a short timeout (2s) instead of probing once.
- **Declare `ContextExhausted` only AFTER confirmed death, and require that write to succeed.**
  `live_sessions_for_stage` filters terminal records out, so marking first can hide a survivor;
  failing to persist after death leaves a stale `Running` record that the next monitor poll charges
  as a crash. Remove the in-memory handle and re-queue only after both proofs succeed.
- **A daemon restart requires the broader persisted-record scan.** `active_sessions` is empty and
  the old process may already be dead, so liveness-filtered discovery misses the exact stale
  `Running` record takedown must retire. At this boundary, an unreadable record or liveness-probe
  error is uncertainty, not absence: leave the stage in `NeedsHandoff` instead of admitting a
  second writer. `NeedsHandoff` detection is level-triggered on every monitor poll until takedown
  succeeds and re-queues the stage, so repairing transient uncertainty does not require a daemon
  restart to rearm the event.
- **Every asynchronous handoff event must still name `stage.session`.** A delayed
  `SessionNeedsHandoff` or `BudgetExceeded` from a predecessor must be ignored after a successor is
  assigned; otherwise a valid old event kills the healthy new agent. Verify that identity in the
  locked transition, again immediately before discovering/killing processes, and again in the
  locked re-queue update: a concurrent manual retry can replace the assignment between any two of
  those steps. Identity is necessary but not sufficient: destructive handoff may begin only from
  `Executing` or `NeedsHandoff`, and the final kill/re-queue checks require `NeedsHandoff`, so a
  same-session event cannot override a concurrent `Blocked` or terminal transition.
- **Judge the freshest resident-token fact, not the largest historical one.** Native compaction can
  lower resident context. Poll matching heartbeat files first, overlay their token count onto the
  in-memory session snapshot, and only then evaluate Red/backstop transitions; preserve the public
  event order separately. Also require the persisted `Running` record to be the exact active stage
  assignment before context-judging it, because predecessor records outlive their ownership.
- **Keep the retry's cause attached to the current session.** A failed budget takedown leaves the
  matching stage in `NeedsHandoff`; on the next poll the monitor must re-emit `BudgetExceeded`, not
  replace it with generic `SessionNeedsHandoff`. Detect sessions before stages so the live budget
  latch can suppress that generic event, while retaining stage-before-session _emission_ ordering.
  Drop latches for missing or non-`Running` records so a predecessor cannot suppress a later normal
  handoff. Retry while the exact over-budget assignment is still `Executing` too: the first handler
  can fail before it persists `NeedsHandoff`, and an edge-triggered in-memory latch would otherwise
  suppress every later attempt. The handler's `NeedsHandoff` mark is idempotent for the same
  verified session; retries must not add attempt time again.
- **Handoff idempotence needs a durable cause, not just session identity.** A Red-band snapshot for
  `(stage_id, session_id)` may predate substantial work done before the 125% backstop. V2 handoffs
  therefore persist an optional typed `origin` (`red_band` or `budget_exceeded`), and lookup scans
  all numbered artifacts for the exact `(stage, session, origin)` tuple. Legacy/manual/malformed
  files cannot suppress the first budget snapshot, while later retries and daemon restarts reuse
  the tagged artifact. A cold-start Red observation reuses that advisory only when its resident-token
  snapshot is identical; newer context or a known Green/Yellow-to-Red re-entry writes a fresh one.
  Two genuine crossings are not retries of one event. Directory or file-read errors remain
  uncertainty and fail the budget action closed.
- **Continuation must validate authorship, and allocation must be serialized.** The highest numbered
  filename may be malformed, manual, or written by another session. Select the newest valid V2
  handoff for the exact outgoing `(stage, session)` pair; only a stage with no predecessor may use
  the legacy latest-file fallback. Allocate the sequence number and crash-atomically write the file
  under one handoff-directory lock, or concurrent daemon/CLI producers can choose and overwrite the
  same name.
- **The stage assignment is the final discovery witness.** Combining `active_sessions` with
  persisted `Running`/`Spawning` records is not enough: after a restart, an assigned session with a
  missing record could still be an untracked writer. Takedown must load the stage's exact assigned
  session as a final check. Even an exact terminal record must be probed: `Completed` is persisted
  before an agent necessarily finishes its merge/teardown path, so workflow state is not process
  death evidence. A missing or mismatched record leaves the stage in `NeedsHandoff` rather than
  treating an empty scan as permission to re-queue.
- **Missing process identity is uncertainty, including after teardown.** Record whether PID
  evidence was already absent before invoking the backend; removing files during teardown cannot
  convert that pre-existing uncertainty into confirmed death. A tmux `kill-server` failure must
  propagate and retain the socket and PID evidence, because unlinking the only control handle can
  strand a live writer while making it look absent. Verified PID/start-time evidence must likewise
  survive SIGTERM until confirmation observes definitive death: signal delivery is asynchronous,
  and deleting the entry immediately makes a slow-exiting process look gone.
- **Heartbeat persistence is an exact, locked read-modify-write.** A complete event session id must
  address exactly `<session-id>.md`, match the stage, and still be `Running` under the same lock that
  applies the heartbeat. Prefix lookup or an unlocked read followed by whole-record save can mutate
  the wrong session or overwrite a concurrent terminal transition. Compare the complete heartbeat,
  not only its whole-second timestamp: context can change twice in one second, especially across
  native compaction.
- **Manual resume queues work; it does not spawn it.** `loom resume` preserves the predecessor id
  while moving `Blocked`/`NeedsHandoff` to `Queued`, then asks the operator to run `loom run`. Only
  the orchestrator may perform predecessor liveness verification, write-ahead session assignment,
  and successor spawn. Direct continuation auto-spawn is rejected so a CLI path cannot orphan or
  double-spawn an agent.
- **Advisory Red handoff readiness is separate from Red-band observation.** Record readiness only
  after the artifact was successfully found or written. If transient I/O makes that operation fail,
  an unchanged Red reading retries on the next poll; remembering only the band transition would
  permanently disarm the handoff until the session left Red.

**How to test a liveness-gated branch without risking the test runner:** write a PID file with NO
start-time line. It verifies as `Unverifiable`, which counts as ALIVE for probing purposes, while
the verified-kill path REFUSES to signal an unverifiable identity — so a test can point this at its
own PID with zero risk of the test killing itself. `Some(u64::MAX)` as the start time gives the
deterministic dead case for the opposite branch.

## The Judge-Close Path Persisted Its Record on an Unconfirmed Kill (2026-09-05)

**What happened:** `a_judge_for_another_dispute_is_left_alone` failed on nearly every CI push while
passing locally. `close_adjudication_session` (`judge_close.rs`) sent `SIGTERM` through
`kill_session`, wrote the session record, and returned; the tests probed `is_process_alive` on the
very next line, so any delay between the signal and the victim's exit fails the assert. On this
32-core developer machine the window never opened, even with the test binary and six spinners
pinned to two CPUs with `taskset -c 0,1` for 100 runs, while a GitHub-hosted 4-vCPU runner is an
oversubscribed VM where a woken process can wait on the hypervisor long enough for the probe to run
first.

**Why:** the takedown path in `stage_takedown.rs` already confirmed kills through
`confirm_session_gone` (see the "Re-Queueing a Stage Without a CONFIRMED Kill" entry above), but the
judge-close path was written separately and probed nothing before persisting. A second kill path was
added without the first path's confirmation step.

**Prevention:** every path that signals a process and then writes a terminal status must confirm
death through the shared bounded poll before the write. A test that kills a real process and asserts
on its death belongs in `scripts/flake-check.sh`'s default filters. A kill-then-probe race can be
impossible to reproduce on a developer machine even pinned to two CPUs under load, so do not wait
for a local reproduction before applying the confirmed-kill rule to a path that signals and then
writes state.

**Fix:** `close_adjudication_session` calls `confirm_session_gone` (now `pub(crate)`) before
persisting the session status; the judge test modules run under the flake-check job.

## Orchestrator Loop: Unbounded Subprocess Freezes All Scheduling

**Mistake:** Session teardown (`handle_stage_completed` → `kill_session` → window close) shelled out with `Command::output()` and no timeout, on the orchestrator's single poll thread. On macOS that call is `osascript`, which blocks indefinitely on a TCC Automation prompt, a terminal modal, or an unresponsive terminal app. One user's daemon froze there for 10 hours: the dependent stage sat `Queued`, no `.work/` file was written, and nothing appeared in the log.
**Why it hid:** the daemon's socket thread is separate, so `loom status` kept reporting "● daemon running". Restarting fixed it, which reads as a transient glitch rather than a hang.
**Prevention:** every external command issued from the poll loop goes through `process::run_bounded`. Teardown steps are best-effort — never `?` between removing the session and `try_auto_merge`, because `StageCompleted` is edge-triggered and never fires twice for the same stage. Check `.work/orchestrator.tick`: a stale tick with a live daemon means the loop is stuck, and the second line names the phase.
**Note:** Linux is less exposed only by accident — its `wmctrl`/`xdotool` paths are `which`-guarded and no-op when the tools are absent. The structure was the bug, not the platform.

## Diagnostics: Restart Destroys the Evidence

**Mistake:** `.work/orchestrator.log` was opened with `File::create`, truncating it on every `loom run`. Restarting the daemon is the standard response to a stuck orchestrator, so the log of the run that got stuck was destroyed at exactly the moment it was needed.
**Fix:** rotate to `orchestrator.log.prev` on start. When diagnosing a stall after a restart, read the `.prev` file — the live log only covers the recovery run.

## detection.rs: Session Exit for Merge States

**Mistake:** `detection.rs` only recognized `Completed` as normal session exit. Merge conflict sessions treated as crashes.
**Fix:** Added `MergeConflict | MergeBlocked` to the matches! pattern. When adding new terminal stage statuses, always update detection.rs.

## macOS GUI App CLI Not on PATH — Detection-Spawn Mismatch (2026-04-27)

**What happened:** `TerminalEmulator::Ghostty` detection succeeded on macOS via a `/Applications/Ghostty.app` path-existence fallback (detection.rs:190-191), but spawn called `Command::new("ghostty")` and failed with "Failed to spawn terminal 'ghostty'. Is it installed?" The Ghostty CLI binary lives inside the bundle at `/Applications/Ghostty.app/Contents/MacOS/ghostty` and is not added to PATH (ghostty-org/ghostty#2483). Detection picked the terminal; spawn couldn't launch it.

**Misleading signal:** `which::which("ghostty")` failing was _handled_ by an explicit `.app` existence check that succeeded. The fallback proved the GUI app was installed, not that its CLI was reachable from a child `Command`. Two-binary detection (`which` OR `.app exists`) silently expanded the set of "detected" terminals beyond the set of "spawnable via PATH" terminals.

**Why it broke:** Detection logic and spawn logic relied on different existence proofs. Detection accepted "the .app exists" as sufficient; spawn assumed the binary was on PATH. The asymmetry produced a guaranteed runtime failure for any macOS user without a manual PATH shim.

**Prevention:**

- For any `TerminalEmulator` variant whose detection has a path-based fallback (anything beyond `which::which(binary())` succeeding), the corresponding `build_command()` arm MUST use a launch path that does not depend on PATH — typically `open -na <AppName> --args ...` (see patterns.md "macOS GUI App Launch Pattern") or AppleScript via `osascript`. Treat any macOS `.app`-bundled tool as PATH-unreachable by default.
- When adding a new terminal emulator: check that detection and spawn agree about _how_ the binary is reachable. If detection falls back to `.app` existence, spawn must NOT call `Command::new(binary())` directly on macOS.

**Fix:** `Self::Ghostty` arm in `emulator.rs:build_command()` is now cfg-gated; macOS reassigns `command = Command::new("open")` and uses `open -na Ghostty --args --working-directory=... --title=... -e bash -c CMD`. Linux behavior unchanged. `binary()` still returns `"ghostty"` (correct for Linux PATH lookup and for any macOS user with a manual shim). Tests `test_ghostty_build_command_macos` and `test_ghostty_build_command_linux` are cfg-gated so each runs on its target platform.

## Sessions Died in One Second on WSL: Sandbox Required but Unavailable (2026-09-03)

**What happened:** A first run on WSL had every Claude session exit within a second of spawn, three times, then block. The crash reports said only "No log output captured", the retry logic blamed `--remote-control` and dropped it, and the stage burned its whole retry budget on a deterministic startup refusal.
**Why:** Generated settings set `sandbox.failIfUnavailable: true` whenever the plan sandbox is enabled, and Claude Code refuses to start when its Linux sandbox cannot initialize (missing `bwrap`/`socat`, or WSL1). Nothing in `loom run` checked those prerequisites; the tmux backend captured no pane output; `hooks/session-end.sh` discarded the SessionEnd `reason`; and the crash handler retried an exit that identical arguments could never fix.
**Prevention:** `loom run` now refuses to start on Linux/WSL when `bwrap` or `socat` is missing or the kernel is WSL1 (`commands/run/sandbox_preflight.rs`); the wrapper tees Claude's stderr to `.loom/work/logs/<session>.stderr.log` and the crash report embeds its tail; a crash inside the fast-fail window with no remote-control fallback left to apply is a `StartupRefusal` and is never retried.
**Fix:** On the host, `sudo apt install bubblewrap socat`, confirm `uname -r` contains `WSL2`, then `loom stage retry <stage-id>`. To see Claude's own error for any instant exit, run the stage's wrapper by hand: `bash .loom/work/wrappers/<pid-key>-wrapper.sh`.
