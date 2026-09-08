//! Socket/loopback dashboard tests: each of these binds or connects to a
//! real TCP listener, so they are skipped in sandboxes without loopback TCP.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tungstenite::client::IntoClientRequest;

use super::{
    assert_security_headers, body, request, skip_without_loopback, start, stop, workspace,
};
use crate::commands::status::web::broadcast::Broadcaster;
use crate::commands::status::web::http;
use crate::commands::status::web::limits::MAX_CONNECTIONS;
use crate::commands::status::web::model::{DaemonState, SnapshotSource, WebSnapshot};
use crate::fs::work_dir::WorkDir;
use crate::models::stage::Stage;
use crate::verify::transitions::create_stage;

fn split_request(port: u16, first: &str, headers: &str) -> TcpStream {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect split request");
    stream
        .write_all(first.as_bytes())
        .expect("write request line");
    stream.flush().expect("flush request line");
    thread::sleep(Duration::from_millis(20));
    stream
        .write_all(headers.as_bytes())
        .expect("write request headers");
    stream.flush().expect("flush request headers");
    thread::sleep(Duration::from_millis(20));
    stream.write_all(b"\r\n").expect("finish request head");
    stream.flush().expect("flush request head");
    stream
}

fn read_http_head(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set read timeout");
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        let mut byte = [0_u8; 1];
        stream.read_exact(&mut byte).expect("read response head");
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes).expect("response head is UTF-8")
}

fn read_websocket_text(stream: &mut TcpStream) -> String {
    let mut head = [0_u8; 2];
    stream
        .read_exact(&mut head)
        .expect("read WebSocket frame head");
    assert_eq!(head[0] & 0x0f, 1, "expected a text frame");
    let length = match head[1] & 0x7f {
        value @ 0..=125 => usize::from(value),
        126 => {
            let mut bytes = [0_u8; 2];
            stream.read_exact(&mut bytes).expect("read frame length");
            usize::from(u16::from_be_bytes(bytes))
        }
        _ => {
            let mut bytes = [0_u8; 8];
            stream.read_exact(&mut bytes).expect("read frame length");
            usize::try_from(u64::from_be_bytes(bytes)).expect("frame length fits usize")
        }
    };
    let mut payload = vec![0_u8; length];
    stream
        .read_exact(&mut payload)
        .expect("read WebSocket text");
    String::from_utf8(payload).expect("WebSocket text is UTF-8")
}

#[test]
fn read_head_rejects_oversized() {
    if skip_without_loopback("read_head_rejects_oversized") {
        return;
    }
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().expect("listener address").port();
    let writer = thread::spawn(move || {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect listener");
        let request = format!(
            "GET / HTTP/1.1\r\nX-Long: {}",
            "x".repeat(http::MAX_HEAD_BYTES)
        );
        stream
            .write_all(request.as_bytes())
            .expect("write oversized request");
    });
    let (mut stream, _) = listener.accept().expect("accept client");
    assert!(http::read_head(&mut stream).is_err());
    writer.join().expect("writer thread");
}

#[test]
fn api_status_returns_snapshot_json() {
    if skip_without_loopback("api_status_returns_snapshot_json") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let response = request(port, "GET /api/status HTTP/1.1\r\nHost: localhost\r\n\r\n");
    stop(running);
    assert!(response.starts_with("HTTP/1.1 200"));
    let snapshot: WebSnapshot = serde_json::from_str(body(&response)).expect("snapshot JSON");
    assert_eq!(snapshot.source, SnapshotSource::Files);
    assert_eq!(snapshot.daemon, DaemonState::NotRunning);
}

