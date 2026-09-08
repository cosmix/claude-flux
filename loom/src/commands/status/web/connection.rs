//! Routing for one dashboard HTTP or WebSocket connection.

use std::io::Read;
use std::net::TcpStream;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::assets;
use super::broadcast::{self, Broadcaster};
use super::config_api;
use super::head::complete as complete_head;
use super::http::{self, RequestHead};
use super::limits::{Lane, Limits, Slot};
use super::ws;

/// How long a single peek may block, bounding how long a connection thread
/// ignores a shutdown request.
const PEEK_TIMEOUT: Duration = Duration::from_millis(250);

/// How long a single blocked `write` syscall may hold the connection thread.
///
/// `set_write_timeout` bounds one syscall, not a whole response, so this is
/// not a budget for serving `index.js`: a client that acknowledges a few bytes
/// just inside the timeout keeps `write_all` looping for far longer. What it
/// does rule out is the thread parking in `write_all` for good once a client
/// that stopped reading altogether fills its receive window — the same hazard
/// the WebSocket lane guards against. The whole-response bound comes from the
/// connection cap instead: a slow reader occupies one of [`MAX_CONNECTIONS`]
/// slots and no more. The budget is looser than the WebSocket lane's because
/// this one writes whole bundle assets rather than one small snapshot frame.
///
/// [`MAX_CONNECTIONS`]: super::limits::MAX_CONNECTIONS
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Body returned when the work directory cannot produce a snapshot. The
/// underlying error names absolute work-directory paths, so it is logged
/// rather than served.
const SNAPSHOT_UNAVAILABLE: &[u8] = b"status snapshot unavailable";

/// Bytes drained from an unfinished request before an error response.
const MAX_DRAIN_BYTES: usize = 64 * 1024;

/// Wall-clock bound on that drain.
///
/// The byte cap alone bounds nothing in time: a client trickling one byte per
/// read timeout satisfies every read, so the loop can run for as many
/// iterations as [`MAX_DRAIN_BYTES`] allows. [`reject_overloaded`] performs
/// that drain on the accept loop, where stalling stops the server answering
/// anyone at all, so the drain gives up on whichever bound it reaches first.
const DRAIN_DEADLINE: Duration = Duration::from_millis(300);

/// The non-WebSocket target selected from a request path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Route {
    Api,
    /// The read/write config surface — the one route that accepts `POST`.
    Config,
    Asset {
        body: &'static [u8],
        mime: &'static str,
    },
    Missing,
    Spa,
}

