//! `loom status --web`: an embedded dashboard with live WebSocket snapshots.
//!
//! # Access control
//!
//! The daemon authenticates its own clients with the `user.token` that
//! `status::ui::tui::daemon_client` presents. The dashboard
//! holds that token on the operator's behalf and then serves the same status
//! data over `/api/status` and `/ws` to any process on the host that can reach
//! 127.0.0.1 - including other local users - with only `Host` and `Origin`
//! checks to keep a browser on another site from reading it. That is the intended
//! trade-off for an operator-run localhost dashboard, but it is a deliberate
//! downgrade of the daemon's authentication model, not an oversight: do not
//! bind this server to a non-loopback address without adding authentication.
//!
//! ## The write surface
//!
//! `/api/config` (`config_api`) is the one route that changes anything: it
//! reads and writes `~/.loom/config.toml` and `.loom/work/config.toml`. It
//! accepts the same reads as everything else here, and gates its writes three
//! deep:
//!
//! 1. **`Host`** - the DNS-rebinding gate `connection::handle` already applies
//!    to every request ahead of routing.
//! 2. **`Origin`, strictly** - `http::origin_allowed_strict` rather than
//!    `http::origin_allowed`. Absence is fine for a same-origin `GET`, which
//!    carries no `Origin` at all, and is refused for a write, which always
//!    would.
//! 3. **A double-submit CSRF token** - minted once per server process, handed
//!    out only in the `GET /api/config` body, required in the `X-Loom-Csrf`
//!    header of every `POST`, and compared in constant time.
//!
//! The third gate holds only because this server sends no CORS header
//! anywhere: that is what stops a cross-site page reading the token out of the
//! `GET` or setting the header on a `POST`. Adding one would defeat it.
//! Request bodies are capped at `http::MAX_BODY_BYTES`, and no response
//! anywhere names an absolute path - a failure that would is logged and served
//! generically, because every local process can read this server.
//!
//! What the write surface does NOT do is raise the read posture above: a local
//! process that could already read the ledger can now also change loom's
//! configuration, which is a real widening of what a same-host caller can do
//! and the reason the gates above are not optional.
//!
//! ## Browser terminals
//!
//! Snapshot routes remain available exactly as before. The opt-in terminal
//! lane requires a startup token cookie and an `Origin` authority equal to the
//! request `Host`; anyone who can read the printed tokenized URL can type into
//! the agents. When enabled, this process adopts the live daemon's recorded
//! tmux socket directory once before serving; without a live daemon it keeps
//! the ambient `TMUX_TMPDIR`.

mod assets;
mod broadcast;
mod config_api;
mod connection;
mod head;
mod http;
mod limits;
pub mod model;
mod terminal;
#[cfg(test)]
mod tests;
mod ws;

use std::io::ErrorKind;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::fs::tmux_tmpdir::{adopt_recorded_tmux_tmpdir, TmuxTmpdirAdoption};
use crate::fs::work_dir::WorkDir;

/// First port considered by `loom status --web` without a value.
pub const DEFAULT_PORT: u16 = 7373;

/// What `loom status --web` was started with.
#[derive(Debug, Clone, Default)]
pub struct ServeOptions {
    /// `Some(token)` when terminals are enabled.
    pub terminal_token: Option<String>,
}

/// The terminal lane configuration resolved from the listener's actual port.
#[derive(Clone)]
pub(super) struct TerminalLane {
    pub(super) token: String,
    pub(super) cookie_name: String,
}

#[cfg(test)]
pub(super) fn cookie_name_for_port(port: u16) -> String {
    TerminalLane::cookie_name(port)
}

/// Start the dashboard server on loopback until Ctrl-C.
pub fn execute(port: Option<u16>, terminals: bool) -> Result<()> {
    let work_dir = WorkDir::new(".")?;
    work_dir.load()?;
    let listener = bind_listener(port)?;
    let actual_port = listener.local_addr()?.port();
    let terminal_lane = terminals
        .then(|| TerminalLane::mint(actual_port))
        .transpose()?;
    if terminals {
        log_adoption(adopt_for(&work_dir));
    }
    match terminal_lane.as_ref() {
        Some(lane) => println!(
            "loom dashboard: http://127.0.0.1:{actual_port}/?token={}  (terminals enabled; Ctrl-C to stop)",
            lane.token
        ),
        None => println!("loom dashboard: http://127.0.0.1:{actual_port}/  (Ctrl-C to stop)"),
    }
    if assets::WEB_ASSETS.is_empty() {
        eprintln!(
            "warning: dashboard assets are not embedded in this binary; run `cd web && bun install && bun run build`, then rebuild loom"
        );
    }

    let running = Arc::new(AtomicBool::new(true));
    let on_ctrl_c = running.clone();
    ctrlc::set_handler(move || on_ctrl_c.store(false, Ordering::SeqCst))
        .context("failed to install Ctrl-C handler")?;
    serve(
        listener,
        PathBuf::from("."),
        running,
        ServeOptions {
            terminal_token: terminal_lane.map(|lane| lane.token),
        },
    )
}

