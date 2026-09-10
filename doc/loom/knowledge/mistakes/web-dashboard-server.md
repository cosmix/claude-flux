# Web Dashboard Server (loom status --web)

> Concurrency, security and testing lessons from building the hand-rolled HTTP/WebSocket
> server under `loom/src/commands/status/web/` and its React frontend. See
> [architecture/web-dashboard.md](../architecture/web-dashboard.md) for the shape of the
> system these fixes apply to.

## clippy::never_loop Caught a Real Drain Bug No Test Could (2026-09-04)

**What happened:** `loom/src/commands/status/web/ws.rs`'s `send_pending()` was meant to
drain the whole frame queue but its match arms returned on the FIRST iteration
(`Ok(frame) if send fails => return false`, `Ok(_) | Err(Empty) => return true`),
conflating "sent one frame" with "queue empty". A client that fell behind drained its
backlog at ~1 frame per 250ms read-timeout tick instead of catching up at once.

**Why:** every test asserted that A frame arrives, never that N queued frames all arrive.

**Prevention:** `clippy::never_loop` on a function named drain/flush/pump/consume_all is a
behaviour bug, never a lint to `#[allow]`. Give each outcome (`Ok(frame)` continues,
`Err(Empty)` returns true, `Err(Disconnected)` returns false) its own arm.

## Two Concurrency Defects a Test Suite Cannot Reach (2026-09-04)

**What happened:** (1) `subscribe()` built an unbounded `mpsc::channel()`, and `publish()`
only drops a subscriber when `send()` fails, which never happens on an unbounded channel
while the receiver lives — and the receiver lives forever if a peer stops reading (no
`set_write_timeout`), so a sleeping tab accumulates every snapshot for the process
lifetime. (2) `subscribe()` read `latest()` and sent frame N, THEN took the subscribers
lock and registered — a `publish()` landing in that window delivers frame N+1 to a list
that doesn't yet include the new client; downstream dedup on `last_body` means if the tree
then stops changing, no further frame is ever sent and the page is stuck stale.

**Prevention:** bound the channel (`sync_channel(N)`, `try_send`, drop on `Full`) and add a
write timeout; take the subscribers lock BEFORE reading `latest` so send-then-register is
atomic. When a "send current state then register" pair spans two locks, the gap is a lost
update, and dedup downstream turns a one-frame glitch into a permanent one.

## A Field Named `since` Meant Two Different Things Across a Worker Boundary (2026-09-04)

**What happened:** `web/src/api/ws.ts` stamped `connectionAtom.since` only on a PHASE change
(never moving while a connection stayed live); `web/src/components/header-lines.tsx`
rendered it as "last frame N ago", so a healthy socket delivering a frame every second showed
an age that climbed forever. Both sides type-checked; parallel workers never saw each other's
code.

**Prevention:** a field crossing a worker boundary needs its units AND meaning pinned in the
brief ("since = ms epoch of the last PHASE CHANGE, not of the last frame") — a number named
`since` passes typecheck against any interpretation. Fix: derive frame age from the
snapshot's own `generated_at` when live, `since` only for phase age.

## Storing `globalThis.fetch`/`setTimeout` as Instance Fields Broke in a Real Browser (2026-09-04)

**What happened:** `web/src/api/ws.ts` stored `globalThis.fetch` and `globalThis.setTimeout`
as instance fields and called them as `this.request(...)`/`this.schedule(...)`. Chrome
throws `Illegal invocation` for both, because they require the original receiver — but
jsdom's mocks don't enforce that, so the test suite passed while every real fetch and
reconnect failed.

**Prevention:** bind (`globalThis.fetch.bind(globalThis)`) or call through a closure. A
DOM-native method stored as a plain reference and invoked as `this.method()` is a receiver
mismatch jsdom will not catch — verify browser API wrappers against a real browser, not just
jsdom.

## A Second Copy of a Status Table, Pinned to Literals, Diverged Silently (fixed in-stage; 2026-09-04)

**What happened:** the dashboard kept two status-metadata tables — the live one in
`web/src/lib/states.ts` (reads the Rust-pinned fixture) and a dead `STATE_META`/`LEGEND`
table in `web/src/lib/format.ts` whose only consumer test (`web/src/lib/format.test.ts`)
asserted against hardcoded literals, not the shared source.

**Prevention:** a test that pins a copy against literals rather than the shared source makes
divergence look verified — worse than no test. Delete the dead copy rather than leaving a
second source of truth nobody reads.

## Two Negated Skip Predicates Over the Same Probe Made Two Suites Mutually Exclusive (2026-09-04)

