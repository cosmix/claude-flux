//! Wire-level tests for the error responses the dashboard writes itself.
//!
//! Every one of these paths answers a request whose bytes were deliberately
//! left unread, so each asserts the response arrives *complete* rather than
//! being destroyed by an RST on close. Asserting on the response builder
//! instead would pass either way.

use std::panic::AssertUnwindSafe;
use std::time::Duration;

use super::terminal::terminal_options;
use super::{
    assert_security_headers, body, request, request_with_timeout, skip_without_loopback, start,
    start_with, stop, workspace,
};
use crate::commands::status::web::http::MAX_HEAD_BYTES;
use crate::commands::status::web::limits::{Lane, Limits, Slot, MAX_CONNECTIONS, MAX_WEBSOCKETS};

/// Read the `Content-Length` a response advertises.
fn content_length(response: &str) -> usize {
    response
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .and_then(|value| value.trim().parse().ok())
        .expect("response carries a Content-Length")
}

#[test]
fn post_with_a_body_is_answered_with_a_readable_405() {
    if skip_without_loopback("post_with_a_body_is_answered_with_a_readable_405") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    // Larger than one `read_head` chunk, so bytes are certain to be unread
    // when the 405 is written.
    let payload = "x".repeat(8 * 1024);
    let response = request(
        port,
        &format!(
            "POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{payload}",
            payload.len()
        ),
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 405"), "{response}");
    assert_security_headers(&response);
    assert_eq!(body(&response), "GET required");
}

#[test]
fn an_oversized_header_block_is_answered_with_a_readable_431() {
    if skip_without_loopback("an_oversized_header_block_is_answered_with_a_readable_431") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    // Never terminated, so the head can only ever overflow the budget.
    let response = request(
        port,
        &format!(
            "GET / HTTP/1.1\r\nHost: localhost\r\nX-Long: {}",
            "y".repeat(MAX_HEAD_BYTES + 4096)
        ),
    );
    stop(running);
    assert!(response.starts_with("HTTP/1.1 431"), "{response}");
    assert_security_headers(&response);
}

#[test]
fn a_silent_client_is_answered_with_a_408() {
    if skip_without_loopback("a_silent_client_is_answered_with_a_408") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let response = request_with_timeout(port, "", Duration::from_secs(20));
    stop(running);
    assert!(response.starts_with("HTTP/1.1 408"), "{response}");
    assert_security_headers(&response);
}

#[test]
fn an_unreadable_work_dir_is_answered_with_a_500_that_names_no_path() {
    if skip_without_loopback("an_unreadable_work_dir_is_answered_with_a_500_that_names_no_path") {
        return;
    }
    let temp = tempfile::tempdir().expect("create temporary workspace");
    // Never initialized, and never even created: no frame can be cached and no
    // fresh file snapshot can be collected, which is the 500's only trigger.
    let (port, running) = start(temp.path().join("missing"));
    let response = request(port, "GET /api/status HTTP/1.1\r\nHost: localhost\r\n\r\n");
    stop(running);
    assert!(response.starts_with("HTTP/1.1 500"), "{response}");
    assert_eq!(body(&response), "status snapshot unavailable");
    assert!(
        !response.contains(&temp.path().display().to_string()),
        "the 500 body leaked a work-directory path: {response}"
    );
    assert_security_headers(&response);
}

#[test]
fn head_omits_the_body_but_keeps_the_length() {
    if skip_without_loopback("head_omits_the_body_but_keeps_the_length") {
        return;
    }
    let (_temp, base) = workspace();
    let (port, running) = start(base);
    let head = request(port, "HEAD / HTTP/1.1\r\nHost: localhost\r\n\r\n");
    let get = request(port, "GET / HTTP/1.1\r\nHost: localhost\r\n\r\n");
    stop(running);
    assert_eq!(
        head.lines().next(),
        get.lines().next(),
        "HEAD and GET must agree on the status line"
    );
    assert!(body(&head).is_empty(), "HEAD response carried a body");
    assert_security_headers(&head);
    let length = content_length(&head);
    assert_eq!(length, body(&get).len());
    assert!(length > 0, "HEAD must report the length a GET would send");
}

#[test]
fn connection_slots_are_admitted_to_the_cap_and_refused_past_it() {
    let limits = Limits::new();
    let held = (0..MAX_CONNECTIONS)
        .map(|index| {
            Slot::acquire(&limits, Lane::Connection)
                .unwrap_or_else(|| panic!("connection slot {index} should be free"))
        })
        .collect::<Vec<_>>();
    assert!(Slot::acquire(&limits, Lane::Connection).is_none());
    drop(held);
    assert!(Slot::acquire(&limits, Lane::Connection).is_some());
}

#[test]
fn websocket_subscriptions_leave_connection_slots_for_the_page() {
    let limits = Limits::new();
    let subscriptions = (0..MAX_WEBSOCKETS)
        .map(|index| {
            (
                Slot::acquire(&limits, Lane::Connection)
                    .unwrap_or_else(|| panic!("connection slot {index} should be free")),
                Slot::acquire(&limits, Lane::WebSocket)
                    .unwrap_or_else(|| panic!("websocket slot {index} should be free")),
            )
        })
        .collect::<Vec<_>>();
    assert!(Slot::acquire(&limits, Lane::WebSocket).is_none());
    let reserve = (0..MAX_CONNECTIONS - MAX_WEBSOCKETS)
        .map(|index| {
            Slot::acquire(&limits, Lane::Connection)
                .unwrap_or_else(|| panic!("reserved slot {index} should be free"))
        })
        .collect::<Vec<_>>();
    assert_eq!(reserve.len(), MAX_CONNECTIONS - MAX_WEBSOCKETS);
    drop(subscriptions);
}

#[test]
fn a_panicking_connection_thread_returns_its_slot() {
    let limits = Limits::new();
    let held = (0..MAX_CONNECTIONS - 1)
        .map(|index| {
            Slot::acquire(&limits, Lane::Connection)
                .unwrap_or_else(|| panic!("connection slot {index} should be free"))
        })
        .collect::<Vec<_>>();
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let _slot = Slot::acquire(&limits, Lane::Connection).expect("last slot should be free");
        assert!(Slot::acquire(&limits, Lane::Connection).is_none());
        panic!("connection thread panicked while holding a slot");
    }));
    assert!(outcome.is_err(), "the closure was expected to panic");
    assert!(Slot::acquire(&limits, Lane::Connection).is_some());
    drop(held);
}

#[test]
fn snapshot_carries_terminals_flag() {
    if skip_without_loopback("snapshot_carries_terminals_flag") {
        return;
    }
    let (_enabled_temp, enabled_base) = workspace();
    let (enabled_port, enabled_running, _) = start_with(enabled_base, terminal_options());
    let enabled = request(
        enabled_port,
        "GET /api/status HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    let (_disabled_temp, disabled_base) = workspace();
    let (disabled_port, disabled_running) = start(disabled_base);
    let disabled = request(
        disabled_port,
        "GET /api/status HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    stop(enabled_running);
    stop(disabled_running);
    assert!(body(&enabled).contains("\"terminals\":true"), "{enabled}");
    assert!(
        body(&disabled).contains("\"terminals\":false"),
        "{disabled}"
    );
}
