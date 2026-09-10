//! Process utilities for loom
//!
//! This module provides common process management functions used across the codebase.

mod environment;
mod identity;
#[doc(hidden)]
pub mod sandbox_probe;

use anyhow::{Context, Result};
use nix::errno::Errno;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::io::Read;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use wait_timeout::ChildExt;

pub use environment::apply_stage_environment;
pub use identity::{
    process_is_zombie, process_start_time, terminate_verified, verify_process_identity,
    IdentityStatus, ProcessIdentity, UnverifiedProcessIdentity,
};

/// Structured wall-clock timeout returned by bounded command helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessTimeoutError {
    operation: String,
    timeout: Duration,
}

impl ProcessTimeoutError {
    pub fn new(operation: impl Into<String>, timeout: Duration) -> Self {
        Self {
            operation: operation.into(),
            timeout,
        }
    }

    pub fn operation(&self) -> &str {
        &self.operation
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

impl std::fmt::Display for ProcessTimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} timed out after {:?}", self.operation, self.timeout)
    }
}

impl std::error::Error for ProcessTimeoutError {}

/// Outcome of a subprocess run under a wall-clock bound.
#[derive(Debug)]
pub enum BoundedOutput {
    /// The child exited on its own before the deadline.
    Completed(Output),
    /// The deadline elapsed first; the child was killed and reaped.
    TimedOut,
}

impl BoundedOutput {
    /// The child's output, or `None` if it was killed at the deadline.
    pub fn completed(self) -> Option<Output> {
        match self {
            BoundedOutput::Completed(output) => Some(output),
            BoundedOutput::TimedOut => None,
        }
    }
}

/// Run a command to completion under a wall-clock bound, killing it if the
/// deadline elapses.
///
/// # Why this exists
///
/// The orchestrator's poll loop is single-threaded: it syncs stage state,
/// spawns ready stages, and handles session teardown in one sequence. A
/// subprocess that never returns therefore does not merely fail one
/// operation — it stops *all* orchestration, silently, with no further log
/// output, while the daemon's socket thread keeps answering `loom status`
/// as if everything were healthy.
///
/// Window management is the dangerous case. On macOS `osascript` sends an
/// Apple Event and blocks with no timeout of its own: a TCC Automation
/// prompt (which a detached daemon cannot surface), a terminal-side modal,
/// or an unresponsive terminal app all park the call indefinitely. Linux is
/// less exposed — the `wmctrl`/`xdotool` paths are `which`-guarded and no-op
/// when the tools are absent — but an unresponsive X server can stall them
/// the same way.
///
/// Every external command issued from the orchestrator loop must therefore be
/// bounded. A timed-out teardown is a warning; an unbounded one is a hang.
///
/// # Output size
///
/// stdout/stderr are drained concurrently on dedicated threads while the main
/// thread waits on the child, so output larger than the OS pipe buffer
/// (~64KB) cannot make the child block on write and stall past the deadline.
///
/// # Deadline scope
///
/// `timeout` bounds the *whole call*, not just the child's run time: once the
/// child exits, collecting its buffered stdout/stderr from the reader
/// threads still has to happen before this function can return, and a
/// descendant that inherited a pipe and outlived the direct child can hold
/// that collection open indefinitely. The remaining time budget (`timeout`
/// minus time already spent) bounds that collection too; running past it is
/// reported the same way as the child itself running past the deadline.
pub fn run_bounded(command: &mut Command, timeout: Duration) -> Result<BoundedOutput> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let started = Instant::now();
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Failed to spawn {:?}", command.get_program()))?;

    let (stdout_reader, stderr_reader) = start_readers(&mut child, command)?;

    match child
        .wait_timeout(timeout)
        .with_context(|| format!("Failed to wait for {:?}", command.get_program()))?
    {
        Some(status) => {
            let remaining = timeout.saturating_sub(started.elapsed());
            collect_output(
                &mut child,
                status,
                stdout_reader,
                stderr_reader,
                remaining,
                command,
            )
        }
        None => {
            kill_group(&mut child);
            let _ = child.wait();
            Ok(BoundedOutput::TimedOut)
        }
    }
}

/// A reader thread's eventual result, or `None` if its pipe was never
/// attached (see [`drain`]).
type ReaderHandle = Option<Receiver<std::io::Result<Vec<u8>>>>;

