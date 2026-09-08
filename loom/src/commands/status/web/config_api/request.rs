//! HTTP handling for `/api/config`: the gates a write must clear, and the
//! responses both methods write.
//!
//! The gate order is deliberate. `Host` is already behind us — `connection`
//! applies it to every request ahead of routing — so this runs `Origin`, then
//! the CSRF token, then the framing checks. The two access gates come first so
//! a request that has no business writing is refused before the server reads a
//! byte of its body.

use std::net::TcpStream;
use std::path::Path;

use super::super::connection::{drain_pending, respond};
use super::super::http::{self, RequestHead};
use super::{csrf, error_body};

/// The JSON content type every response on this route carries.
const JSON: &str = "application/json; charset=utf-8";

/// A refused request: status, reason phrase, and the body to serve.
struct Rejection(u16, &'static str, String);

/// Serve `GET`/`HEAD /api/config`.
///
/// Keeps [`http::origin_allowed`]'s lenient rule, matching `/api/status`: a
/// browser sends no `Origin` on a same-origin `GET`, and requiring one would
/// break the dashboard's own fetch. The CSRF token in this body is safe under
/// that rule because a cross-site page can cause this request but cannot read
/// its response — no CORS header is served here or anywhere else.
pub(in crate::commands::status::web) fn serve_get(
    stream: &mut TcpStream,
    head: &RequestHead,
    base: &Path,
) {
    if !http::origin_allowed(head.origin.as_deref()) {
        let body = error_body("origin not allowed");
        respond(stream, head, 403, "Forbidden", JSON, body.as_bytes());
        return;
    }
    match super::payload(base) {
        Ok(body) => respond(stream, head, 200, "OK", JSON, body.as_bytes()),
        Err(error) => {
            // Names absolute config paths, so it is logged rather than served.
            // The served message says which file to look at and where the
            // reason went, since the page this fails is the one an operator
            // would otherwise use to fix it.
            tracing::warn!("dashboard could not collect the config payload: {error}");
            let body = error_body(
                "config unavailable: a config file could not be read; the reason is on the loom status --web terminal",
            );
            respond(
                stream,
                head,
                500,
                "Internal Server Error",
                JSON,
                body.as_bytes(),
            );
        }
    }
}

/// Serve `POST /api/config`.
///
/// `prefix` is the body bytes that already arrived with the head.
pub(in crate::commands::status::web) fn handle_post(
    stream: &mut TcpStream,
    head: &RequestHead,
    prefix: Vec<u8>,
    base: &Path,
) {
    let (status, reason, body) = match gate(head) {
        Err(Rejection(status, reason, body)) => (status, reason, body),
        Ok(length) => match http::read_body(stream, prefix, length) {
            Ok(body) => {
                let (status, body) = super::update(base, &body);
                (status, reason_for(status), body)
            }
            Err(error) => {
                tracing::debug!("dashboard could not read a config request body: {error}");
                (400, "Bad Request", error_body("incomplete request body"))
            }
        },
    };
    // Unconditional, not just on the refusals: a rejected request never had its
    // body read, and even an accepted one leaves whatever the client sent past
    // its own `Content-Length`. Closing a socket with unread bytes still in the
    // receive buffer makes Linux answer RST instead of FIN, which discards the
    // response before it arrives.
    drain_pending(stream);
    let _ = http::write_response(stream, status, reason, JSON, body.as_bytes(), true);
}

/// Check every gate a write must clear, yielding the body length to read.
fn gate(head: &RequestHead) -> Result<usize, Rejection> {
    if !http::origin_allowed_strict(head.origin.as_deref()) {
        return Err(Rejection(
            403,
            "Forbidden",
            error_body("origin not allowed"),
        ));
    }
    if !csrf::verify(head.csrf_token.as_deref()) {
        return Err(Rejection(
            403,
            "Forbidden",
            error_body("csrf token missing or invalid"),
        ));
    }
    if !is_json(head.content_type.as_deref()) {
        return Err(Rejection(
            415,
            "Unsupported Media Type",
            error_body("expected Content-Type: application/json"),
        ));
    }
    length(head)
}

/// The declared body length, refused when absent, unreadable, or over the cap.
///
/// A missing `Content-Length` is refused rather than treated as an empty body:
/// this server speaks no chunked encoding, so a body it cannot frame is a
/// request it cannot honestly read.
fn length(head: &RequestHead) -> Result<usize, Rejection> {
    let Some(declared) = head.content_length.as_deref() else {
        return Err(Rejection(
            411,
            "Length Required",
            error_body("Content-Length is required"),
        ));
    };
    // Digits only, per RFC 9110 section 8.6: `parse` alone would accept `+8`
    // and surrounding whitespace, and a length this server reads differently
    // from whatever wrote it is the shape request smuggling takes.
    let declared = declared.trim();
    let numeric = !declared.is_empty() && declared.bytes().all(|byte| byte.is_ascii_digit());
    let Some(length) = numeric.then(|| declared.parse::<usize>().ok()).flatten() else {
        return Err(Rejection(
            400,
            "Bad Request",
            error_body("Content-Length is not a byte count"),
        ));
    };
    if length > http::MAX_BODY_BYTES {
        return Err(Rejection(
            413,
            "Content Too Large",
            error_body(&format!(
                "request body exceeds {} bytes",
                http::MAX_BODY_BYTES
            )),
        ));
    }
    Ok(length)
}

/// Whether the request declares a JSON body.
///
/// Required, and not only for parsing: `application/json` is not a
/// CORS-safelisted content type, so a cross-site page cannot send it without a
/// preflight this server fails. That makes the check part of the same defense
/// the CSRF header rests on.
fn is_json(content_type: Option<&str>) -> bool {
    content_type.is_some_and(|value| {
        value
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .eq_ignore_ascii_case("application/json")
    })
}

/// The reason phrase for a status the update path produced.
fn reason_for(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        409 => "Conflict",
        _ => "Internal Server Error",
    }
}
