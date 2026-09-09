//! The upgrade admission gate: the checks a `/ws/terminal/...` request must
//! pass before a PTY child is ever spawned - feature flag, `Origin`, the
//! token cookie, the stage/mode path shape, and the terminal capacity cap.

use crate::commands::status::web::limits::{acquire_terminal_slot, MAX_TERMINALS};
use crate::commands::status::web::tests::{
    body, request, skip_without_loopback, start, start_with, stop, workspace,
};

use super::super::upgrade::terminal_path;
use super::{connect_terminal, cookie, terminal_options, terminal_request};

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

#[test]
fn terminal_path_table() {
    let table = [
        ("/ws/terminal/my-stage/view", Some(("my-stage", false))),
        ("/ws/terminal/my-stage/control", Some(("my-stage", true))),
        ("/ws/terminal/my-stage", None),
        ("/ws/terminal/my-stage/view/extra", None),
        ("/ws/terminal/../view", None),
        ("/ws/terminal/a b/view", None),
        ("/ws/terminal/my-stage/attach", None),
        ("/ws/terminal//view", None),
        ("/api/status", None),
    ];
    for (path, expected) in table {
        assert_eq!(terminal_path(path), expected, "path {path:?}");
    }
}
