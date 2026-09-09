//! What the bridge does when one side stops keeping up: a browser flooding a
//! child that will not read, and a child flooding a browser that will not
//! read. Both directions have a queue, a cap, and a way out; these cover the
//! caps holding, the data surviving them intact, and the bridge giving up
//! only when nothing is moving at all.
use std::net::TcpStream;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use tungstenite::{Message, WebSocket};

use super::super::bridge::GATE_STALL_TIMEOUT;
use super::super::protocol::{Mode, CLOSE_REFUSED, CLOSE_SERVER_STOPPING};
use super::{
    gating_flood, join_within, skip_bridge_test, start_bridge, start_bridge_with,
    wait_for_binary_progressing, wait_for_close, wait_for_close_frame,
};

/// How long the stalled half of the test below watches for a marker that must
/// never appear. A bridge that drained the master into its out-buffer
/// regardless of the stall would move 16 KiB per iteration and park twice per
/// iteration on the 250 ms write timeout - about 32 KiB a second, so this is
/// comfortably longer than such a bridge would need to swallow the child's
/// whole 256 KiB of output.
const STALL_WATCH: Duration = Duration::from_secs(8);

/// Send `payload` as 32 KiB frames, and fail if that took longer than
/// `budget`.
///
/// Every gated test here depends on the whole flood being in flight before
/// some deadline of the fixture's own - a child that starts draining, or the
/// bridge's own give-up timer. Miss it and the queue never fills, the gate
/// never shuts, and the test passes having exercised the ungated path
/// instead. The budget turns that silent loss of coverage into a failure.
fn flood_within(socket: &mut WebSocket<TcpStream>, payload: &[u8], budget: Duration) {
    let sending = Instant::now();
    for chunk in payload.chunks(32 * 1024) {
        socket
            .send(Message::binary(chunk.to_vec()))
            .expect("send flooded terminal input");
    }
    let flooding = sending.elapsed();
    assert!(
        flooding < budget,
        "the flood took {flooding:?}, past the {budget:?} this fixture stays gated for, \
         so what the assertions below saw was not the gated path"
    );
}

