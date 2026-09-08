use std::net::{TcpListener, TcpStream};
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
};
use super::pty::PtyChild;

struct Fixture {
    socket: WebSocket<TcpStream>,
    running: Arc<AtomicBool>,
    server: JoinHandle<()>,
}

fn start_bridge(script: &'static str, mode: Mode) -> Fixture {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind bridge listener");
    let port = listener.local_addr().expect("listener address").port();
    let running = Arc::new(AtomicBool::new(true));
    let server_running = Arc::clone(&running);
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept bridge client");
        let socket = tungstenite::accept(stream).expect("accept WebSocket");
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        let child = PtyChild::spawn(&mut command, WindowSize { cols: 80, rows: 24 })
            .expect("spawn PTY child");
        bridge::run(socket, child, mode, server_running.as_ref());
    });
    let request = format!("ws://127.0.0.1:{port}/")
        .into_client_request()
        .expect("build WebSocket request");
    let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect bridge client");
    let (mut socket, _) = tungstenite::client(request, stream).expect("handshake bridge client");
    socket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("set client read timeout");
    Fixture {
        socket,
        running,
        server,
    }
}

fn wait_for_binary(socket: &mut WebSocket<TcpStream>, needle: &[u8], timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match socket.read() {
            Ok(Message::Binary(bytes))
                if bytes.windows(needle.len()).any(|part| part == needle) =>
            {
                return true;
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
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match socket.read() {
            Ok(Message::Close(Some(frame))) => return Some(frame.code.into()),
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