#[test]
fn api_status_rejects_foreign_origin() {
    if skip_without_loopback("api_status_rejects_foreign_origin") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let response = request(
        port,
        "GET /api/status HTTP/1.1\r\nHost: localhost\r\nOrigin: http://evil.example\r\n\r\n",
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 403"));
    assert_security_headers(&response);
}

#[test]
fn api_status_rejects_a_rebound_host() {
    if skip_without_loopback("api_status_rejects_a_rebound_host") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let response = request(
        port,
        "GET /api/status HTTP/1.1\r\nHost: evil.example\r\n\r\n",
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 403"), "{response}");
    assert_eq!(body(&response), "host not allowed");
    assert_security_headers(&response);
}

#[test]
fn index_serves_embedded_page() {
    if skip_without_loopback("index_serves_embedded_page") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let index = request(port, "GET / HTTP/1.1\r\nHost: localhost\r\n\r\n");
    let fallback = request(
        port,
        "GET /stages/anything HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    let missing = request(
        port,
        "GET /assets/nope.js HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    stop(running);
    assert!(index.starts_with("HTTP/1.1 200"));
    assert!(body(&index).contains("<div id=\"root\">"));
    assert!(fallback.starts_with("HTTP/1.1 200"));
    assert_eq!(body(&fallback), body(&index));
    assert!(missing.starts_with("HTTP/1.1 404"));
}

/// Complete a WebSocket handshake against a test server.
fn connect_websocket(port: u16) -> tungstenite::WebSocket<TcpStream> {
    let url = format!("ws://127.0.0.1:{port}/ws");
    let request = url
        .as_str()
        .into_client_request()
        .expect("build WebSocket request");
    let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect WebSocket");
    let (mut socket, _) =
        tungstenite::client(request, stream).expect("complete WebSocket handshake");
    socket
        .get_mut()
        // Generous: a republished frame waits out one file-poll interval.
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set WebSocket timeout");
    socket
}

fn read_snapshot(socket: &mut tungstenite::WebSocket<TcpStream>) -> WebSnapshot {
    let frame = socket.read().expect("read snapshot frame");
    serde_json::from_str(frame.to_text().expect("text frame")).expect("snapshot JSON")
}

#[test]
fn websocket_delivers_a_snapshot() {
    if skip_without_loopback("websocket_delivers_a_snapshot") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let mut socket = connect_websocket(port);
    assert_eq!(read_snapshot(&mut socket).source, SnapshotSource::Files);
    let _ = socket.close(None);
    stop(running);
}

/// The live-update loop must keep publishing past the first paint: a `handle`
/// that returned after one send would freeze the dashboard with every
/// single-frame test still passing.
#[test]
fn websocket_delivers_successive_frames() {
    if skip_without_loopback("websocket_delivers_successive_frames") {
        return;
    }
    let (_temp, base) = workspace();
    let work_root = WorkDir::new(&base)
        .expect("build work dir")
        .root()
        .to_path_buf();
    let (port, running) = start(base);
    let mut socket = connect_websocket(port);
    let first = read_snapshot(&mut socket);
    assert!(first.status.stages.is_empty());
    // An unchanged body is suppressed, so a second frame arrives only because
    // this stage changed the serialized tree.
    create_stage(&Stage::new("Live Update".to_owned(), None), &work_root).expect("create stage");
    let second = read_snapshot(&mut socket);
    stop(running);
    assert_eq!(second.status.stages.len(), 1);
    assert!(
        second.generated_at > first.generated_at,
        "the second frame must be newer than the first"
    );
}

#[test]
fn websocket_closes_on_client_close() {
    if skip_without_loopback("websocket_closes_on_client_close") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let mut socket = connect_websocket(port);
    let _ = read_snapshot(&mut socket);
    let peer = socket.get_mut();
    // A masked, empty close frame, written past tungstenite so the read below
    // observes the server's own close rather than client-side bookkeeping.
    peer.write_all(&[0x88, 0x80, 0, 0, 0, 0])
        .expect("send close frame");
    let mut byte = [0_u8; 1];
    let closed = peer.read(&mut byte);
    stop(running);
    match closed {
        Ok(0) => {}
        Ok(_) => panic!("server sent data after the client's close frame"),
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
        Err(error) => panic!("server did not close after the client's close frame: {error}"),
    }
}

#[test]
fn websocket_rejects_foreign_origin() {
    if skip_without_loopback("websocket_rejects_foreign_origin") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let response = request(
        port,
        "GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nOrigin: http://evil.example\r\n\r\n",
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 403"));
}

#[test]
fn broadcaster_publishes_file_snapshot_without_daemon() {
    if skip_without_loopback("broadcaster_publishes_file_snapshot_without_daemon") {
        return;
    }
    let (_temp, base) = workspace();
    let running = Arc::new(AtomicBool::new(true));
    let broadcaster = Broadcaster::spawn(base, running.clone(), false);
    let frame = broadcaster
        .subscribe()
        .recv_timeout(Duration::from_secs(5))
        .expect("file frame");
    stop(running);
    let snapshot: WebSnapshot = serde_json::from_str(&frame).expect("snapshot JSON");
    assert_eq!(snapshot.source, SnapshotSource::Files);
}

#[test]
fn handshake_survives_a_split_head() {
    if skip_without_loopback("handshake_survives_a_split_head") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let headers = concat!(
        "Host: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n",
        "Sec-WebSocket-Version: 13\r\n",
        "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
    );
    let mut stream = split_request(port, "GET /ws HTTP/1.1\r\n", headers);
    let response = read_http_head(&mut stream);
    assert!(response.starts_with("HTTP/1.1 101"));
    let snapshot: WebSnapshot =
        serde_json::from_str(&read_websocket_text(&mut stream)).expect("snapshot JSON");
    assert_eq!(snapshot.source, SnapshotSource::Files);
    stop(running);
}

#[test]
fn split_head_get_is_routed_normally() {
    if skip_without_loopback("split_head_get_is_routed_normally") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let mut stream = split_request(port, "GET / HTTP/1.1\r\n", "Host: localhost\r\n");
    let response = read_http_head(&mut stream);
    stop(running);
    assert!(response.starts_with("HTTP/1.1 200"));
    assert_security_headers(&response);
}

#[test]
fn a_connection_past_the_cap_is_answered_503() {
    if skip_without_loopback("a_connection_past_the_cap_is_answered_503") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);

    // Silent clients hold their slots for the whole head timeout, and the
    // accept loop takes connections in the order they were opened, so the
    // request below always arrives at a full pool.
    let held = (0..MAX_CONNECTIONS)
        .map(|index| {
            TcpStream::connect(("127.0.0.1", port))
                .unwrap_or_else(|error| panic!("hold connection {index}: {error}"))
        })
        .collect::<Vec<_>>();
    let response = request(port, "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
    drop(held);
    stop(running);

    assert!(
        response.starts_with("HTTP/1.1 503 Service Unavailable"),
        "{response}"
    );
    assert!(
        body(&response).contains("connection limit reached"),
        "{response}"
    );
}
