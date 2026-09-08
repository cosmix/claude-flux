//! Wire-level tests for `/api/config`: the gates a write must clear, and the
//! bytes the route reads before it clears them.
//!
//! The semantics behind the route — what each scope writes, what it clears,
//! what it refuses — are tested without a socket in `config_api::tests`. What
//! can only be seen from out here is the request itself: an absent `Origin`, a
//! header a cross-site page could not have set, a `Content-Length` that lies.

use std::io::Write;
use std::net::TcpStream;

use super::{
    assert_security_headers, body, request, skip_without_loopback, start, stop, workspace,
};
use crate::commands::status::web::http::MAX_BODY_BYTES;

/// The token the running server will accept — the same process, so the same
/// `OnceLock`.
fn token() -> &'static str {
    crate::commands::status::web::config_api::test_token()
}

/// A `POST /api/config` with every gate satisfied except what `headers` and
/// `payload` override.
fn post(port: u16, headers: &str, payload: &str) -> String {
    request(
        port,
        &format!(
            "POST /api/config HTTP/1.1\r\nHost: localhost\r\n{headers}Content-Length: {}\r\n\r\n{payload}",
            payload.len()
        ),
    )
}

/// The headers a legitimate dashboard write carries.
fn valid_headers() -> String {
    format!(
        "Origin: http://127.0.0.1:7373\r\nContent-Type: application/json\r\nX-Loom-Csrf: {}\r\n",
        token()
    )
}

#[test]
fn get_api_config_serves_the_registry() {
    if skip_without_loopback("get_api_config_serves_the_registry") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let response = request(port, "GET /api/config HTTP/1.1\r\nHost: localhost\r\n\r\n");
    stop(running);
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(
        response.contains("Content-Type: application/json"),
        "{response}"
    );
    assert_security_headers(&response);
    let payload: serde_json::Value =
        serde_json::from_str(body(&response)).expect("payload is JSON");
    assert_eq!(
        payload["entries"].as_array().expect("entries").len(),
        crate::user_config::keys::KEYS.len()
    );
    assert_eq!(payload["csrf_token"], token());
}

#[test]
fn get_api_config_answers_head_without_a_body() {
    if skip_without_loopback("get_api_config_answers_head_without_a_body") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let response = request(port, "HEAD /api/config HTTP/1.1\r\nHost: localhost\r\n\r\n");
    stop(running);
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert_eq!(body(&response), "");
}