**What happened:** `loom/src/commands/status/web/tests/socket.rs` had one test set skip
unless `assets::WEB_ASSETS.is_empty()` and another skip unless it was non-empty. Deleting
`web/dist` silently dropped all real-page coverage while the suite stayed green.

**Prevention:** if two skip predicates over the same probe are negations of each other, no
single configuration runs both. Assert the build fact once and unit-test the degraded branch
on a pure function instead.

## Bare Package Specifier Inside a CSS `url()` Needed Verification, Not Trust (2026-09-04)

**What happened:** a plan correction named `@fontsource-variable/inter/latin.css`, which the
installed package does not ship (only `index.css`, `wght.css`, etc). Substituted an explicit
`@font-face` pointing at the one `.woff2` file that exists. Open risk that acceptance did not
directly test: Vite resolving a bare specifier inside a CSS `url()` is not guaranteed the way
a `./`-prefixed path is — verify at `bun run build`, not by reading the CSS.

## A CI Smoke Script Depended on Ambient `.loom/work` (2026-09-04)

**What happened:** `scripts/smoke-web-dashboard.sh` launched `loom status --web 0` in its
inherited CWD; `status::web::execute` (`loom/src/commands/status/web/mod.rs:41-42`) bails
with "does not exist. Run loom init first" when `./.loom/work` is absent. Every
local/worktree run has that symlink, so the script passed everywhere except a clean CI
checkout, where `.loom/work` is gitignored and never exists.

**Prevention:** any script a CI job runs must be tested from a genuinely fresh checkout
(`mkdir $TMPDIR/x && cd $TMPDIR/x && git init -q && <script>`) — that is the only thing that
distinguishes "works" from "works because my working copy has state CI will not have."
Fixed by having the script build its own scratch workspace (`mktemp -d`, create
`.loom/work` there, run from it).

## React Flow Dropped Every Edge Because Rebuilt Nodes Carried No `measured` (2026-09-05)

**What happened:** the graph view rebuilds React Flow's `nodes` array from the snapshot on
every frame (`buildNodes`, `web/src/components/graph/stage-graph.tsx`). `adoptUserNodes`
(`@xyflow/system` 0.0.82) rebuilds the internal node for any object it has not seen by
reference, and its `parseHandles` keeps the measured handle bounds only when the incoming
node carries `measured`. Ours carried `width`/`height` but no `measured`, so every frame
wiped every node's handle bounds; `getEdgePosition` returns null without them, so all edges
dropped until a ResizeObserver round-trip restored them. When a frame landed inside that
round-trip, `useNodeObserver`'s `isInitialized` read false on two consecutive renders, its
effect (deps `[isInitialized, node.hidden]`) did not re-run, the observed element's size had
not changed, and nothing re-measured: the edges stayed gone until a card resized. Reported as
"occasionally, after the dashboard has been open a while, the edges disappear".

**Why:** rebuilding a prop every frame looks free for a controlled component, but React Flow
keeps per-node measurement state keyed by object identity, and hands it back only to an
object that declares it was measured.

**Prevention:** when a library keeps measured state for objects you hand it, either keep the
object identity stable across frames or declare the measurement on the object you pass.
`web/src/lib/graph.ts` already lays each card out at exactly the size it renders at, so
`measured: { width, height }` costs nothing and makes the graph independent of the observer.

**Fix:** `measured` on both node kinds in `buildNodes`. The regression test
(`web/src/components/graph/stage-graph.test.ts`) drives `adoptUserNodes` twice with freshly
built arrays and asserts the handle bounds survive — a jsdom render cannot catch this, since
the inert `ResizeObserver` stub never measures at all and a synchronous stub always recovers
before the assertion, so the test pins the library contract instead of the DOM.

## A Hover Focus Outlived the Chip That Set It (2026-09-05)

**What happened:** `StateKey` renders one chip per state present in the plan. A chip whose
last stage leaves that state unmounts under the pointer and never fires its `onMouseLeave`,
so `Canvas`'s `hovered` focus stayed on a state no stage was in. `tracedIds` returned an
empty set, which dims everything: nodes to 0.3 opacity, edges to 0.18 (`web/src/graph.css`).
That also reads as the edges disappearing.

**Why:** the focus was validated for one of the two shapes it can take — a pinned stage id
was checked against the snapshot, a status was not.

**Prevention:** state naming something in the data (an id, a status) needs re-validating
against each new snapshot, for every shape it can take, not just the shape whose staleness
was noticed first.

**Fix:** the status branch of `tracedIds` returns null when no stage has that status, the
same as the stage branch already did.

