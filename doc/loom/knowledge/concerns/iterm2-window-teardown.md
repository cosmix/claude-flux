# Iterm2 Window Teardown

> iTerm2 spawn never names its window, so teardown cannot find it to close

## iTerm2 Windows Survive Stage Completion — Spawn Never Names the Window (GitHub #7, 2026-08-29)

GitHub issue #7 (open since 2026-02-04) reports that a completed stage leaves the iTerm2 window
open on macOS while Linux DEs close theirs. Commit `2a68aee2` (2026-02-05) added the per-terminal
AppleScript close functions, but the tree still reproduces the report:

**Cause.** Teardown closes by window title: `orchestrator/core/completion_handler.rs:49-67` →
`NativeBackend::kill_session` (`terminal/native/mod.rs:335-347`) → `close_iterm2_window`
(`terminal/native/window_ops.rs:165-183`), which runs `every window whose name is
"loom-<stage-id>"`. The iTerm2 spawn arm (`terminal/emulator.rs:219-238`) does `create window with
default profile` + `write text` and never uses its `title` parameter — `git log -S 'set name of'` is
empty, so no version ever named the session. The query matches nothing, the close returns `false`,
and `kill_session` falls through to `pid_only_terminate` (`native/mod.rs:352-365`). That kills
`claude`, but `write text` typed the wrapper into an interactive shell, so the shell returns to its
prompt and the window stays. Linux emulators are launched with `-e <wrapper>`
(`emulator.rs:180-200`): the window closes on its own when the exec'd `claude` dies, and
`wmctrl -F -c` also matches the `--title` given at launch. `iterm2_window_exists`
(`window_ops.rs:378`) has the same gap, so the `is_session_alive` title fallback is inert on iTerm2
too.

**Why it hid.** `test_iterm2_build_command` (`emulator.rs:417-435`) never asserts that the title
appears in the generated script.

**A second, independent defect in the same path (verified 2026-08-29).** Spawn addresses
`tell application "iTerm"` (`emulator.rs:229`); both teardown paths address
`tell application "iTerm2"` (`window_ops.rs:167`, `:380`). iTerm2's scriptable application name is
`iTerm`, so the close and the liveness check target a name that does not resolve. Naming the window
is therefore necessary but not sufficient — a fix that only adds `set name to` would leave teardown
failing for this second reason, with the same silent `false` return. Found with
`rg -n 'application "iTerm' loom/src/orchestrator/terminal/`, which is also the check that proves
it fixed.

**Terminal.app (the issue's TBD).** Unverified — needs a macOS host. Its arm does `set custom title
of front window` (`emulator.rs:214`), so the close can match, with two risks: a Terminal.app
window's AppleScript `name` is the full displayed title (custom title plus process and size
components under default profile settings), and nothing pins the title against Claude Code's own
OSC title updates — the wrapper (`native/wrapper.rs`, `exec env -i …`) emits no title escape and
passes no `CLAUDE_CODE_DISABLE_TERMINAL_TITLE`.

**Fix shape.** In the iTerm2 arm add `set name to "{escaped_title}"` inside `tell current session
of current window` before `write text`, and assert it in `test_iterm2_build_command`. Consider
exporting `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1` through the wrapper's env allowlist (both copies —
see "Two Diverging Copies of the Stage Environment Allowlist" in concerns.md) so neither macOS
terminal's window name drifts before teardown. Verify on macOS with both terminals before closing #7.