#[test]
fn a_cross_origin_get_is_refused() {
    if skip_without_loopback("a_cross_origin_get_is_refused") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let response = request(
        port,
        "GET /api/config HTTP/1.1\r\nHost: localhost\r\nOrigin: http://evil.example\r\n\r\n",
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 403"), "{response}");
    assert!(body(&response).contains("origin not allowed"), "{response}");
}

#[test]
fn a_write_needs_an_origin_even_though_a_read_does_not() {
    if skip_without_loopback("a_write_needs_an_origin_even_though_a_read_does_not") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    // The same absent Origin the GET above is served under.
    let response = post(
        port,
        &format!(
            "Content-Type: application/json\r\nX-Loom-Csrf: {}\r\n",
            token()
        ),
        r#"{"scope":"user","name":"update.check","value":"false"}"#,
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 403"), "{response}");
    assert!(body(&response).contains("origin not allowed"), "{response}");
}

#[test]
fn a_cross_origin_write_is_refused() {
    if skip_without_loopback("a_cross_origin_write_is_refused") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let response = post(
        port,
        &format!(
            "Origin: http://evil.example\r\nContent-Type: application/json\r\nX-Loom-Csrf: {}\r\n",
            token()
        ),
        r#"{"scope":"user","name":"update.check","value":"false"}"#,
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 403"), "{response}");
}

#[test]
fn a_write_without_a_csrf_token_is_refused() {
    if skip_without_loopback("a_write_without_a_csrf_token_is_refused") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let missing = post(
        port,
        "Origin: http://127.0.0.1:7373\r\nContent-Type: application/json\r\n",
        r#"{"scope":"project","name":"terminal.backend","value":"tmux"}"#,
    );
    let wrong = post(
        port,
        &format!(
            "Origin: http://127.0.0.1:7373\r\nContent-Type: application/json\r\nX-Loom-Csrf: {}\r\n",
            "0".repeat(64)
        ),
        r#"{"scope":"project","name":"terminal.backend","value":"tmux"}"#,
    );
    stop(running);
    for response in [&missing, &wrong] {
        assert!(response.starts_with("HTTP/1.1 403"), "{response}");
        assert!(body(response).contains("csrf"), "{response}");
    }
}

#[test]
fn a_write_must_declare_a_json_body() {
    if skip_without_loopback("a_write_must_declare_a_json_body") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let response = post(
        port,
        &format!(
            "Origin: http://127.0.0.1:7373\r\nContent-Type: text/plain\r\nX-Loom-Csrf: {}\r\n",
            token()
        ),
        r#"{"scope":"project","name":"terminal.backend","value":"tmux"}"#,
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 415"), "{response}");
}

#[test]
fn a_write_without_a_content_length_is_refused() {
    if skip_without_loopback("a_write_without_a_content_length_is_refused") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let response = request(
        port,
        &format!(
            "POST /api/config HTTP/1.1\r\nHost: localhost\r\n{}\r\n",
            valid_headers()
        ),
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 411"), "{response}");
}

#[test]
fn an_oversized_body_is_refused_by_its_declared_length() {
    if skip_without_loopback("an_oversized_body_is_refused_by_its_declared_length") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let response = request(
        port,
        &format!(
            "POST /api/config HTTP/1.1\r\nHost: localhost\r\n{}Content-Length: {}\r\n\r\n",
            valid_headers(),
            MAX_BODY_BYTES + 1
        ),
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 413"), "{response}");
    assert_security_headers(&response);
}

/// A body far longer than its `Content-Length` must not be read past the
/// declared count: the server answers the declared prefix and leaves the rest
/// for the drain, so a lying length can never make it buffer past the cap.
#[test]
fn a_lying_content_length_does_not_widen_what_is_read() {
    if skip_without_loopback("a_lying_content_length_does_not_widen_what_is_read") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let payload = r#"{"scope":"project","name":"terminal.backend","value":"tmux"}"#;
    let head = format!(
        "POST /api/config HTTP/1.1\r\nHost: localhost\r\n{}Content-Length: {}\r\n\r\n",
        valid_headers(),
        payload.len()
    );
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to test server");
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .expect("set read timeout");
    stream.write_all(head.as_bytes()).expect("write head");
    stream.write_all(payload.as_bytes()).expect("write body");
    // Far past the cap, and past the declared length: never read, never parsed.
    let _ = stream.write_all(&vec![b'x'; MAX_BODY_BYTES * 4]);
    let mut response = String::new();
    let _ = std::io::Read::read_to_string(&mut stream, &mut response);
    stop(running);
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(body(&response).contains("\"new\":\"tmux\""), "{response}");
}

#[test]
fn a_valid_write_round_trips_through_a_get() {
    if skip_without_loopback("a_valid_write_round_trips_through_a_get") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let written = post(
        port,
        &valid_headers(),
        r#"{"scope":"project","name":"context.ceiling_tokens","value":"900000"}"#,
    );
    let reread = request(port, "GET /api/config HTTP/1.1\r\nHost: localhost\r\n\r\n");
    stop(running);
    assert!(written.starts_with("HTTP/1.1 200"), "{written}");
    let payload: serde_json::Value = serde_json::from_str(body(&reread)).expect("payload is JSON");
    let ceiling = payload["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|entry| entry["name"] == "context.ceiling_tokens")
        .expect("the ceiling entry")
        .clone();
    assert_eq!(ceiling["project"]["value"], "900000");
    assert_eq!(ceiling["effective"]["source"], "project");
}

#[test]
fn a_post_to_any_other_path_is_still_a_405() {
    if skip_without_loopback("a_post_to_any_other_path_is_still_a_405") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let responses = [
        request(
            port,
            "POST /api/status HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n",
        ),
        request(
            port,
            "POST /api/configuration HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n",
        ),
    ];
    stop(running);
    for response in &responses {
        assert!(response.starts_with("HTTP/1.1 405"), "{response}");
        assert_eq!(body(response), "GET required");
    }
}

#[test]
fn a_write_from_a_foreign_host_header_never_reaches_the_route() {
    if skip_without_loopback("a_write_from_a_foreign_host_header_never_reaches_the_route") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let response = request(
        port,
        &format!(
            "POST /api/config HTTP/1.1\r\nHost: evil.example\r\n{}Content-Length: 2\r\n\r\n{{}}",
            valid_headers()
        ),
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 403"), "{response}");
    assert_eq!(body(&response), "host not allowed");
}
