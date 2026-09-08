//! Shared fixtures for the dashboard's connection-handling tests, split into
//! [`socket`] (tests that open a real loopback connection), [`errors`] (the
//! self-written HTTP error responses, also over loopback), [`terminal`] (the
//! terminal upgrade's refusal gates and the cookie/token bootstrap flow) and
//! [`pure`] (tests that call the routing, parsing, and classification
//! functions directly).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::fs::work_dir::WorkDir;
use crate::process::sandbox_probe::{loopback_bindable, skip_unless};
use tempfile::TempDir;

#[path = "tests/config_api.rs"]
mod config_api;
#[path = "tests/errors.rs"]
mod errors;
#[path = "tests/ports.rs"]
mod ports;
#[path = "tests/pure.rs"]
mod pure;
#[path = "tests/socket.rs"]
mod socket;

/// Build a fresh `.loom/work` directory for a test server.
pub(super) fn workspace() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("create temporary workspace");
    let base = temp.path().to_path_buf();
    WorkDir::new(&base)
        .expect("build work dir")
        .initialize()
        .expect("initialize work dir");
    (temp, base)
}

/// Assert a response carries every header the dashboard always sends.
pub(super) fn assert_security_headers(response: &str) {
    for header in [
        "Cache-Control: no-store",
        "X-Content-Type-Options: nosniff",
        "X-Frame-Options: DENY",
        "Content-Security-Policy: default-src 'self'",
    ] {
        assert!(response.contains(header), "missing {header}");
    }
}

/// Start a dashboard server on an ephemeral loopback port.
pub(super) fn start(base: PathBuf) -> (u16, Arc<AtomicBool>) {
    let (port, running, _) =
        start_with(base, crate::commands::status::web::ServeOptions::default());
    (port, running)
}

pub(super) fn start_with(
    base: PathBuf,
    options: crate::commands::status::web::ServeOptions,
) -> (
    u16,
    Arc<AtomicBool>,
    Arc<crate::commands::status::web::limits::Limits>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let port = listener.local_addr().expect("server address").port();
    let running = Arc::new(AtomicBool::new(true));
    let serve_running = running.clone();
    let limits = crate::commands::status::web::limits::Limits::new();
    let serve_limits = limits.clone();
    thread::spawn(move || {
        let _ = crate::commands::status::web::serve_with(
            listener,
            base,
            serve_running,
            options,
            serve_limits,
        );
    });
    (port, running, limits)
}

pub(super) fn stop(running: Arc<AtomicBool>) {
    running.store(false, Ordering::SeqCst);
    thread::sleep(Duration::from_millis(100));
}

pub(super) fn skip_without_loopback(test_name: &str) -> bool {
    skip_unless(
        loopback_bindable(),
        test_name,
        "loopback TCP is unavailable",
    )
}

/// Send one raw request and read the whole response back.
pub(super) fn request(port: u16, request: &str) -> String {
    request_with_timeout(port, request, Duration::from_secs(5))
}

fn request_with_timeout(port: u16, request: &str, timeout: Duration) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to test server");
    stream
        .set_read_timeout(Some(timeout))
        .expect("set read timeout");
    stream.write_all(request.as_bytes()).expect("write request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    response
}

pub(super) fn body(response: &str) -> &str {
    response.split_once("\r\n\r\n").map_or("", |(_, body)| body)
}