## A push-fed page read liveness out of the payload, not the transport

**What happened:** The dashboard header rendered `daemonLine(snapshot.daemon, snapshot.tick_age_secs)` straight off `snapshotAtom`. When the feed stopped — the serving process gone, not just the daemon — React kept the last frame, so the page claimed `daemon running · tick 3s ago` for as long as the tab stayed open, and the badge sat on `reconnecting` forever because `ConnectionState.since` was re-stamped on every retry.

**Why:** Every liveness field on the wire (`daemon`, `tick_age_secs`, `generated_at`) is a statement about the moment the frame was generated. A frame is evidence of the past; only the socket is evidence of the present. Nothing in the payload can report its own staleness.

**Prevention:** In a push-fed UI, derive "is this current?" from the transport state (`connectionAtom.phase`), never from the frame's contents. Frame AGE is also the wrong signal here — `loom/src/commands/status/web/broadcast.rs` suppresses unchanged frames, so a quiet live feed looks old while being perfectly current. Model an outage as one event with one start time held across retries, or every reconnect resets the age the operator is reading.

**Fix:** `daemonLine` took a third `staleSecs` argument that wins over every daemon state (`web/src/lib/format.ts`); `DaemonLine` computes it from the connection phase — stale for `reconnecting`/`offline`/`error`, not for `connecting` (the initial `/api/status` fetch lands during that phase). `ws.ts` preserves the outage `since` across retries and flips to a new `offline` phase after the fourth consecutive failure. `web/src/routes/shell.test.tsx` pins the wiring at the rendered-page level, which is the layer the unit tests could not reach: `daemonLine` and the socket state machine were both individually correct while the page was wrong.

## PTY Master Fd Leaked Into the Child, Defeating Hangup Detection (2026-09-09)

**What happened:** confirmed by measurement, not inspection — `cargo test --lib
...tests_pty::pty_child_shutdown_reaps_the_child -- --exact` finished in exactly `2.00s` on
two consecutive runs, matching `PtyChild::shutdown`'s fallback deadline to the millisecond,
never the fast path. `nix::pty::openpty` wraps libc `openpty(3)`, which has no flags argument
and opens the master **without** `O_CLOEXEC`. The `fcntl` at `pty.rs:25-27` set `O_NONBLOCK`
(a file-status flag) but never touched the descriptor flags, so the master survived into the
tmux child across `Command::spawn`'s fork+exec. With the child holding its own copy,
`shutdown`'s `drop(master)` (`pty.rs:107` at the time) was never the last reference, the slave
never saw a hangup, the child never got `SIGHUP`, and every terminal close burned the full 2s
deadline before `SIGKILL`. Ironically the surrounding code already carried a comment about
dropping the three slave clones so the master would see the child's hangup — the master was
the fd nobody CLOEXEC'd, defeating exactly that mechanism.

**Why:** `openpty` gives `O_CLOEXEC` on neither end, and a timing assertion whose own budget
(`elapsed < 3s`) exceeds the fallback deadline cannot distinguish the fast path from the slow
one — it passed either way.

**Prevention:** after any `openpty`/`posix_openpt`-style call, `fcntl(F_SETFD(FD_CLOEXEC))` the
master explicitly — never assume it. When timing a "did the fast path fire" behaviour, assert
strictly *below* the fallback deadline, not merely under some larger ceiling.

## A Backpressure Gate Made a Half-Closed Browser Invisible, and Two Fixes for It Failed First (2026-09-09)

**What happened:** the terminal bridge's inbound gate narrows the polled TCP event mask to
`empty()` once the pending-input queue is full, so it stops asking whether the socket is
readable. The brief that reached the implementer claimed "POLLHUP/POLLERR/POLLNVAL are
reported whether or not you ask for them, so masking POLLIN off costs nothing in disconnect
detection" — true for those three flags, wrong as a conclusion. A TCP peer's `close()` sends a
FIN, which sets `POLLIN` (read returns EOF) and, on Linux, `POLLRDHUP` — **not** `POLLHUP`,
which `tcp_poll` only sets after an RST or `sk_shutdown == SHUTDOWN_MASK`. So a gated,
half-closed browser tab (control terminal, a paste over 64 KiB, a slow-draining tmux child,
or simply closing the tab while the PTY is quiet) was invisible for the whole gated window,
holding one of 8 terminal slots, a PTY and a tmux child indefinitely. The claim had been
copied verbatim from a reviewer into an implementer's brief and propagated unchecked through
two agents.

