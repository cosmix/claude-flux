//! Snapshot production from the daemon subscription or the local work files.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::commands::status::data::{collect_status_data, StatusData};
use crate::commands::status::ui::tui::daemon_client;
use crate::daemon::{read_message, Response};
use crate::fs::work_dir::WorkDir;

use super::model::{collect_snapshot, SnapshotSource, WebSnapshot};

const RECONNECT_ATTEMPTS: usize = 3;
const FILE_POLL_COUNT: usize = 5;
const FILE_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Frames a subscriber may fall behind by before it is disconnected.
///
/// Frames are produced at most every [`FILE_POLL_INTERVAL`] on the file lane,
/// and only on a genuine tree change on the daemon lane, so a client that has
/// not drained eight of them is wedged rather than merely busy — a suspended
/// tab whose TCP window has closed, say. Buffering more for it would pin every
/// queued snapshot for the life of the process; disconnecting costs it only a
/// reconnect, which the client already performs.
pub(super) const SUBSCRIBER_QUEUE_DEPTH: usize = 8;

/// Fans JSON snapshot frames out to all connected WebSocket clients.
#[derive(Clone)]
pub struct Broadcaster {
    inner: Arc<Inner>,
}

/// Lock order, taken by both [`Broadcaster::publish`] and
/// [`Broadcaster::subscribe`]: `last_body`, then `subscribers`, then `latest`.
/// Registering a subscriber reads `latest` while holding `subscribers`, so a
/// concurrent `publish` cannot slip a frame past a half-registered client —
/// which would otherwise strand it on a stale snapshot for as long as the tree
/// stayed unchanged, because `publish` suppresses unchanged frames.
struct Inner {
    latest: Mutex<Option<Arc<String>>>,
    last_body: Mutex<Option<String>>,
    subscribers: Mutex<Vec<mpsc::SyncSender<Arc<String>>>>,
    terminals: bool,
}

/// The result of interpreting one daemon response.
pub enum DaemonStep {
    Frame(String),
    Ignore,
    Degraded(String),
}

enum DaemonExit {
    Disconnected { received_frame: bool },
    StreamError { received_frame: bool },
    Degraded(String),
}

impl Broadcaster {
    /// Start the producer thread for `base`.
    pub fn spawn(base: PathBuf, running: Arc<AtomicBool>, terminals: bool) -> Self {
        let this = Self::with_terminals(terminals);
        let producer = this.clone();
        thread::spawn(move || run(producer, base, running, terminals));
        this
    }

    /// Queue the most recent frame, if any, for a newly-connected client.
    pub fn subscribe(&self) -> mpsc::Receiver<Arc<String>> {
        let (sender, receiver) = mpsc::sync_channel(SUBSCRIBER_QUEUE_DEPTH);
        let mut subscribers = self
            .inner
            .subscribers
            .lock()
            .expect("dashboard subscribers poisoned");
        if let Some(frame) = self.latest() {
            let _ = sender.try_send(frame);
        }
        subscribers.push(sender);
        receiver
    }

    /// Return the latest frame delivered to subscribers.
    pub fn latest(&self) -> Option<Arc<String>> {
        self.inner
            .latest
            .lock()
            .expect("dashboard latest frame poisoned")
            .clone()
    }

    #[cfg(test)]
    pub(super) fn new() -> Self {
        Self::with_terminals(false)
    }

    fn with_terminals(terminals: bool) -> Self {
        Self {
            inner: Arc::new(Inner {
                latest: Mutex::new(None),
                last_body: Mutex::new(None),
                subscribers: Mutex::new(Vec::new()),
                terminals,
            }),
        }
    }

    pub(super) fn terminals(&self) -> bool {
        self.inner.terminals
    }

    pub(super) fn publish(&self, json: String) {
        let body = body_without_timestamp(&json);
        let mut last_body = self
            .inner
            .last_body
            .lock()
            .expect("dashboard body poisoned");
        if last_body.as_deref() == Some(body.as_str()) {
            return;
        }
        *last_body = Some(body);

        let frame = Arc::new(json);
        let mut subscribers = self
            .inner
            .subscribers
            .lock()
            .expect("dashboard subscribers poisoned");
        *self
            .inner
            .latest
            .lock()
            .expect("dashboard latest frame poisoned") = Some(frame.clone());
        subscribers.retain(|sender| sender.try_send(frame.clone()).is_ok());
    }
}

/// Serialize a status update as the exact daemon frame body.
pub fn snapshot_frame(
    work_path: &Path,
    data: StatusData,
    source: SnapshotSource,
    terminals: bool,
) -> Result<String> {
    serde_json::to_string(&collect_snapshot(work_path, data, source, terminals))
        .context("serialize dashboard snapshot")
}

/// Classify a daemon response without opening a daemon socket.
pub fn classify_response(
    work_path: &Path,
    response: Response,
    terminals: bool,
) -> Result<DaemonStep> {
    match response {
        Response::StatusUpdate { data } => {
            snapshot_frame(work_path, *data, SnapshotSource::Daemon, terminals)
                .map(DaemonStep::Frame)
        }
        Response::Error { message } => Ok(DaemonStep::Degraded(message)),
        _ => Ok(DaemonStep::Ignore),
    }
}

