//! Tests for the terminal upgrade's refusal gates and the cookie/token
//! bootstrap flow that gates them.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::commands::status::web::limits::{acquire_terminal_slot, Limits, MAX_TERMINALS};
use crate::commands::status::web::tests::{
    assert_security_headers, body, request, skip_without_loopback, start, start_with, stop,
    workspace,
};
use crate::commands::status::web::{self, ServeOptions, TerminalLane};
use tungstenite::client::IntoClientRequest;

pub(in crate::commands::status::web) fn terminal_options() -> ServeOptions {
    ServeOptions {
        terminal_token: Some("a".repeat(64)),
    }
}

fn cookie(port: u16, value: &str) -> String {
    format!("{}={value}", web::cookie_name_for_port(port))
}

fn terminal_request(port: u16, cookie: Option<&str>, origin: Option<&str>) -> String {
    let cookie = cookie.map(|cookie| format!("Cookie: {cookie}\r\n"));
    let origin = origin.map(|origin| format!("Origin: {origin}\r\n"));
    format!(
        "GET /ws/terminal/missing/view HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n{}{}\r\n",
        origin.unwrap_or_default(), cookie.unwrap_or_default()
    )
}

fn connect_terminal(port: u16, token: &str) -> tungstenite::WebSocket<TcpStream> {
    let url = format!("ws://127.0.0.1:{port}/ws/terminal/missing/view");
    let mut request = url
        .as_str()
        .into_client_request()
        .expect("terminal request");
    request.headers_mut().insert(
        "Origin",
        format!("http://127.0.0.1:{port}").parse().unwrap(),
    );
    request
        .headers_mut()
        .insert("Cookie", cookie(port, token).parse().unwrap());
    tungstenite::client(request, TcpStream::connect(("127.0.0.1", port)).unwrap())
        .expect("terminal handshake")
        .0
}

#[test]
fn terminal_upgrade_when_disabled_is_404() {
    if skip_without_loopback("terminal_upgrade_when_disabled_is_404") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let response = request(port, &terminal_request(port, None, None));
    stop(running);
    assert!(response.starts_with("HTTP/1.1 404"), "{response}");
}

#[test]
fn terminal_upgrade_without_origin_is_403() {
    if skip_without_loopback("terminal_upgrade_without_origin_is_403") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running, _) = start_with(base, terminal_options());
    let response = request(
        port,
        &terminal_request(port, Some(&cookie(port, &"a".repeat(64))), None),
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 403"), "{response}");
}

#[test]
fn terminal_upgrade_with_foreign_origin_is_403() {
    if skip_without_loopback("terminal_upgrade_with_foreign_origin_is_403") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running, _) = start_with(base, terminal_options());
    let response = request(
        port,
        &terminal_request(
            port,
            Some(&cookie(port, &"a".repeat(64))),
            Some("http://evil.example"),
        ),
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 403"), "{response}");
}

#[test]
fn terminal_upgrade_without_token_is_401() {
    if skip_without_loopback("terminal_upgrade_without_token_is_401") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running, _) = start_with(base, terminal_options());
    let response = request(
        port,
        &terminal_request(port, None, Some(&format!("http://127.0.0.1:{port}"))),
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 401"), "{response}");
}

#[test]
fn terminal_upgrade_with_wrong_token_is_401() {
    if skip_without_loopback("terminal_upgrade_with_wrong_token_is_401") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running, _) = start_with(base, terminal_options());
    let response = request(
        port,
        &terminal_request(
            port,
            Some(&cookie(port, "wrong")),
            Some(&format!("http://127.0.0.1:{port}")),
        ),
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 401"), "{response}");
}

#[test]
fn terminal_unknown_stage_closes_4004() {
    if skip_without_loopback("terminal_unknown_stage_closes_4004") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running, _) = start_with(base, terminal_options());
    let frame = connect_terminal(port, &"a".repeat(64))
        .read()
        .expect("refusal frame");
    stop(running);
    assert!(
        matches!(frame, tungstenite::Message::Close(Some(frame)) if frame.code == tungstenite::protocol::frame::coding::CloseCode::Library(4004))
    );
}