The first fix attempted — `recv(MSG_PEEK|MSG_DONTWAIT)` as a portable EOF probe, overriding a
reviewer's POLLRDHUP suggestion on cross-platform grounds — also failed, for a reason obvious
only in hindsight: `recv` returns 0 only once the receive queue is **empty** and the FIN has
been consumed in sequence, but the gate exists precisely because the queue was left unread.
While gated, the queue is almost always non-empty, so the peek returns `>0` and the probe is
silent in exactly the state it was built to detect.

**Why:** "reported whether or not you asked for it" is a true statement about `POLLHUP`
specifically, never a general statement about disconnect detection — the ordinary TCP close
path is `POLLIN`/`POLLRDHUP`. And any EOF-by-read probe is defeated by design whenever the
surrounding code deliberately leaves the receive queue unread.

**Fix (the one that shipped):** a portable stall deadline instead of any readiness- or
read-based detector — `gate_stalled` tracks how long `pending` has sat at its cap and gives up
after `GATE_STALL_TIMEOUT` (30s), closing with code 4008. It needs no `cfg`, works identically
on every platform, and also catches the case no readiness flag or FIN can: a peer that is
alive but has wedged the PTY, so nothing ever signals its side.

**Prevention:** before masking a readiness flag off any socket, name the exact syscall-level
event that signals peer close on each target platform — do not generalise from "this flag
fires unconditionally." Before proposing any mechanism that *reads* a socket to detect EOF,
ask what is already sitting in the queue in the state being detected; a backpressure gate
guarantees that queue is non-empty, which defeats every read-based EOF probe. A claim
inherited from a reviewer and pasted into a brief carries no more authority than one invented
from scratch — verify it before it reaches an implementer.

## Front-Truncating a Bounded Input Queue Executes an Attacker-Controlled Suffix (2026-09-09)

**What happened:** an early version of the terminal bridge's pending-input queue dropped from
the **front** on overflow — the dangerous direction for a terminal. A typed command is
submitted by its *last* byte (the newline), so front-truncation keeps that trailing newline
and discards the beginning: what reaches the agent is a suffix of what was typed, already
terminated, so the shell executes it. `echo "summarise <70KB of pasted log>"` front-truncated
becomes `...tail of the log"` + newline — a complete, executable line. Root cause was the
absence of backpressure: the socket was drained on any `POLLIN` with no reference to the
queue's length, making it a lossy ring rather than a staging buffer for one partial write.

**Why:** the queue was sized to bound memory, but nothing stopped the socket read that fed it
once full, so "bounded" meant "silently lossy" rather than "backpressured."

**Fix:** gate the inbound read on `pending.len() < MAX_PENDING_INPUT` and let TCP's own
receive window stall the browser instead — nothing is lost, ordering is preserved, and
`pending` now peaks near the WebSocket read-buffer size, so no drop policy is ever reached.
Coupling to watch: this is only safe while the outbound direction (`pump_pty`) drains the PTY
unconditionally — "stop reading input because the queue is full" plus "the queue is full
because the child is blocked writing" is the classic proxy deadlock.

**Prevention:** when a bounded queue sits between a network peer and a PTY, identify which
*end* of the stream carries the commit token (for line-oriented input, the trailing newline)
before choosing a drop policy — any policy that discards context while preserving the commit
token turns a truncated paste into an executed command. Prefer backpressure over any drop
policy when the upstream transport already has flow control (TCP) to push back with.

## An Integration-Verify Reviewer Cannot See the Plan, So It Confidently Reports Settled Decisions as Defects (2026-09-09)

**What happened:** two of three independent code reviewers flagged the terminal bridge's
`Resize` handling in view mode ("should not resize the agent's real PTY") as a bug. It was a
documented, deliberate plan decision — `-f read-only` blocks input only, and `ignore-size` was
explicitly considered and rejected because it would force the agent's real column count into
the browser's well. A third reviewer, working from the same tree-only vantage point, correctly
found a real defect (the front-truncation issue above) that looked structurally identical in
its report: "this design choice looks wrong, here is why." Nothing in a diff-and-code review
distinguishes "the reviewer hasn't seen the rationale" from "the rationale doesn't exist."

**Why:** an integration-verify reviewer reasons from the tree alone; it has no channel to the
plan document that recorded *why* a behaviour is the way it is.

**Prevention:** before briefing a fix for any review finding that would change a deliberate
behaviour, grep the plan's decisions table for the same words. When two reviewers disagree
about whether something is a defect, or a finding contradicts an earlier settled decision, halt
before briefing an implementer — do not let a "fix" get written (or, worse, a test that PINS
the wrong behaviour as correct) before the contradiction is resolved.
