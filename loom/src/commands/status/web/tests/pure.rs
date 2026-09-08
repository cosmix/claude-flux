//! Pure-function dashboard tests: request-head parsing, origin checks, MIME
//! lookup, routing, and daemon-response classification. None of these open a
//! socket.

use std::panic::AssertUnwindSafe;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use clap::CommandFactory;

use super::{assert_security_headers, workspace};
use crate::cli::Cli;
use crate::commands::status::data::StatusData;
use crate::commands::status::web::broadcast::{self, DaemonStep, SUBSCRIBER_QUEUE_DEPTH};
use crate::commands::status::web::connection::{self, Route};
use crate::commands::status::web::limits::{Lane, Limits, Slot, MAX_CONNECTIONS, MAX_WEBSOCKETS};
use crate::commands::status::web::model::{SnapshotSource, WebSnapshot};
use crate::commands::status::web::{assets, http, DEFAULT_PORT};
use crate::daemon::Response;
use crate::fs::work_dir::WorkDir;

#[test]
fn parse_head_reads_method_path_and_upgrade() {
    let request = b"GET /ws?x=1 HTTP/1.1\r\nHost: a\r\nUpgrade: websocket\r\nOrigin: http://127.0.0.1:7373\r\n\r\n";
    let head = http::parse_head(request)
        .expect("parse request")
        .expect("complete head");
    assert_eq!(head.method, "GET");
    assert_eq!(head.path, "/ws");
    assert!(head.upgrade_websocket);
    assert_eq!(head.origin.as_deref(), Some("http://127.0.0.1:7373"));
    assert_eq!(head.host.as_deref(), Some("a"));
}

#[test]
fn parse_head_returns_none_on_partial() {
    assert!(http::parse_head(b"GET / HTTP/1.1\r\nHost: localhost\r\n")
        .expect("parse partial request")
        .is_none());
}

#[test]
fn parse_head_rejects_duplicate_gate_headers() {
    for request in [
        b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nHost: evil.example\r\n\r\n".as_slice(),
        b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: http://127.0.0.1\r\nOrigin: http://evil.example\r\n\r\n".as_slice(),
    ] {
        assert!(
            http::parse_head(request).is_err(),
            "a duplicated gate header must not resolve to its first value"
        );
    }
}

#[test]
fn origin_allowed_accepts_loopback_and_rejects_foreign() {
    for origin in [
        None,
        Some("http://127.0.0.1:5173"),
        Some("http://localhost:7373"),
        Some("https://localhost"),
        Some("http://LocalHost:7373"),
        Some("http://[::1]:7373"),
    ] {
        assert!(http::origin_allowed(origin), "{origin:?}");
    }
    for origin in [
        "http://evil.example",
        "http://127.0.0.1.evil.example",
        "http://[2001:db8::1]:7373",
        "null",
    ] {
        assert!(!http::origin_allowed(Some(origin)));
    }
}

#[test]
fn host_allowed_accepts_loopback_and_rejects_everything_else() {
    for host in [
        "127.0.0.1:41599",
        "127.0.0.1",
        "localhost",
        "LOCALHOST",
        "[::1]:7373",
    ] {
        assert!(http::host_allowed(Some(host)), "{host}");
    }
    for host in ["evil.example", "127.0.0.1.evil.example", "loom.test:7373"] {
        assert!(!http::host_allowed(Some(host)), "{host}");
    }
    // HTTP/1.1 mandates Host, so an absent one is a rejection.
    assert!(!http::host_allowed(None));
}

#[test]
fn mime_for_known_extensions() {
    assert_eq!(
        assets::mime_for("assets/index.js"),
        "text/javascript; charset=utf-8"
    );
    assert_eq!(assets::mime_for("app.css"), "text/css; charset=utf-8");
    assert_eq!(assets::mime_for("font.woff2"), "font/woff2");
    assert_eq!(assets::mime_for("blob.bin"), "application/octet-stream");
}

#[test]
fn route_prefers_assets_then_api_then_spa_fallback() {
    assert_eq!(connection::route("/api/status"), Route::Api);
    assert_eq!(connection::route("/api/bogus"), Route::Missing);
    assert_eq!(connection::route("/assets/nope.js"), Route::Missing);
    assert_eq!(connection::route("/stages/anything"), Route::Spa);
}

#[test]
fn index_response_reports_a_missing_bundle() {
    let (status, _, content_type, body) = connection::index_response(None);
    assert_eq!(status, 503);
    assert_eq!(content_type, "text/plain; charset=utf-8");
    assert!(String::from_utf8_lossy(body).contains("web/dist"));
    let (status, _, content_type, body) = connection::index_response(Some(b"<div id=\"root\">"));
    assert_eq!(status, 200);
    assert_eq!(content_type, "text/html; charset=utf-8");
    assert_eq!(body, b"<div id=\"root\">");
}

#[test]
fn embedded_bundle_is_not_empty() {
    assert!(
        !assets::WEB_ASSETS.is_empty(),
        "the dashboard bundle is not embedded; run `cd web && bun install && bun run build`, then rebuild loom"
    );
}

#[test]
fn route_rejects_traversal_before_the_spa_fallback() {
    for path in ["/../../etc/passwd", "/assets/../secret", "/.."] {
        assert_eq!(connection::route(path), Route::Missing, "{path}");
    }
}