/// Adopt the live daemon's tmux directory for terminal attachments only.
pub(super) fn adopt_for(work_dir: &WorkDir) -> TmuxTmpdirAdoption {
    adopt_recorded_tmux_tmpdir(work_dir.root())
}

fn log_adoption(adoption: TmuxTmpdirAdoption) {
    if let TmuxTmpdirAdoption::Adopted { recorded, ambient } = adoption {
        let display = |value: &Option<std::ffi::OsString>| {
            value
                .as_ref()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| "<unset>".to_owned())
        };
        println!(
            "Using the orchestrator's tmux socket dir (TMUX_TMPDIR={}) instead of this shell's ({})",
            display(&recorded), display(&ambient)
        );
    }
}

/// Bind an explicit port exactly, or select the first available default-range port.
pub(super) fn bind_listener(port: Option<u16>) -> Result<TcpListener> {
    match port {
        Some(port) => bind_loopback(port),
        None => bind_first_available_loopback_port(DEFAULT_PORT),
    }
}

fn bind_loopback(port: u16) -> Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port))
        .with_context(|| format!("failed to bind 127.0.0.1:{port}"))
}

pub(super) fn bind_first_available_loopback_port(start_port: u16) -> Result<TcpListener> {
    for port in start_port..=u16::MAX {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => return Ok(listener),
            Err(error) if error.kind() == ErrorKind::AddrInUse => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("failed to bind 127.0.0.1:{port}"))
            }
        }
    }
    bail!("failed to bind a loopback port from {start_port} through 65535")
}

/// Serve an already-bound listener until `running` becomes false.
///
/// `base` scopes the work directory the snapshots are read from, but *not* the
/// daemon connection: `daemon_client` resolves the socket and the `user.token`
/// through `commands::common::work_dir_path`, which is relative to the
/// process's current directory. [`execute`] passes `"."`, so the two agree
/// there; a caller passing some other directory - a test's tempdir, say - gets
/// file snapshots from `base` and any daemon subscription from the CWD.
pub fn serve(
    listener: TcpListener,
    base: PathBuf,
    running: Arc<AtomicBool>,
    options: ServeOptions,
) -> Result<()> {
    serve_with(listener, base, running, options, limits::Limits::new())
}

fn serve_with(
    listener: TcpListener,
    base: PathBuf,
    running: Arc<AtomicBool>,
    options: ServeOptions,
    limits: Arc<limits::Limits>,
) -> Result<()> {
    listener.set_nonblocking(true)?;
    let lane = terminal_lane(&listener, &options)?;
    let broadcaster = broadcast::Broadcaster::spawn(base.clone(), running.clone(), lane.is_some());
    while running.load(Ordering::SeqCst) {
        accept_connection(&listener, &broadcaster, &base, &running, &limits, &lane);
    }
    drain_connections(&limits);
    Ok(())
}

fn accept_connection(
    listener: &TcpListener,
    broadcaster: &broadcast::Broadcaster,
    base: &std::path::Path,
    running: &Arc<AtomicBool>,
    limits: &Arc<limits::Limits>,
    lane: &Option<TerminalLane>,
) {
    match listener.accept() {
        Ok((stream, _)) => spawn_connection(stream, broadcaster, base, running, limits, lane),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            thread::sleep(Duration::from_millis(50));
        }
        Err(error) => tracing::warn!("dashboard accept failed: {error}"),
    }
}

fn spawn_connection(
    mut stream: std::net::TcpStream,
    broadcaster: &broadcast::Broadcaster,
    base: &std::path::Path,
    running: &Arc<AtomicBool>,
    limits: &Arc<limits::Limits>,
    lane: &Option<TerminalLane>,
) {
    if let Err(error) = stream.set_nonblocking(false) {
        tracing::warn!("dashboard could not configure a client socket: {error}");
        return;
    }
    let Some(slot) = limits::Slot::acquire(limits, limits::Lane::Connection) else {
        connection::reject_overloaded(&mut stream);
        return;
    };
    let broadcaster = broadcaster.clone();
    let base = base.to_path_buf();
    let running = running.clone();
    let limits = limits.clone();
    let lane = lane.clone();
    if let Err(error) = thread::Builder::new()
        .name("loom-dashboard-conn".to_owned())
        .spawn(move || {
            connection::handle(
                stream,
                &broadcaster,
                &base,
                &running,
                &limits,
                lane.as_ref(),
                slot,
            )
        })
    {
        tracing::warn!("dashboard could not spawn a connection thread: {error}");
    }
}

fn terminal_lane(listener: &TcpListener, options: &ServeOptions) -> Result<Option<TerminalLane>> {
    let port = listener.local_addr()?.port();
    Ok(options
        .terminal_token
        .as_ref()
        .map(|token| TerminalLane::from_token(token.clone(), port)))
}

fn drain_connections(limits: &limits::Limits) {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while limits.connection_count() != 0 && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
}
