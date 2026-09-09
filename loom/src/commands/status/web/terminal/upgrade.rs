//! Admission, resolution, and tmux hand-off for one terminal WebSocket.

use std::net::TcpStream;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, WebSocketConfig};

use crate::orchestrator::terminal::tmux::viewer::is_plain_identifier;

use super::bridge;
use super::pty::PtyChild;
use super::resolve::{self, attach_args, Refusal, Target};
use super::token;
use super::{Mode, WindowSize};
use crate::commands::status::web::connection::fail;
use crate::commands::status::web::http::{host_allowed, RequestHead};
use crate::commands::status::web::limits::{acquire_terminal_slot, Limits, Slot};
use crate::commands::status::web::TerminalLane;

const MAX_INBOUND_BYTES: usize = 64 * 1024;

impl TerminalLane {
    pub(in crate::commands::status::web) fn mint(port: u16) -> std::io::Result<Self> {
        Ok(Self::from_token(token::mint()?, port))
    }

    pub(in crate::commands::status::web) fn from_token(token: String, port: u16) -> Self {
        Self {
            token,
            cookie_name: Self::cookie_name(port),
        }
    }

    pub(in crate::commands::status::web) fn cookie_name(port: u16) -> String {
        token::cookie_name(port)
    }

    pub(in crate::commands::status::web) fn bootstrap_cookie(
        &self,
        query: Option<&str>,
    ) -> Option<Option<String>> {
        let presented = token::query_token(query)?;
        Some(token::matches(Some(presented), &self.token).then(|| {
            format!(
                "{}={presented}; Path=/; HttpOnly; SameSite=Strict",
                self.cookie_name
            )
        }))
    }

    pub(in crate::commands::status::web) fn cookie_matches(&self, cookie: Option<&str>) -> bool {
        token::matches(token::cookie_token(cookie, &self.cookie_name), &self.token)
    }
}

/// Upgrade one validated terminal request, or answer its pre-upgrade refusal.
pub(crate) fn handle_upgrade(
    mut stream: TcpStream,
    head: &RequestHead,
    base: &Path,
    lane: Option<&TerminalLane>,
    running: &AtomicBool,
    limits: &Arc<Limits>,
) {
    let Some((stage_id, control, _slot)) = admit(&mut stream, head, lane, running, limits) else {
        return;
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_INBOUND_BYTES))
        .max_frame_size(Some(MAX_INBOUND_BYTES));
    let Ok(socket) = tungstenite::accept_with_config(stream, Some(config)) else {
        tracing::warn!("dashboard terminal WebSocket handshake failed");
        return;
    };
    start_bridge(socket, stage_id, base, control, running);
}

fn admit<'a>(
    stream: &mut TcpStream,
    head: &'a RequestHead,
    lane: Option<&TerminalLane>,
    running: &AtomicBool,
    limits: &Arc<Limits>,
) -> Option<(&'a str, bool, Slot)> {
    // Re-asserted here rather than trusted to the caller: `connection::handle`
    // checks this ahead of routing today, but `handle_upgrade` is
    // `pub(crate)` and this lane is a keystroke-injection surface, so the
    // gate belongs to the lane it guards rather than to whichever caller
    // happens to sit above it.
    if !host_allowed(head.host.as_deref()) {
        fail(stream, 403, "Forbidden", b"host not allowed");
        return None;
    }
    let Some(lane) = lane else {
        fail(stream, 404, "Not Found", b"not found");
        return None;
    };
    if !origin_matches_host(head.origin.as_deref(), head.host.as_deref()) {
        fail(stream, 403, "Forbidden", b"origin not allowed");
        return None;
    }
    if !lane.cookie_matches(head.cookie.as_deref()) {
        fail(stream, 401, "Unauthorized", b"dashboard token required");
        return None;
    }
    let Some((stage_id, control)) = terminal_path(&head.path) else {
        fail(stream, 404, "Not Found", b"not found");
        return None;
    };
    let Some(slot) = acquire_terminal_slot(limits) else {
        fail(
            stream,
            503,
            "Service Unavailable",
            b"terminal limit reached",
        );
        return None;
    };
    if !running.load(Ordering::SeqCst) {
        fail(stream, 503, "Service Unavailable", b"server stopping");
        return None;
    }
    Some((stage_id, control, slot))
}

fn start_bridge(
    mut socket: tungstenite::WebSocket<TcpStream>,
    stage_id: &str,
    base: &Path,
    control: bool,
    running: &AtomicBool,
) {
    let target = match resolve::resolve_target(stage_id, base) {
        Ok(Ok(target)) => target,
        Ok(Err(refusal)) => return close_refusal(&mut socket, refusal),
        Err(error) => {
            tracing::warn!("dashboard terminal target resolution failed: {error}");
            let _ = socket.close(None);
            return;
        }
    };
    let mut command = tmux_command(&target, control);
    let Ok(child) = PtyChild::spawn(
        &mut command,
        WindowSize {
            cols: 120,
            rows: 36,
        },
    ) else {
        tracing::warn!("dashboard terminal could not start tmux attach client");
        let _ = socket.close(None);
        return;
    };
    bridge::run(socket, child, mode(control), running);
}

/// Require a browser origin with an HTTP(S) authority byte-equal to `Host`.
pub(super) fn origin_matches_host(origin: Option<&str>, host: Option<&str>) -> bool {
    let (Some(origin), Some(host)) = (origin, host) else {
        return false;
    };
    let Some((scheme, rest)) = origin.split_once("://") else {
        return false;
    };
    if !matches!(scheme, "http" | "https") {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    !authority.is_empty() && authority == host
}

pub(super) fn terminal_path(path: &str) -> Option<(&str, bool)> {
    let mut parts = path.strip_prefix("/ws/terminal/")?.split('/');
    let stage_id = parts.next()?;
    let control = match parts.next()? {
        "view" => false,
        "control" => true,
        _ => return None,
    };
    (parts.next().is_none() && is_plain_identifier(stage_id)).then_some((stage_id, control))
}

fn close_refusal(socket: &mut tungstenite::WebSocket<TcpStream>, refusal: Refusal) {
    let frame = CloseFrame {
        code: CloseCode::Library(refusal.close_code()),
        reason: refusal.reason().into(),
    };
    let _ = socket.close(Some(frame));
    let _ = socket.flush();
}

fn tmux_command(target: &Target, control: bool) -> Command {
    let mut command = Command::new("tmux");
    command.args(attach_args(target, mode(control)));
    command.env_remove("TMUX");
    command.env("TERM", "xterm-256color");
    command
}

fn mode(control: bool) -> Mode {
    if control {
        Mode::Control
    } else {
        Mode::View
    }
}