/// Produce a fresh file-backed frame for an HTTP request with no cached frame.
pub(super) fn fresh_file_snapshot(base: &Path, terminals: bool) -> Result<String> {
    let work_dir = WorkDir::new(base)?;
    work_dir.load()?;
    let data = collect_status_data(&work_dir)?;
    snapshot_frame(work_dir.root(), data, SnapshotSource::Files, terminals)
}

fn run(this: Broadcaster, base: PathBuf, running: Arc<AtomicBool>, terminals: bool) {
    let Ok(work_dir) = WorkDir::new(&base).and_then(|work_dir| {
        work_dir.load()?;
        Ok(work_dir)
    }) else {
        tracing::warn!("dashboard could not load work directory");
        return;
    };
    let work_path = work_dir.root().to_path_buf();
    let mut failures = 0;
    while running.load(Ordering::SeqCst) {
        match daemon_session(&work_path) {
            Ok(stream) => match forward_daemon(&this, stream, &work_path, &running, terminals) {
                DaemonExit::Degraded(message) => {
                    poll_files(
                        &this,
                        &work_dir,
                        &work_path,
                        &running,
                        Some(&message),
                        terminals,
                    );
                    failures = 0;
                }
                DaemonExit::Disconnected { received_frame }
                | DaemonExit::StreamError { received_frame } => {
                    if received_frame {
                        failures = 0;
                    }
                    failures += 1;
                    sleep_while_running(&running, Duration::from_millis(500));
                    if failures >= RECONNECT_ATTEMPTS {
                        poll_files(&this, &work_dir, &work_path, &running, None, terminals);
                        failures = 0;
                    }
                }
            },
            Err(error) => {
                tracing::debug!("dashboard daemon subscription unavailable: {error}");
                poll_files(&this, &work_dir, &work_path, &running, None, terminals);
                failures = 0;
            }
        }
    }
}

fn daemon_session(work_path: &Path) -> Result<UnixStream> {
    let mut stream = daemon_client::connect(&work_path.join("orchestrator.sock"))?;
    daemon_client::subscribe(&mut stream)?;
    stream.set_read_timeout(Some(Duration::from_millis(50)))?;
    Ok(stream)
}

fn forward_daemon(
    broadcaster: &Broadcaster,
    mut stream: UnixStream,
    work_path: &Path,
    running: &AtomicBool,
    terminals: bool,
) -> DaemonExit {
    let mut received_frame = false;
    while running.load(Ordering::SeqCst) {
        match read_message::<Response, _>(&mut stream) {
            Ok(response) => match classify_response(work_path, response, terminals) {
                Ok(DaemonStep::Frame(frame)) => {
                    broadcaster.publish(frame);
                    received_frame = true;
                }
                Ok(DaemonStep::Ignore) => {}
                Ok(DaemonStep::Degraded(message)) => return DaemonExit::Degraded(message),
                Err(error) => tracing::warn!("dashboard could not serialize daemon frame: {error}"),
            },
            Err(error) if daemon_client::is_socket_disconnected(&error) => {
                return DaemonExit::Disconnected { received_frame };
            }
            Err(error) if is_read_timeout(&error) => {}
            Err(error) => {
                tracing::warn!("dashboard daemon stream error: {error}");
                return DaemonExit::StreamError { received_frame };
            }
        }
    }
    DaemonExit::Disconnected { received_frame }
}

fn poll_files(
    broadcaster: &Broadcaster,
    work_dir: &WorkDir,
    work_path: &Path,
    running: &AtomicBool,
    notice: Option<&str>,
    terminals: bool,
) {
    for _ in 0..FILE_POLL_COUNT {
        if !running.load(Ordering::SeqCst) {
            return;
        }
        if let Err(error) = poll_files_once(broadcaster, work_dir, work_path, notice, terminals) {
            tracing::warn!("dashboard file snapshot failed: {error}");
        }
        sleep_while_running(running, FILE_POLL_INTERVAL);
    }
}

pub(super) fn poll_files_once(
    broadcaster: &Broadcaster,
    work_dir: &WorkDir,
    work_path: &Path,
    notice: Option<&str>,
    terminals: bool,
) -> Result<()> {
    let data = collect_status_data(work_dir)?;
    let mut snapshot = collect_snapshot(work_path, data, SnapshotSource::Files, terminals);
    snapshot.notice = notice.map(str::to_owned);
    broadcaster.publish(serde_json::to_string(&snapshot).context("serialize file snapshot")?);
    Ok(())
}

fn body_without_timestamp(json: &str) -> String {
    let Ok(mut snapshot) = serde_json::from_str::<WebSnapshot>(json) else {
        return json.to_owned();
    };
    snapshot.generated_at = DateTime::<Utc>::UNIX_EPOCH;
    serde_json::to_string(&snapshot).unwrap_or_else(|_| json.to_owned())
}

fn sleep_while_running(running: &AtomicBool, duration: Duration) {
    let mut remaining = duration;
    while !remaining.is_zero() && running.load(Ordering::SeqCst) {
        let slice = remaining.min(Duration::from_millis(100));
        thread::sleep(slice);
        remaining = remaining.saturating_sub(slice);
    }
}

fn is_read_timeout(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<io::Error>().is_some_and(|io_error| {
            matches!(
                io_error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            )
        })
    })
}
