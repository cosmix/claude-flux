use std::net::{TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tungstenite::client::IntoClientRequest;
use tungstenite::{Message, WebSocket};

use crate::process::sandbox_probe::{loopback_bindable, skip_unless};

use super::bridge;
use super::protocol::{
    parse_client_frame, ClientFrame, Mode, WindowSize, CLOSE_ENDED, CLOSE_SERVER_STOPPING,
    CLOSE_TOO_LARGE,
};
use super::pty::PtyChild;
use crate::commands::status::web::limits::MAX_INBOUND_BYTES;
mod backpressure;
mod spin;

struct Fixture {
    socket: WebSocket<TcpStream>,
    running: Arc<AtomicBool>,
    server: JoinHandle<()>,
}

fn start_bridge(script: impl Into<String>, mode: Mode) -> Fixture {
    start_bridge_with(script, mode, None, bridge::GATE_STALL_TIMEOUT)
}

/// `socket_bytes` shrinks both ends' kernel buffers. Left at `None` the
/// autotuned defaults hold megabytes, so a client that stops reading is
/// invisible to the bridge; shrunk, a client that stops reading stalls the
/// bridge's writes after a few tens of kilobytes.
///
/// `stall_timeout` is the bridge's give-up deadline for an input queue that
/// never drains. Everything but the test for that deadline passes
/// `GATE_STALL_TIMEOUT`, which is far longer than any test here runs and so
/// cannot fire.
fn start_bridge_with(
    script: impl Into<String>,
    mode: Mode,
    socket_bytes: Option<libc::c_int>,
    stall_timeout: Duration,
) -> Fixture {
    let script = script.into();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind bridge listener");
    let port = listener.local_addr().expect("listener address").port();
    let running = Arc::new(AtomicBool::new(true));
    let server_running = Arc::clone(&running);
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept bridge client");
        if let Some(bytes) = socket_bytes {
            shrink_buffers(&stream, bytes);
        }
        let socket = tungstenite::accept(stream).expect("accept WebSocket");
        let mut command = Command::new("sh");
        command.args(["-c", script.as_str()]);
        let child = PtyChild::spawn(&mut command, WindowSize { cols: 80, rows: 24 })
            .expect("spawn PTY child");
        bridge::run_with(socket, child, mode, server_running.as_ref(), stall_timeout);
    });
    let request = format!("ws://127.0.0.1:{port}/")
        .into_client_request()
        .expect("build WebSocket request");
    let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect bridge client");
    if let Some(bytes) = socket_bytes {
        shrink_buffers(&stream, bytes);
    }
    let (mut socket, _) = tungstenite::client(request, stream).expect("handshake bridge client");
    socket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("set client read timeout");
    // Several tests here flood a bridge whose child never drains, so the
    // client's own `send` is the call that would block if the kernel buffers
    // ever failed to hold the flood. Without this the test hangs; with it,
    // `send` panics on its `expect` and names the test that wedged.
    socket
        .get_mut()
        .set_write_timeout(Some(Duration::from_secs(5)))
        .expect("set client write timeout");
    Fixture {
        socket,
        running,
        server,
    }
}

