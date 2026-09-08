//! Reading a request head off a dashboard connection without consuming it.

use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::connection::fail;
use super::http::{self, RequestHead, MAX_HEAD_BYTES};

/// Total budget for a client to deliver a complete request head.
pub(super) const HEAD_TIMEOUT: Duration = Duration::from_secs(5);

/// Outcome of one peek-and-parse attempt for a request head.
enum HeadAttempt {
    /// The peeked bytes hold a complete head.
    Ready(Box<RequestHead>),
    /// The head is still partial; peek again after a short sleep.
    Retry,
    /// The head can never complete (too large, timed out, or malformed); an
    /// HTTP error response has already been written to `stream`.
    Failed,
}

/// Write a plain-text HTTP error response and report the head as failed.
fn fail_head(stream: &mut TcpStream, status: u16, reason: &str, body: &[u8]) -> HeadAttempt {
    fail(stream, status, reason, body);
    HeadAttempt::Failed
}

/// Peek up to `MAX_HEAD_BYTES` without consuming them, so `/ws` can later hand
/// the socket to `tungstenite::accept` with the handshake bytes still unread,
/// and classify the result as complete, retryable, or a terminal failure.
fn peek_head(stream: &mut TcpStream, buffer: &mut [u8], started: Instant) -> HeadAttempt {
    match stream.peek(buffer) {
        Ok(0) => {
            tracing::debug!("dashboard client closed before sending a request head");
            HeadAttempt::Failed
        }
        Ok(read) => match http::parse_head(&buffer[..read]) {
            Ok(Some(head)) => HeadAttempt::Ready(Box::new(head)),
            Ok(None) if read >= MAX_HEAD_BYTES => fail_head(
                stream,
                431,
                "Request Header Fields Too Large",
                b"request head too large",
            ),
            Ok(None) if started.elapsed() >= HEAD_TIMEOUT => {
                fail_head(stream, 408, "Request Timeout", b"request head timed out")
            }
            Ok(None) => HeadAttempt::Retry,
            Err(error) => {
                tracing::debug!("dashboard request head was invalid: {error}");
                fail_head(stream, 400, "Bad Request", b"bad request")
            }
        },
        // A client that connects and then says nothing times out every peek;
        // once the budget is spent it gets the 408 this branch advertises.
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            if started.elapsed() < HEAD_TIMEOUT {
                HeadAttempt::Retry
            } else {
                fail_head(stream, 408, "Request Timeout", b"request head timed out")
            }
        }
        Err(error) => {
            tracing::debug!("dashboard request peek failed: {error}");
            HeadAttempt::Failed
        }
    }
}

/// Peek-retry until the request head is complete, times out, overflows the
/// head budget, or fails to parse; on any terminal outcome besides success,
/// the caller has already had an HTTP error response written for it.
pub(super) fn complete(stream: &mut TcpStream, running: &AtomicBool) -> Option<RequestHead> {
    let started = Instant::now();
    let mut buffer = [0_u8; MAX_HEAD_BYTES];
    while running.load(Ordering::SeqCst) {
        match peek_head(stream, &mut buffer, started) {
            HeadAttempt::Ready(head) => return Some(*head),
            HeadAttempt::Retry => std::thread::sleep(Duration::from_millis(5)),
            HeadAttempt::Failed => return None,
        }
    }
    None
}