/// Grace period given to the reader threads to unblock once the process
/// group has been killed after a post-exit deadline was already exceeded.
const READER_GRACE: Duration = Duration::from_secs(1);

/// Kill the whole process group of `child` so a control command cannot leave
/// descendants behind, whether the deadline expired while it was still
/// running or a reader thread failed to start after it did.
fn kill_group(child: &mut Child) {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(child.id()) {
        let _ = kill(Pid::from_raw(-pid), Signal::SIGKILL);
    }
    #[cfg(not(unix))]
    let _ = child.kill();
}

/// Start the stdout/stderr reader threads for an already-spawned `child`.
///
/// If the OS refuses to create a reader thread (`RLIMIT_NPROC`, memory
/// pressure), the child is already running with nothing left to drain its
/// pipes: kill its process group before returning `Err` so the failure
/// cannot leave an orphaned, unbounded process behind.
fn start_readers(child: &mut Child, command: &Command) -> Result<(ReaderHandle, ReaderHandle)> {
    let stdout = match drain(child.stdout.take()) {
        Ok(reader) => reader,
        Err(error) => return Err(reader_spawn_failed(child, "stdout", error, command)),
    };
    let stderr = match drain(child.stderr.take()) {
        Ok(reader) => reader,
        Err(error) => return Err(reader_spawn_failed(child, "stderr", error, command)),
    };
    Ok((stdout, stderr))
}

fn reader_spawn_failed(
    child: &mut Child,
    stream: &str,
    error: std::io::Error,
    command: &Command,
) -> anyhow::Error {
    kill_group(child);
    let _ = child.wait();
    anyhow::Error::new(error).context(format!(
        "Failed to start {stream} reader for {:?}",
        command.get_program()
    ))
}

/// Spawn a named thread that reads `pipe` to completion and sends the result
/// down a channel, or return `Ok(None)` if the pipe was never attached.
/// `Err` only when the OS refuses to create the thread.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> std::io::Result<ReaderHandle> {
    let Some(mut pipe) = pipe else {
        return Ok(None);
    };
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("loom-process-reader".to_string())
        .spawn(move || {
            let mut buf = Vec::new();
            let result = pipe.read_to_end(&mut buf).map(|_| buf);
            let _ = tx.send(result);
        })?;
    Ok(Some(rx))
}

/// Receive both readers' output within `remaining`. If either hasn't
/// delivered by then, the direct child has already exited but a descendant
/// still holds a pipe open: the caller's deadline was exceeded while output
/// was held open, so this reports [`BoundedOutput::TimedOut`] regardless of
/// whether the descendant, once the process group is killed, still manages
/// to unblock the reader. [`READER_GRACE`] only bounds that cleanup attempt
/// so the reader threads exit promptly instead of lingering past return.
fn collect_output(
    child: &mut Child,
    status: ExitStatus,
    stdout_reader: ReaderHandle,
    stderr_reader: ReaderHandle,
    remaining: Duration,
    command: &Command,
) -> Result<BoundedOutput> {
    let stdout = recv_stream(&stdout_reader, remaining);
    let stderr = recv_stream(&stderr_reader, remaining);
    if let (Some(stdout), Some(stderr)) = (stdout, stderr) {
        return finish_output(status, stdout, stderr, command).map(BoundedOutput::Completed);
    }
    kill_group(child);
    let _ = child.wait();
    let _ = recv_stream(&stdout_reader, READER_GRACE);
    let _ = recv_stream(&stderr_reader, READER_GRACE);
    Ok(BoundedOutput::TimedOut)
}

/// Receive one reader's result within `deadline`, or `None` if it hasn't
/// delivered yet. A missing `receiver` (pipe never attached) resolves
/// immediately to empty output, matching the pre-channel `drain` behavior.
fn recv_stream(receiver: &ReaderHandle, deadline: Duration) -> Option<std::io::Result<Vec<u8>>> {
    let Some(receiver) = receiver else {
        return Some(Ok(Vec::new()));
    };
    match receiver.recv_timeout(deadline) {
        Ok(result) => Some(result),
        Err(mpsc::RecvTimeoutError::Timeout) => None,
        Err(mpsc::RecvTimeoutError::Disconnected) => Some(Err(std::io::Error::other(
            "reader thread exited without sending a result (likely panicked)",
        ))),
    }
}