/// Handle one accepted connection to completion, or until `running` clears.
///
/// `_slot` is the connection's reservation: holding it here releases it when
/// this thread ends, panic included.
pub(super) fn handle(
    mut stream: TcpStream,
    broadcaster: &Broadcaster,
    base: &Path,
    running: &AtomicBool,
    limits: &Arc<Limits>,
    _slot: Slot,
) {
    if stream.set_read_timeout(Some(PEEK_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err()
    {
        return;
    }
    let Some(peeked) = complete_head(&mut stream, running) else {
        return;
    };
    // Ahead of routing, so the gate covers `/ws`, `/api/status` and the
    // embedded assets alike.
    if !http::host_allowed(peeked.host.as_deref()) {
        fail(&mut stream, 403, "Forbidden", b"host not allowed");
        return;
    }
    if peeked.path == "/ws" && peeked.upgrade_websocket {
        handle_websocket_upgrade(stream, &peeked, broadcaster, running, limits);
        return;
    }

    let Ok((head, body_prefix)) = http::read_head(&mut stream) else {
        fail(&mut stream, 400, "Bad Request", b"bad request");
        return;
    };
    // Routed before the method gate so `POST` opens for `/api/config` and
    // nothing else: every other path answers a `POST` with the same 405 it
    // always has.
    let route = route(&head.path);
    match head.method.as_str() {
        "GET" | "HEAD" => handle_route(&mut stream, &head, route, broadcaster, base),
        "POST" if route == Route::Config => {
            config_api::handle_post(&mut stream, &head, body_prefix, base)
        }
        _ => fail(&mut stream, 405, "Method Not Allowed", b"GET required"),
    }
}

/// Upgrade an accepted `/ws` connection, or reject it if the origin check or
/// the WebSocket sub-cap turns it away. `stream`'s head was only peeked, not
/// consumed, above; on success that leaves the handshake bytes unread for
/// `tungstenite::accept` to parse itself.
fn handle_websocket_upgrade(
    mut stream: TcpStream,
    peeked: &RequestHead,
    broadcaster: &Broadcaster,
    running: &AtomicBool,
    limits: &Arc<Limits>,
) {
    if !http::origin_allowed(peeked.origin.as_deref()) {
        fail(&mut stream, 403, "Forbidden", b"origin not allowed");
        return;
    }
    // Held until this subscription ends, so open tabs cannot consume the
    // connection slots ordinary requests need.
    let Some(_slot) = Slot::acquire(limits, Lane::WebSocket) else {
        fail(
            &mut stream,
            503,
            "Service Unavailable",
            b"dashboard subscription limit reached",
        );
        return;
    };
    if stream.set_read_timeout(None).is_ok() {
        ws::handle(stream, broadcaster.subscribe(), running);
    }
}

/// Read and discard whatever the client has already sent, up to
/// [`MAX_DRAIN_BYTES`].
///
/// Every caller reaches this with request bytes deliberately left unconsumed:
/// the head was peeked rather than read, or the head parsed but a body
/// followed it. Closing a socket with unread bytes still in the receive buffer
/// makes Linux answer with RST instead of FIN, which discards the response we
/// just wrote before it reaches the client. Draining first makes the close a
/// clean FIN, so the error status actually arrives.
pub(super) fn drain_pending(stream: &mut TcpStream) {
    if stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .is_err()
    {
        return;
    }
    let mut chunk = [0_u8; 4096];
    let mut drained = 0;
    let deadline = Instant::now() + DRAIN_DEADLINE;
    while drained < MAX_DRAIN_BYTES && Instant::now() < deadline {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(read) => drained += read,
        }
    }
}

/// Turn a connection away because no connection slot was free, without
/// occupying one. Runs on the accept loop, so it only drains and answers.
pub(super) fn reject_overloaded(stream: &mut TcpStream) {
    if stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err() {
        return;
    }
    fail(
        stream,
        503,
        "Service Unavailable",
        b"dashboard connection limit reached",
    );
}

/// Drain the unread request bytes, then write a plain-text error response.
pub(super) fn fail(stream: &mut TcpStream, status: u16, reason: &str, body: &[u8]) {
    drain_pending(stream);
    let _ = http::write_response(
        stream,
        status,
        reason,
        "text/plain; charset=utf-8",
        body,
        true,
    );
}

fn handle_route(
    stream: &mut TcpStream,
    head: &RequestHead,
    route: Route,
    broadcaster: &Broadcaster,
    base: &Path,
) {
    match route {
        Route::Api => serve_api(stream, head, broadcaster, base),
        Route::Config => config_api::serve_get(stream, head, base),
        Route::Asset { body, mime } => respond(stream, head, 200, "OK", mime, body),
        Route::Missing => respond(
            stream,
            head,
            404,
            "Not Found",
            "text/plain; charset=utf-8",
            b"not found",
        ),
        Route::Spa => serve_index(stream, head),
    }
}

fn serve_api(stream: &mut TcpStream, head: &RequestHead, broadcaster: &Broadcaster, base: &Path) {
    if !http::origin_allowed(head.origin.as_deref()) {
        respond(
            stream,
            head,
            403,
            "Forbidden",
            "text/plain; charset=utf-8",
            b"origin not allowed",
        );
        return;
    }
    match broadcaster
        .latest()
        .map(|frame| (*frame).clone())
        .map(Ok)
        .unwrap_or_else(|| broadcast::fresh_file_snapshot(base))
    {
        Ok(frame) => respond(
            stream,
            head,
            200,
            "OK",
            "application/json; charset=utf-8",
            frame.as_bytes(),
        ),
        Err(error) => {
            tracing::warn!("dashboard could not collect a status snapshot: {error}");
            respond(
                stream,
                head,
                500,
                "Internal Server Error",
                "text/plain; charset=utf-8",
                SNAPSHOT_UNAVAILABLE,
            );
        }
    }
}

/// The response for the SPA entry page: the embedded page, or a 503 naming
/// what to build when the bundle is absent.
pub(super) fn index_response(
    page: Option<&'static [u8]>,
) -> (u16, &'static str, &'static str, &'static [u8]) {
    match page {
        Some(body) => (200, "OK", "text/html; charset=utf-8", body),
        None => (
            503,
            "Service Unavailable",
            "text/plain; charset=utf-8",
            b"dashboard assets are not embedded; build web/dist and rebuild loom",
        ),
    }
}

fn serve_index(stream: &mut TcpStream, head: &RequestHead) {
    let (status, reason, content_type, body) = index_response(assets::index_html());
    respond(stream, head, status, reason, content_type, body);
}

/// Answer a routed request, omitting the body when the client sent HEAD.
pub(super) fn respond(
    stream: &mut TcpStream,
    head: &RequestHead,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
) {
    let _ = http::write_response(
        stream,
        status,
        reason,
        content_type,
        body,
        head.method != "HEAD",
    );
}

/// Resolve `path` to an embedded asset, leaving `/` to the SPA branch.
fn asset_for(path: &str) -> Option<(&'static [u8], &'static str)> {
    if path == "/" {
        return None;
    }
    assets::lookup(path)
}

/// Select an ordinary HTTP route, with assets taking precedence over SPA fallback.
pub(super) fn route(path: &str) -> Route {
    if path.split('/').any(|segment| segment == "..") {
        // Nothing leaks - asset lookup never touches the filesystem - but
        // answering a traversal probe with the SPA's 200 is a scanner flag.
        Route::Missing
    } else if let Some((body, mime)) = asset_for(path) {
        Route::Asset { body, mime }
    } else if path == "/api/status" {
        Route::Api
    } else if path == "/api/config" {
        Route::Config
    } else if path.starts_with("/assets/") || path.starts_with("/api/") {
        Route::Missing
    } else {
        Route::Spa
    }
}