#[test]
fn terminal_upgrade_with_same_site_other_port_origin_is_403() {
    if skip_without_loopback("terminal_upgrade_with_same_site_other_port_origin_is_403") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running, _) = start_with(base, terminal_options());
    let origin = format!("http://127.0.0.1:{}", port.saturating_add(1));
    let response = request(
        port,
        &terminal_request(port, Some(&cookie(port, &"a".repeat(64))), Some(&origin)),
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 403"), "{response}");
}

#[test]
fn token_bootstrap_sets_cookie_and_redirects() {
    if skip_without_loopback("token_bootstrap_sets_cookie_and_redirects") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running, _) = start_with(base, terminal_options());
    let response = request(
        port,
        &format!(
            "GET /?token={} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n",
            "a".repeat(64)
        ),
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 302"));
    assert_security_headers(&response);
    for value in ["Location: /", "HttpOnly", "SameSite=Strict", "Path=/"] {
        assert!(response.contains(value), "{response}");
    }
}

#[test]
fn token_bootstrap_with_wrong_token_is_403() {
    if skip_without_loopback("token_bootstrap_with_wrong_token_is_403") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running, _) = start_with(base, terminal_options());
    let response = request(
        port,
        &format!("GET /?token=wrong HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"),
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 403"), "{response}");
}

#[test]
fn plain_get_of_a_terminal_path_is_404() {
    if skip_without_loopback("plain_get_of_a_terminal_path_is_404") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let response = request(
        port,
        "GET /ws/terminal/x/view HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 404"), "{response}");
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
#[test]
fn a_late_upgrade_after_shutdown_spawns_no_child() {
    if skip_without_loopback("a_late_upgrade_after_shutdown_spawns_no_child") {
        return;
    }
    let (_temp, base) = workspace();
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
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    thread::sleep(Duration::from_millis(200));
    running.store(false, Ordering::SeqCst);
    assert!(result
        .recv_timeout(Duration::from_secs(4))
        .expect("server returned")
        .is_ok());
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
fn two_servers_do_not_share_a_cookie_name() {
    if skip_without_loopback("two_servers_do_not_share_a_cookie_name") {
        return;
    }
    let (_a_temp, a_base) = workspace();
    let (_b_temp, b_base) = workspace();
    let (a_port, a_running, _) = start_with(a_base, terminal_options());
    let (b_port, b_running, _) = start_with(b_base, terminal_options());
    let token = "a".repeat(64);
    let a = request(
        a_port,
        &format!("GET /?token={token} HTTP/1.1\r\nHost: 127.0.0.1:{a_port}\r\n\r\n"),
    );
    let b = request(
        b_port,
        &format!("GET /?token={token} HTTP/1.1\r\nHost: 127.0.0.1:{b_port}\r\n\r\n"),
    );
    assert!(a.contains(&format!(
        "Set-Cookie: {}=",
        web::cookie_name_for_port(a_port)
    )));
    assert!(b.contains(&format!(
        "Set-Cookie: {}=",
        web::cookie_name_for_port(b_port)
    )));
    drop(connect_terminal(a_port, &token));
    stop(a_running);
    stop(b_running);
}

#[test]
fn terminal_slots_are_capped() {
    if skip_without_loopback("terminal_slots_are_capped") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running, limits) = start_with(base, terminal_options());
    let held = (0..MAX_TERMINALS)
        .map(|_| acquire_terminal_slot(&limits).unwrap())
        .collect::<Vec<_>>();
    let token = "a".repeat(64);
    let response = request(
        port,
        &terminal_request(
            port,
            Some(&cookie(port, &token)),
            Some(&format!("http://127.0.0.1:{port}")),
        ),
    );
    assert!(response.starts_with("HTTP/1.1 503") && body(&response) == "terminal limit reached");
    drop(held);
    drop(connect_terminal(port, &token));
    stop(running);
}