fn shrink_buffers(stream: &TcpStream, bytes: libc::c_int) {
    for option in [libc::SO_SNDBUF, libc::SO_RCVBUF] {
        // SAFETY: `stream` owns a live socket fd for the duration of the call,
        // and the value pointer and length describe one complete `c_int`.
        let rc = unsafe {
            libc::setsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                option,
                (&raw const bytes).cast::<libc::c_void>(),
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        // Checked, not discarded: a failure here leaves the autotuned
        // megabyte-scale buffers in place, and every caller's stalled client
        // would then be absorbed by the network and test nothing at all.
        assert_ne!(rc, -1, "shrink socket buffer option {option}");
    }
}

/// Read until `needle` shows up in the bridge's output, or the deadline
/// passes. Output accumulates across messages because the bridge reads the
/// PTY in 16 KiB chunks, so any needle can straddle a message boundary.
fn wait_for_binary(socket: &mut WebSocket<TcpStream>, needle: &[u8], timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let mut seen: Vec<u8> = Vec::new();
    while Instant::now() < deadline {
        match socket.read() {
            Ok(Message::Binary(bytes)) => {
                seen.extend_from_slice(&bytes);
                if seen.windows(needle.len()).any(|part| part == needle) {
                    return true;
                }
            }
            Ok(Message::Close(_)) => return false,
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => panic!("read bridge output: {error}"),
        }
    }
    false
}

fn wait_for_close(socket: &mut WebSocket<TcpStream>, timeout: Duration) -> Option<u16> {
    wait_for_close_frame(socket, timeout).map(|(code, _)| code)
}

/// The close code *and* its reason. Two of the bridge's stops share code 4008
/// with `resolve`'s refusals, so the reason is the only thing that says which
/// one arrived.
fn wait_for_close_frame(
    socket: &mut WebSocket<TcpStream>,
    timeout: Duration,
) -> Option<(u16, String)> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match socket.read() {
            Ok(Message::Close(Some(frame))) => {
                return Some((frame.code.into(), frame.reason.as_str().to_owned()))
            }
            Ok(Message::Close(None)) => return None,
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => panic!("read bridge close: {error}"),
        }
    }
    None
}

fn join_within(handle: JoinHandle<()>, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !handle.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(handle.is_finished(), "bridge server did not stop in time");
    handle.join().expect("bridge server thread");
}

/// Mirrors `bridge.rs`'s private `MAX_PENDING_INPUT` (64 KiB, `bridge.rs:25`).
const MAX_PENDING_INPUT: usize = 64 * 1024;

/// Mirrors tungstenite's own `WebSocketConfig::read_buffer_size` default
/// (128 KiB), which `bridge.rs`'s `configure` never overrides.
const TUNGSTENITE_READ_BUFFER_SIZE: usize = 128 * 1024;

/// Input sized and shaped to shut the bridge's backpressure gate and keep it
/// shut, for a child that never reads its stdin.
///
/// 64-byte newline-terminated lines: the newlines keep the PTY's canonical
/// line buffer flow-controlling - refusing further writes once its own small
/// buffer holds a complete line it cannot deliver - where an un-terminated
/// run of bytes is silently discarded instead, which drains `pending` back to
/// zero and opens the gate again. The total clears `MAX_PENDING_INPUT` *and*
/// tungstenite's read buffer, because the first raw read off the wire fills
/// that buffer in one syscall and everything inside it is already off the
/// wire before it is ever decoded into `pending`; only bytes past both are
/// still queued in the kernel once the gate shuts.
fn gating_flood() -> Vec<u8> {
    let payload: Vec<u8> = (0..8192_u32)
        .flat_map(|line| format!("{line:063}\n").into_bytes())
        .collect();
    assert!(payload.len() > MAX_PENDING_INPUT + TUNGSTENITE_READ_BUFFER_SIZE);
    payload
}

fn skip_bridge_test(name: &str) -> bool {
    skip_unless(
        loopback_bindable(),
        name,
        "loopback sockets are unavailable in this sandbox",
    )
}