/// Fail the moment `marker` shows up, and keep watching for the whole window.
fn assert_never_appears(marker: &Path, window: Duration, why: &str) {
    let deadline = Instant::now() + window;
    while Instant::now() < deadline {
        assert!(!marker.exists(), "{why}");
        thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn bridge_backpressure_keeps_a_flood_of_input_whole() {
    if skip_bridge_test("bridge_backpressure_keeps_a_flood_of_input_whole") {
        return;
    }
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("typed");
    // 128 KiB, twice `MAX_PENDING_INPUT`, as 64-byte numbered lines: the
    // numbering makes a gap in the middle visible, and the newlines keep the
    // PTY's canonical line buffer flow-controlling instead of discarding.
    let payload: Vec<u8> = (0..2048_u32)
        .flat_map(|line| format!("{line:063}\n").into_bytes())
        .collect();
    // The sleep holds the PTY's input buffer shut for long enough that
    // `pending` fills and the socket read gate closes; `awk` then drains
    // every line up to the sentinel. `stty -echo` keeps those bytes from
    // coming back the other way while the client is busy writing.
    let script = format!(
        "stty -echo; printf 'ready\\n'; sleep 3; \
         awk '/^ZZEND$/{{exit}} {{print}}' > '{}'; printf 'drained\\n'",
        path.display()
    );
    let mut fixture = start_bridge(script, Mode::Control);
    assert!(wait_for_binary_progressing(
        &mut fixture.socket,
        b"ready",
        Duration::from_secs(10)
    ));
    let mut sent = payload.clone();
    sent.extend_from_slice(b"ZZEND\n");
    // Two seconds against the child's three second hold, leaving a second for
    // the lag between it printing `ready` and this thread seeing it.
    flood_within(&mut fixture.socket, &sent, Duration::from_secs(2));
    assert!(wait_for_binary_progressing(
        &mut fixture.socket,
        b"drained",
        Duration::from_secs(20)
    ));
    fixture.socket.close(None).expect("close bridge client");
    join_within(fixture.server, Duration::from_secs(5));
    let typed = std::fs::read(&path).expect("read what the child was typed");
    // Named, not dumped: a queue that drops its oldest bytes to stay under
    // the cap fails the length, one that reorders fails the contents.
    assert_eq!(typed.len(), payload.len(), "bytes typed by the child");
    assert!(typed == payload, "the bytes typed were not the bytes sent");
}

#[test]
fn bridge_survives_a_client_that_stops_reading() {
    if skip_bridge_test("bridge_survives_a_client_that_stops_reading") {
        return;
    }
    // Both ends' kernel buffers shrunk to 8 KiB, so 100 KiB of output cannot
    // sit in the network: with the client asleep well past the 250 ms write
    // timeout, `send` has to time out, which is the path under test. The
    // burst stays far below `MAX_WRITE_BUFFER`, so a bridge that retries the
    // write rather than reading it as a dead peer still delivers the marker.
    let mut fixture = start_bridge_with(
        "head -c 102400 /dev/zero | tr '\\0' 'x'; printf 'END\\n'",
        Mode::Control,
        Some(8 * 1024),
        GATE_STALL_TIMEOUT,
    );
    thread::sleep(Duration::from_millis(750));
    assert!(wait_for_binary_progressing(
        &mut fixture.socket,
        b"END",
        Duration::from_secs(20)
    ));
    fixture.socket.close(None).expect("close bridge client");
    join_within(fixture.server, Duration::from_secs(5));
}

#[test]
fn bridge_survives_a_flooded_child() {
    if skip_bridge_test("bridge_survives_a_flooded_child") {
        return;
    }
    let mut fixture = start_bridge("sleep 30", Mode::Control);
    for _ in 0..4 {
        fixture
            .socket
            .send(Message::binary(vec![b'x'; 50 * 1024]))
            .expect("send flooded terminal input");
    }
    fixture.running.store(false, Ordering::SeqCst);
    assert_eq!(
        wait_for_close(&mut fixture.socket, Duration::from_secs(3)),
        Some(CLOSE_SERVER_STOPPING)
    );
    join_within(fixture.server, Duration::from_secs(3));
}

#[test]
fn bridge_gives_up_on_a_child_that_never_drains() {
    if skip_bridge_test("bridge_gives_up_on_a_child_that_never_drains") {
        return;
    }
    // `sleep 30` never reads its stdin, so once the PTY's line buffer is full
    // nothing this client sends is ever consumed and the gate, once shut,
    // stays shut. One second stands in for `GATE_STALL_TIMEOUT`: short enough
    // to be a test, long enough that the flood below finishes well inside it.
    let mut fixture = start_bridge_with("sleep 30", Mode::Control, None, Duration::from_secs(1));
    // Half the deadline, so a flood that somehow outlasted it would be
    // reported as itself rather than as a close code that never arrived.
    flood_within(
        &mut fixture.socket,
        &gating_flood(),
        Duration::from_millis(500),
    );
    let (code, reason) = wait_for_close_frame(&mut fixture.socket, Duration::from_secs(10))
        .expect("a close frame saying why the bridge gave up");
    // The code alone would not settle it: `resolve`'s refusals close with 4008
    // too. The reason is what names the input queue as the thing that stalled.
    assert_eq!(code, CLOSE_REFUSED);
    assert_eq!(reason, "input stalled");
    join_within(fixture.server, Duration::from_secs(5));
}

/// 256 KiB of child output and then a marker, with both ends' kernel buffers
/// shrunk to 8 KiB. That is far past what those buffers and the PTY's own
/// output buffer together absorb, so a bridge that stops reading the master
/// while its writes are stalled leaves the child blocked in `write` with the
/// marker untouched - which is the assertion, and the only way to see that
/// tmux's backpressure reaches tmux rather than being absorbed here. It is
/// also well under `MAX_WRITE_BUFFER`, so a bridge that kept draining the
/// master into its out-buffer regardless would swallow all of it and let the
/// child run to completion.
#[test]
fn bridge_leaves_pty_output_upstream_while_the_client_is_stalled() {
    if skip_bridge_test("bridge_leaves_pty_output_upstream_while_the_client_is_stalled") {
        return;
    }
    let dir = TempDir::new().expect("temp dir");
    let finished = dir.path().join("finished");
    let script = format!(
        "head -c 262144 /dev/zero | tr '\\0' 'x'; touch '{}'; printf 'DONE\\n'",
        finished.display()
    );
    let mut stalled = start_bridge_with(
        script.clone(),
        Mode::Control,
        Some(8 * 1024),
        GATE_STALL_TIMEOUT,
    );
    // This client never reads, so nothing here may touch `stalled.socket`
    // until the watch is over.
    assert_never_appears(
        &finished,
        STALL_WATCH,
        "the child finished writing while the client was stalled, so the bridge drained the PTY \
         into its own out-buffer instead of leaving the output upstream",
    );
    stalled
        .socket
        .close(None)
        .expect("close the stalled client");
    join_within(stalled.server, Duration::from_secs(5));

    // Positive control, same script and same marker path: a client that does
    // read drains the bridge, the child runs to the end, and the marker
    // appears. Without this the assertion above would also pass against a
    // script that could never write the marker at all.
    let mut reading = start_bridge_with(script, Mode::Control, Some(8 * 1024), GATE_STALL_TIMEOUT);
    assert!(wait_for_binary_progressing(
        &mut reading.socket,
        b"DONE",
        Duration::from_secs(30)
    ));
    assert!(
        finished.exists(),
        "a reading client drained the whole output but the child never reached the marker"
    );
    reading
        .socket
        .close(None)
        .expect("close the reading client");
    join_within(reading.server, Duration::from_secs(5));
}
