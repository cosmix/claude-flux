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

mod assets;
mod broadcast;
mod connection;
mod head;
mod http;
mod limits;
pub mod model;
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

use crate::fs::work_dir::WorkDir;

/// First port considered by `loom status --web` without a value.
pub const DEFAULT_PORT: u16 = 7373;

/// Start the dashboard server on loopback until Ctrl-C.
pub fn execute(port: Option<u16>) -> Result<()> {
    let work_dir = WorkDir::new(".")?;
    work_dir.load()?;
    let listener = bind_listener(port)?;
    let actual_port = listener.local_addr()?.port();
    println!("loom dashboard: http://127.0.0.1:{actual_port}/  (Ctrl-C to stop)");
    if assets::WEB_ASSETS.is_empty() {
        eprintln!(
            "warning: dashboard assets are not embedded in this binary; run `cd web && bun install && bun run build`, then rebuild loom"
        );
    }

    let running = Arc::new(AtomicBool::new(true));
    let on_ctrl_c = running.clone();
    ctrlc::set_handler(move || on_ctrl_c.store(false, Ordering::SeqCst))
        .context("failed to install Ctrl-C handler")?;
    serve(listener, PathBuf::from("."), running)
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
pub fn serve(listener: TcpListener, base: PathBuf, running: Arc<AtomicBool>) -> Result<()> {
    listener.set_nonblocking(true)?;
    let broadcaster = broadcast::Broadcaster::spawn(base.clone(), running.clone());
    let limits = limits::Limits::new();
    while running.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                if let Err(error) = stream.set_nonblocking(false) {
                    tracing::warn!("dashboard could not configure a client socket: {error}");
                    continue;
                }
                let Some(slot) = limits::Slot::acquire(&limits, limits::Lane::Connection) else {
                    connection::reject_overloaded(&mut stream);
                    continue;
                };
                let broadcaster = broadcaster.clone();
                let base = base.clone();
                let running = running.clone();
                let limits = limits.clone();
                // One thread per connection, and the OS may refuse it; dropping
                // that one client beats panicking the accept loop out from under
                // every other.
                if let Err(error) = thread::Builder::new()
                    .name("loom-dashboard-conn".to_owned())
                    .spawn(move || {
                        connection::handle(stream, &broadcaster, &base, &running, &limits, slot)
                    })
                {
                    tracing::warn!("dashboard could not spawn a connection thread: {error}");
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => tracing::warn!("dashboard accept failed: {error}"),
        }
    }
    Ok(())
}