#[test]
fn snapshot_frame_serializes_a_daemon_update() {
    let (_temp, base) = workspace();
    let work_dir = WorkDir::new(&base).expect("build work dir");
    let frame = broadcast::snapshot_frame(
        work_dir.root(),
        StatusData::default(),
        SnapshotSource::Daemon,
    )
    .expect("serialize daemon snapshot");
    let snapshot: WebSnapshot = serde_json::from_str(&frame).expect("snapshot JSON");
    assert_eq!(snapshot.source, SnapshotSource::Daemon);
}

#[test]
fn oversized_daemon_response_is_degraded() {
    let (_temp, base) = workspace();
    let work_dir = WorkDir::new(&base).expect("build work dir");
    let message = "daemon response exceeded the wire limit".to_owned();
    match broadcast::classify_response(
        work_dir.root(),
        Response::Error {
            message: message.clone(),
        },
    ) {
        Ok(DaemonStep::Degraded(actual)) => assert_eq!(actual, message),
        _ => panic!("Response::Error must degrade to the file lane"),
    }
    assert!(matches!(
        broadcast::classify_response(
            work_dir.root(),
            Response::StatusUpdate {
                data: Box::new(StatusData::default()),
            },
        ),
        Ok(DaemonStep::Frame(_))
    ));
}

#[test]
fn publish_skips_an_unchanged_tree() {
    let (_temp, base) = workspace();
    let work_dir = WorkDir::new(&base).expect("build work dir");
    let broadcaster = broadcast::Broadcaster::new();
    let receiver = broadcaster.subscribe();
    broadcast::poll_files_once(&broadcaster, &work_dir, work_dir.root(), None)
        .expect("first file poll");
    broadcast::poll_files_once(&broadcaster, &work_dir, work_dir.root(), None)
        .expect("second file poll");
    receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("first frame");
    assert!(receiver.recv_timeout(Duration::from_millis(150)).is_err());
}

#[test]
fn poll_files_once_carries_the_degrade_notice() {
    let (_temp, base) = workspace();
    let work_dir = WorkDir::new(&base).expect("build work dir");
    let broadcaster = broadcast::Broadcaster::new();
    let receiver = broadcaster.subscribe();
    broadcast::poll_files_once(
        &broadcaster,
        &work_dir,
        work_dir.root(),
        Some("daemon lane degraded"),
    )
    .expect("file poll carrying a notice");
    let frame = receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("degraded frame");
    let snapshot: WebSnapshot = serde_json::from_str(&frame).expect("snapshot JSON");
    assert_eq!(snapshot.notice.as_deref(), Some("daemon lane degraded"));
    // Pins the real publish path (not just the fixture helper) to the
    // build's tag-derived version.
    assert_eq!(snapshot.version, crate::version::VERSION);
}

#[test]
fn a_stalled_subscriber_is_dropped_once_its_queue_fills() {
    let broadcaster = broadcast::Broadcaster::new();
    let receiver = broadcaster.subscribe();
    // Frames that never parse as a snapshot fall back to comparing the raw
    // JSON, so each of these counts as a change and is published.
    for index in 0..SUBSCRIBER_QUEUE_DEPTH + 4 {
        broadcaster.publish(format!("{{\"frame\":{index}}}"));
    }
    let mut delivered = 0;
    while receiver.try_recv().is_ok() {
        delivered += 1;
    }
    assert_eq!(delivered, SUBSCRIBER_QUEUE_DEPTH);
    assert_eq!(receiver.try_recv(), Err(TryRecvError::Disconnected));
}

#[test]
fn the_cli_preserves_omitted_and_explicit_dashboard_ports() {
    let absent = Cli::command()
        .try_get_matches_from(["loom", "status"])
        .expect("parse `loom status`");
    let absent_status = absent
        .subcommand_matches("status")
        .expect("status subcommand");
    assert!(!absent_status.contains_id("web"));

    let bare = Cli::command()
        .try_get_matches_from(["loom", "status", "--web"])
        .expect("parse `loom status --web`");
    let bare_status = bare
        .subcommand_matches("status")
        .expect("status subcommand");
    assert!(bare_status.contains_id("web"));
    assert_eq!(bare_status.get_one::<u16>("web"), None);

    for port in [0, DEFAULT_PORT] {
        let port = port.to_string();
        let expected = port.parse::<u16>().expect("valid test port");
        let matches = Cli::command()
            .try_get_matches_from(["loom", "status", "--web", &port])
            .expect("parse explicit dashboard port");
        let status = matches
            .subcommand_matches("status")
            .expect("status subcommand");
        assert!(status.contains_id("web"));
        assert_eq!(status.get_one::<u16>("web"), Some(&expected));
    }
}

#[test]
fn every_http_response_carries_the_security_headers() {
    for (status, reason) in [
        (200, "OK"),
        (403, "Forbidden"),
        (404, "Not Found"),
        (405, "Method Not Allowed"),
        (408, "Request Timeout"),
        (431, "Request Header Fields Too Large"),
        (500, "Internal Server Error"),
        (503, "Service Unavailable"),
    ] {
        let response =
            String::from_utf8(http::response_bytes(status, reason, "text/plain", b"body"))
                .expect("response bytes are UTF-8");
        assert_security_headers(&response);
        for directive in [
            "connect-src 'self';",
            "base-uri 'none';",
            "form-action 'none';",
            "frame-ancestors 'none'",
        ] {
            assert!(response.contains(directive), "missing {directive}");
        }
        assert!(
            !response.contains("ws://"),
            "connect-src must not name loopback WebSocket ports"
        );
    }
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
    // A live `/ws` thread holds a slot in both lanes, so the fixture does too.
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
