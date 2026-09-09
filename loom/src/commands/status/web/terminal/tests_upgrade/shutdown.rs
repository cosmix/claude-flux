//! Shutdown behaviour: a stopped server refuses a new upgrade outright, and a
//! shutdown already in flight still drains the terminal it was holding before
//! it finishes.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::commands::status::web::limits::{acquire_terminal_slot, Limits};
use crate::commands::status::web::tests::{body, skip_without_loopback, workspace};
use crate::commands::status::web::{self, TerminalLane};
use tungstenite::client::IntoClientRequest;

use super::super::bridge;
use super::super::protocol::{Mode, WindowSize};
use super::super::pty::PtyChild;
use super::{cookie, terminal_options, terminal_request};

/// Poll-and-assert join, so a shutdown regression that leaves the bridge
/// thread running fails fast with a clear message instead of hanging on the
/// benign child's own lifetime.
fn join_within(handle: thread::JoinHandle<()>, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !handle.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(handle.is_finished(), "bridge thread did not stop in time");
    handle.join().expect("bridge thread");
}

#[test]
fn terminal_upgrade_after_stop_is_503() {
    if skip_without_loopback("terminal_upgrade_after_stop_is_503") {
        return;
    }
    let (_temp, base) = workspace();
    let token = "a".repeat(64);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let (server, _) = listener.accept().unwrap();
    let request = terminal_request(
        port,
        Some(&cookie(port, &token)),
        Some(&format!("http://127.0.0.1:{port}")),
    );
    let head = crate::commands::status::web::http::parse_head(request.as_bytes())
        .unwrap()
        .unwrap();
    client.write_all(request.as_bytes()).unwrap();
    let lane = TerminalLane {
        token,
        cookie_name: web::cookie_name_for_port(port),
    };
    let limits = Limits::new();
    let running = AtomicBool::new(false);
    web::terminal::handle_upgrade(server, &head, &base, Some(&lane), &running, &limits);
    let mut response = String::new();
    client.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 503"), "{response}");
    assert_eq!(body(&response), "server stopping");
}

/// Everything [`a_late_upgrade_after_shutdown_spawns_no_child`] needs to drive
/// a shutdown and check the terminal slot came back: the port to reconnect
/// to, the shutdown flag and slot accounting shared with the terminal
/// [`attach_live_terminal`] gives it, and the server thread's result.
struct RunningServer {
    port: u16,
    running: Arc<AtomicBool>,
    limits: Arc<Limits>,
    result: std::sync::mpsc::Receiver<anyhow::Result<()>>,
}

/// Starts a `serve_with` server on a fresh loopback listener.
fn start_server(base: PathBuf) -> RunningServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let running = Arc::new(AtomicBool::new(true));
    let limits = Limits::new();
    let server_running = running.clone();
    let server_limits = limits.clone();
    let (done, result) = std::sync::mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = done.send(web::serve_with(
            listener,
            base,
            server_running,
            terminal_options(),
            server_limits,
        ));
    });
    RunningServer {
        port,
        running,
        limits,
        result,
    }
}

/// Attaches one real, live terminal to `server`: a genuine `bridge::run`
/// holding a slot against the same `limits`/`running` Arcs the server itself
/// uses - the same `Slot` and shutdown path a real connection thread relies
/// on - so the count this polls up to 1 before returning is provably
/// non-zero before shutdown, rather than pinned at zero for the whole test
/// (as it was when nothing here ever incremented it). Returns the bridge
/// thread so the caller can join it once shutdown is underway.
fn attach_live_terminal(server: &RunningServer) -> thread::JoinHandle<()> {
    let bridge_listener = TcpListener::bind("127.0.0.1:0").expect("bind bridge listener");
    let bridge_port = bridge_listener.local_addr().unwrap().port();
    let bridge_running = server.running.clone();
    let bridge_limits = server.limits.clone();
    let bridge_thread = thread::spawn(move || {
        let _slot = acquire_terminal_slot(&bridge_limits).expect("acquire terminal slot");
        let (stream, _) = bridge_listener.accept().expect("accept bridge client");
        let socket = tungstenite::accept(stream).expect("accept bridge websocket");
        let mut command = Command::new("sleep");
        command.arg("30");
        let child = PtyChild::spawn(&mut command, WindowSize { cols: 80, rows: 24 })
            .expect("spawn benign PTY child");
        bridge::run(socket, child, Mode::View, bridge_running.as_ref());
    });
    let bridge_request = format!("ws://127.0.0.1:{bridge_port}/")
        .into_client_request()
        .expect("build bridge client request");
    let bridge_stream =
        TcpStream::connect(("127.0.0.1", bridge_port)).expect("connect bridge client");
    let (_bridge_client, _) =
        tungstenite::client(bridge_request, bridge_stream).expect("bridge client handshake");

    let deadline = Instant::now() + Duration::from_secs(3);
    while server.limits.terminal_count() == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        server.limits.terminal_count(),
        1,
        "the live terminal should be counted before shutdown"
    );
    bridge_thread
}

/// Flips `running` false, waits for the server and the bridge thread to both
/// stop, then sends a terminal upgrade on a connection that was already open
/// before shutdown completed - and asserts it spawns no child: an empty or
/// 503 response, and the terminal slot back at zero.
fn shut_down_and_assert_slot_freed(server: RunningServer, bridge_thread: thread::JoinHandle<()>) {
    let RunningServer {
        port,
        running,
        limits,
        result,
    } = server;

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    thread::sleep(Duration::from_millis(200));
    running.store(false, Ordering::SeqCst);
    assert!(result
        .recv_timeout(Duration::from_secs(4))
        .expect("server returned")
        .is_ok());
    join_within(bridge_thread, Duration::from_secs(3));

    let _ = client.write_all(
        terminal_request(
            port,
            Some(&cookie(port, &"a".repeat(64))),
            Some(&format!("http://127.0.0.1:{port}")),
        )
        .as_bytes(),
    );
    let mut response = String::new();
    let _ = client.read_to_string(&mut response);
    assert!(
        response.is_empty() || response.starts_with("HTTP/1.1 503"),
        "{response}"
    );
    assert_eq!(limits.terminal_count(), 0);
}

#[test]
fn a_late_upgrade_after_shutdown_spawns_no_child() {
    if skip_without_loopback("a_late_upgrade_after_shutdown_spawns_no_child") {
        return;
    }
    let (_temp, base) = workspace();
    let server = start_server(base);
    let bridge_thread = attach_live_terminal(&server);
    shut_down_and_assert_slot_freed(server, bridge_thread);
}
