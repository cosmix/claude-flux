---
---
# Status Broadcast Hardening

> Frame-overflow eviction and read-timeout desync from a widened status broadcast.

## Status Broadcast Hardening: Frame-Overflow Eviction and Read-Timeout Desync (2026-09-04)

Two independent defects surfaced once the daemon started pushing the full `StatusData` (not the
narrower old `StageInfo`) over the status socket, both found by an opus-tier security review after
a sonnet-tier pass reported "everything else checked clean" on the same diff.

**Frame-overflow subscriber eviction.** `loom/src/daemon/wire.rs:15` caps a response at
`MAX_RESPONSE_BYTES = 2 MiB`. `write_json_frame` bails BEFORE writing anything when a response would
exceed that (`loom/src/daemon/wire.rs:169-173`), and the broadcaster treated every write error identically to a dead
peer: `subs.retain_mut(|stream| write_message(stream, response).is_ok())`
(`daemon/server/broadcast.rs:190,195`). So a single oversized `StatusData` — which needs a large
number of stages contributing evidence, not the "~256 KB single string" first estimated (see
Correction below) — evicts EVERY subscriber on that tick, the TUI reads EOF and prints "Daemon
exited" (`commands/status/ui/tui/app.rs:190-198`), and reconnecting is evicted again on the next broadcast. The fix
(`broadcast_retaining_live`/`response_fits_frame`) sends `Response::Error` instead of silently
skipping the tick, because the TUI's header liveness indicator reads `.loom/work/orchestrator.tick`
directly, not the broadcast — a skip would leave the header claiming a healthy daemon over frozen
rows. `Response::Error` already routes into `TuiApp.last_error`/the footer and is small enough to fit
by construction, so no recursion into the size check is possible.

*Correction to the original magnitude estimate:* the evidence field is not ~256 KB. `read_log_tail`
(`loom/src/orchestrator/spawner.rs:97-110`) reads the last `MAX_LOG_READ_BYTES=256*1024` by byte offset but keeps only the
last `max_lines` (`STARTUP_REFUSAL_TAIL_LINES=20`, `loom/src/orchestrator/core/crash_classification.rs:23,187`) and clamps the
joined result to `MAX_LOG_TAIL_BYTES=16*1024` (`loom/src/orchestrator/spawner.rs:28,125-134`). A startup-refusal stage
contributes at most ~16 KB across at most 20 lines, so exceeding the 2 MiB frame needs on the order
of 128 such stages, not eight. The eviction defect stands regardless of the exact number: whenever
the payload does exceed the cap, every subscriber is dropped every tick with no recovery. Check a
reviewer's magnitude claim against the clamp that actually applies before repeating its severity.

**Verifying the regression guard actually guards.** The fix initially shipped with ZERO tests, and a
green 3950-test suite did not catch it: reverting `broadcast_retaining_live` to the old one-liner
left every test green. Proof that a guard guards is to break the guarded code on purpose and watch
red: copy the file aside, restore the pre-fix body, run the narrow test target, confirm failures,
restore the file byte-identically (`diff -q`), re-run to confirm green again. For every defect fixed
under adversarial review, ask "if someone reverted just this hunk, which test goes red?" — a module
with no `#[cfg(test)] mod tests` at all is the tell that nobody has ever had to test it.

**Read-timeout socket desync.** `TuiApp::connect_and_subscribe` sets a 50ms read timeout
(`commands/status/ui/tui/app.rs:116`); `read_message` calls `read_exact` twice (`loom/src/daemon/wire.rs:191-193` length, `206-208` body).
`read_exact`'s contract leaves it unspecified how many bytes were consumed on error, so a frame
straddling the 50ms window desyncs the stream. `is_socket_disconnected` returned `false` for
`WouldBlock`/`TimedOut` (`commands/status/ui/tui/daemon_client.rs:80-83`), so `receive_response` fell into `Err(_) => {}`
(`commands/status/ui/tui/app.rs:199`) and kept reading the desynced stream — the next length prefix is four misread JSON
bytes, which almost certainly exceeds `MAX_RESPONSE_BYTES` and is bailed on too, so the desync is
never flagged as a disconnect and the dashboard shows stale data forever with `last_error` still
`None`. The old narrow payload rarely spanned a 50ms window; the wide one does routinely. Fix:
`reconnect_after_read_error` (`commands/status/ui/tui/app.rs:222-242`) retries the connection on any non-timeout read error,
a plain for-loop with named `RECONNECT_ATTEMPTS(3)`/`RECONNECT_BACKOFF(80ms)` constants (no new
dependency, no thread — the file was already at 391/400 lines and had no room for anything heavier).
A follow-up attempt to make the desync detection race-free with `UnixStream::peek` was reverted: that
API is unstable (`E0658 unix_socket_peek`, rust-lang/rust#76923) and was the crate's only compile
error — a subagent's code arriving with passing-looking tests is not evidence it builds, because
those tests never ran if the crate itself does not compile. **Prevention (general):** `read_exact` on
a socket with a read timeout is only safe if a partial read tears the connection down; whenever a
payload widens, re-examine every read timeout it now has to fit inside, and compile a design before
defending it on its merits.

`reconnect_after_read_error` has no unit test — `TuiApp` owns the live socket and a real
`Terminal<CrosstermBackend<Stdout>>` directly with no trait seam, so it is covered only by the full
build. A future refactor extracting a testable connect-retry helper would take `work_path`/`backoff`
as parameters instead of `&self`.

Separately, the evidence cap that feeds these payloads was off by one: `MAX_EVIDENCE_LINES` was
originally 20, but a full startup-refusal crash produces 21 entries (`loom/src/orchestrator/core/crash_classification.rs:210-211`
builds `vec![reason]` then extends with up to 20 more), so the cap silently dropped the last line and
made `commands/status/render/attention.rs`'s "... N more lines" undercount by one. Raised to 32 (headroom, not just
the fix) with an explicit truncation marker pushed when it bites, so the count stays honest at any
future cap — check the real maximum a field can hold before choosing a round-number cap for it.