/// Turn the two receive results into `Output`, converting a reader I/O error
/// into a contextual error naming `command`'s program and stream.
fn finish_output(
    status: ExitStatus,
    stdout: std::io::Result<Vec<u8>>,
    stderr: std::io::Result<Vec<u8>>,
    command: &Command,
) -> Result<Output> {
    let stdout =
        stdout.with_context(|| format!("Failed to read stdout of {:?}", command.get_program()))?;
    let stderr =
        stderr.with_context(|| format!("Failed to read stderr of {:?}", command.get_program()))?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// Run a command under a deadline and turn expiry into a typed error.
pub fn run_bounded_output(
    command: &mut Command,
    timeout: Duration,
    operation: impl Into<String>,
) -> Result<Output> {
    let operation = operation.into();
    match run_bounded(command, timeout)? {
        BoundedOutput::Completed(output) => Ok(output),
        BoundedOutput::TimedOut => Err(ProcessTimeoutError::new(operation, timeout).into()),
    }
}

/// Send `SIGTERM` to a process.
///
/// Returns `Ok(true)` when the signal was delivered, `Ok(false)` when the
/// process no longer exists (`ESRCH` — already gone, which is a success for
/// every caller here), and `Err` for any other failure.
///
/// Prefer this over shelling out to `kill(1)`: it cannot block, and it avoids
/// a fork+exec on a path that runs for every session teardown.
pub fn terminate(pid: u32) -> Result<bool> {
    let pid_i32 = i32::try_from(pid).with_context(|| format!("PID {pid} exceeds i32::MAX"))?;

    match kill(Pid::from_raw(pid_i32), Signal::SIGTERM) {
        Ok(()) => Ok(true),
        Err(Errno::ESRCH) => Ok(false),
        Err(e) => Err(anyhow::anyhow!("Failed to signal process {pid}: {e}")),
    }
}

/// Check if a process with the given PID is alive
///
/// Uses `nix::sys::signal::kill` with signal `None` (null signal / signal 0) to check
/// process existence. This properly distinguishes between:
/// - Process exists and we can signal it (`Ok(())`)
/// - Process exists but we lack permission (`EPERM`)
/// - Process does not exist (`ESRCH`)
///
/// # PID Reuse Warning
///
/// **RACE CONDITION HAZARD**: This function is subject to PID reuse races. Between the time
/// you check if a PID is alive and the time you act on that information, the process may
/// have exited and the OS may have reassigned that PID to a new, unrelated process.
///
/// This means `is_process_alive(pid) == true` only tells you "a process with this PID exists
/// right now", NOT "the process I'm tracking is still running". The new process may be
/// completely unrelated to the one you're monitoring.
///
/// **Safe usage patterns:**
/// - Only use this for informational purposes (logging, diagnostics)
/// - DO NOT use this as the sole basis for sending signals or making state transitions
/// - Prefer tracking processes via process handles or file locks when possible
/// - If you must use PIDs, combine with additional identity checks (start time, parent PID, etc.)
///
/// # Arguments
/// * `pid` - The process ID to check
///
/// # Returns
/// * `true` - The process exists (regardless of signal permission)
/// * `false` - The process doesn't exist or the PID is invalid
///
/// # Example
/// ```ignore
/// use loom::process::is_process_alive;
///
/// let our_pid = std::process::id();
/// assert!(is_process_alive(our_pid));
///
/// // Non-existent PID
/// assert!(!is_process_alive(999999999));
/// ```
pub fn is_process_alive(pid: u32) -> bool {
    let pid_i32 = match i32::try_from(pid) {
        Ok(v) => v,
        Err(_) => {
            // PID exceeds i32::MAX, treat as non-existent
            return false;
        }
    };

    // Send null signal (signal 0) to check process existence without
    // actually delivering a signal. The kernel returns different errors
    // depending on whether the process exists vs. permission denied.
    let exists = match kill(Pid::from_raw(pid_i32), None) {
        Ok(()) => true,             // Process exists and we can signal it
        Err(Errno::EPERM) => true,  // Process exists but we lack permission
        Err(Errno::ESRCH) => false, // No such process
        Err(_) => false,            // Other error, treat as non-existent
    };

    // A zombie still answers the null signal: the PID entry survives until the
    // parent reaps it. It cannot run, so it is not alive for any question loom
    // asks (crash detection, session liveness, daemon singleton ownership).
    exists && !identity::process_is_zombie(pid)
}

#[cfg(test)]
mod tests;