#[test]
fn bridge_relays_bytes_both_ways() {
    if skip_bridge_test("bridge_relays_bytes_both_ways") {
        return;
    }
    let mut fixture = start_bridge("read l; printf 'echo:%s\\n' \"$l\"", Mode::Control);
    fixture
        .socket
        .send(Message::text(r#"{"resize":{"cols":80,"rows":24}}"#))
        .expect("send resize");
    fixture
        .socket
        .send(Message::binary(b"ping\n".to_vec()))
        .expect("send terminal input");
    assert!(wait_for_binary(
        &mut fixture.socket,
        b"echo:ping",
        Duration::from_secs(10)
    ));
    assert_eq!(
        wait_for_close(&mut fixture.socket, Duration::from_secs(10)),
        Some(CLOSE_ENDED)
    );
    join_within(fixture.server, Duration::from_secs(3));
}

#[test]
fn bridge_relays_non_ascii_bytes_exactly() {
    if skip_bridge_test("bridge_relays_non_ascii_bytes_exactly") {
        return;
    }
    // `cat` is a pure byte pipe: unlike the shell `read` the other fixtures
    // in this file use, it does not strip or otherwise mangle any byte it is
    // handed, so it is the only fixture that can make a byte-exact assertion
    // honest.
    let mut fixture = start_bridge("cat", Mode::Control);
    // Lone high bytes (`0x80`, `0xFF`) plus the UTF-8 encoding of "e"-acute
    // (`0xC3 0xA9`). A trailing newline closes the PTY's canonical-mode
    // line so `cat` reads and echoes the payload back; the newline itself
    // is excluded from the needle below because `ONLCR` rewrites an
    // outgoing `\n` to `\r\n` and would break a literal match on it.
    let payload: &[u8] = &[0xC3, 0xA9, 0x80, 0xFF];
    let mut sent = payload.to_vec();
    sent.push(b'\n');
    fixture
        .socket
        .send(Message::binary(sent))
        .expect("send non-ASCII terminal input");
    assert!(wait_for_binary(
        &mut fixture.socket,
        payload,
        Duration::from_secs(10)
    ));
    fixture.socket.close(None).expect("close bridge client");
    join_within(fixture.server, Duration::from_secs(3));
}

#[test]
fn bridge_drops_input_in_view_mode() {
    if skip_bridge_test("bridge_drops_input_in_view_mode") {
        return;
    }
    let mut fixture = start_bridge("read l; printf 'echo:%s\\n' \"$l\"", Mode::View);
    fixture
        .socket
        .send(Message::binary(b"ping\n".to_vec()))
        .expect("send ignored terminal input");
    assert!(!wait_for_binary(
        &mut fixture.socket,
        b"echo:",
        Duration::from_secs(1)
    ));
    fixture.socket.close(None).expect("close bridge client");
    join_within(fixture.server, Duration::from_secs(3));
}

#[test]
fn bridge_closes_1001_when_the_server_stops() {
    if skip_bridge_test("bridge_closes_1001_when_the_server_stops") {
        return;
    }
    let mut fixture = start_bridge(
        "while read l; do printf 'echo:%s\\n' \"$l\"; done",
        Mode::Control,
    );
    fixture
        .socket
        .send(Message::binary(b"ping\n".to_vec()))
        .expect("send terminal input");
    assert!(wait_for_binary(
        &mut fixture.socket,
        b"echo:ping",
        Duration::from_secs(10)
    ));
    fixture.running.store(false, Ordering::SeqCst);
    assert_eq!(
        wait_for_close(&mut fixture.socket, Duration::from_secs(3)),
        Some(CLOSE_SERVER_STOPPING)
    );
    join_within(fixture.server, Duration::from_secs(3));
}

#[test]
fn parse_client_frame_table() {
    if skip_bridge_test("parse_client_frame_table") {
        return;
    }
    let valid = WindowSize { cols: 80, rows: 24 };
    let cases = vec![
        (
            Message::binary(b"abc".to_vec()),
            Some(ClientFrame::Input(b"abc".to_vec())),
        ),
        (
            Message::text(r#"{"resize":{"cols":80,"rows":24}}"#),
            Some(ClientFrame::Resize(valid)),
        ),
        (
            Message::text(r#"{"resize":{"cols":80,"rows":24}},"extra":true}"#),
            None,
        ),
        (Message::text("not json"), None),
        (Message::Ping(Default::default()), None),
        (Message::Pong(Default::default()), None),
        (Message::Close(None), None),
    ];
    for (message, expected) in cases {
        assert_eq!(parse_client_frame(&message), expected);
    }
}

#[test]
fn bridge_closes_1009_for_a_message_over_the_cap() {
    if skip_bridge_test("bridge_closes_1009_for_a_message_over_the_cap") {
        return;
    }
    // A child that outlives the close: `finish` drops the socket only after
    // reaping, and dropping it while the oversize payload sits unread in the
    // receive queue would reset the connection and lose the close frame.
    let mut fixture = start_bridge("sleep 1", Mode::Control);
    fixture
        .socket
        .send(Message::binary(vec![b'p'; MAX_INBOUND_BYTES + 1]))
        .expect("send an oversize paste");
    assert_eq!(
        wait_for_close(&mut fixture.socket, Duration::from_secs(10)),
        Some(CLOSE_TOO_LARGE)
    );
    join_within(fixture.server, Duration::from_secs(5));
}
