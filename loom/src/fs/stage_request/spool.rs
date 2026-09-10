//! Append/read/drain mechanics for the stage-request spool.
//!
//! Kept deliberately close to `fs/memory/spool.rs`: same relative-path shape,
//! same locking, same size bound, same "a malformed line is skipped, never an
//! error" rule. A reader of one should recognise the other.

use anyhow::{Context, Result};
use fs2::FileExt;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::types::StageRequest;
use crate::daemon::MAX_REQUEST_BYTES;
use crate::fs::safe_read::open_regular_no_follow;
use crate::git::worktree::find_worktree_root_from_cwd;

/// Spool location relative to a worktree root.
pub const SPOOL_RELPATH: &str = ".loom/stage-request-spool.jsonl";

/// Refuse further appends past this size (bytes). Bounds a runaway agent that
/// keeps queueing while the daemon (for whatever reason) isn't draining -
/// without this a stuck spool grows without limit. Same bound as the memory
/// spool, for the same reason.
pub const SPOOL_MAX_BYTES: u64 = 1024 * 1024;

/// What a drain pass accomplished.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrainOutcome {
    /// Requests the stage actually took.
    pub applied: usize,
    /// Lines that could not be parsed, plus requests the daemon refused.
    /// Both are dead to the caller in the same way: nothing changed, and
    /// retrying the same bytes would not change that.
    pub skipped: usize,
}

/// Absolute path of a worktree's spool file.
pub fn spool_path(worktree_root: &Path) -> PathBuf {
    worktree_root.join(SPOOL_RELPATH)
}

/// The worktree whose spool this process should queue into.
///
/// Spooling only makes sense inside a worktree - that is the one place the
/// daemon knows to look, and the only thing it attributes a drained request
/// by. Off a worktree there is nowhere meaningful to write, so this is an
/// error naming why rather than a spool file nothing will ever read. Resolved
/// through the same [`find_worktree_root_from_cwd`] the memory CLI's spool
/// fallback uses, so both answer the question identically.
pub fn spool_target_from_cwd() -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("Failed to get current directory")?;
    find_worktree_root_from_cwd(&cwd).ok_or_else(|| {
        anyhow::anyhow!(
            "Cannot reach the loom daemon over .loom/work/orchestrator.sock, and this is not a \
             stage worktree, so there is nowhere to queue the request. Run this from the \
             stage's worktree, or from a shell that can reach the daemon."
        )
    })
}

/// Append one request as a single JSON line, creating `.loom/` if needed.
///
/// Holds an exclusive lock for the whole read-size/write operation: `O_APPEND`
/// alone only guarantees atomicity up to the platform's atomic write size
/// (commonly 4096 bytes), and a dispute carrying 4 KiB of failure output plus a
/// reason exceeds that, so two concurrent appends could otherwise interleave
/// their bytes on one line.
pub fn append_to_spool(worktree_root: &Path, request: &StageRequest) -> Result<()> {
    // `to_string` (not `to_string_pretty`) so the payload is guaranteed to be
    // a single line - serde_json escapes any newline inside a reason.
    let line = serde_json::to_string(request).context("Failed to serialize stage request")?;
    // Parity with the socket path, which refuses a request frame over
    // MAX_REQUEST_BYTES: without this the spool would accept what a live
    // daemon would have rejected, making the fallback the weaker gate.
    if line.len() > MAX_REQUEST_BYTES {
        anyhow::bail!(
            "Stage request is {} bytes, over the {MAX_REQUEST_BYTES}-byte limit the daemon \
             accepts; shorten the reason or the failure output",
            line.len()
        );
    }

    let path = spool_path(worktree_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create spool directory: {}", parent.display()))?;
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("Failed to open stage request spool: {}", path.display()))?;
    file.lock_exclusive()
        .with_context(|| format!("Failed to lock stage request spool: {}", path.display()))?;

    let size = file
        .metadata()
        .with_context(|| format!("Failed to stat stage request spool: {}", path.display()))?
        .len();
    if size >= SPOOL_MAX_BYTES {
        anyhow::bail!(
            "Stage request spool {} has reached its {SPOOL_MAX_BYTES}-byte cap; \
             the loom daemon has not drained it yet",
            path.display()
        );
    }

    writeln!(file, "{line}").with_context(|| {
        format!(
            "Failed to append to stage request spool: {}",
            path.display()
        )
    })?;

    Ok(())
}

/// Pending requests, WITHOUT applying or removing them. Empty vec when no
/// spool exists.
///
/// Missing-spool is the overwhelmingly common case (every daemon tick, for
/// every stage that has queued nothing) so this must stay cheap and must never
/// create the file or the `.loom/` directory. Exercised only by tests -
/// production code learns about spooled requests solely through
/// [`drain_spool`].
#[cfg(test)]
pub(crate) fn read_pending(worktree_root: &Path) -> Result<Vec<StageRequest>> {
    let path = spool_path(worktree_root);
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = fs::File::open(&path)
        .with_context(|| format!("Failed to open stage request spool: {}", path.display()))?;
    file.lock_shared()
        .with_context(|| format!("Failed to lock stage request spool: {}", path.display()))?;

    let mut contents = String::new();
    (&file)
        .read_to_string(&mut contents)
        .with_context(|| format!("Failed to read stage request spool: {}", path.display()))?;

    Ok(parse_requests(&contents).0)
}

/// Hand every pending request to `sink`, then truncate the spool.
///
/// Delivery is at-least-once: requests are only removed after `sink` has
/// returned `Ok` for every one of them and the file is truncated, all under one
/// exclusive lock. If any `sink` call errors, the error propagates and the file
/// is left untouched - including requests `sink` already applied - so the whole
/// batch redelivers on the next pass. A crash between the last successful
/// `sink` call and the truncate has the same effect. That is why the sink
/// treats a REFUSAL as success (see [`super::apply::drain_requests`]): a stage
/// the daemon declines to block is answered, not failed, and redelivering it
/// forever would wedge every request behind it.
///
/// Malformed lines are skipped (counted, not retried) rather than blocking the
/// requests around them; they are discarded on the truncate that follows a
/// successful pass, since a line that couldn't parse this time never will.
///
/// A spool the session replaced with a symlink (at the spool or its `.loom/`
/// directory), a hard link, or a FIFO is refused with an `Err` and left
/// untouched (see `open_regular_no_follow` in `fs::safe_read`), so the
/// trusted daemon never reads or truncates a file outside the worktree.
pub(super) fn drain_spool(
    worktree_root: &Path,
    sink: &mut dyn FnMut(&StageRequest) -> Result<()>,
) -> Result<DrainOutcome> {
    let path = spool_path(worktree_root);
    let opened = open_regular_no_follow(worktree_root, SPOOL_RELPATH, libc::O_RDWR)
        .with_context(|| format!("Stage request spool {} was not drained", path.display()))?;
    let Some(mut file) = opened else {
        return Ok(DrainOutcome::default());
    };
    file.lock_exclusive()
        .with_context(|| format!("Failed to lock stage request spool: {}", path.display()))?;

    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .with_context(|| format!("Failed to read stage request spool: {}", path.display()))?;

    let (requests, skipped) = parse_requests(&contents);
    for request in &requests {
        sink(request)?;
    }

    file.set_len(0)
        .with_context(|| format!("Failed to truncate stage request spool: {}", path.display()))?;

    // `applied` counts what the sink was handed; the sink knows how many of
    // those the daemon actually took, and adjusts both numbers on the way out.
    Ok(DrainOutcome {
        applied: requests.len(),
        skipped,
    })
}

/// Parse spool contents into (successfully-parsed requests, malformed-line
/// count). Blank lines are ignored entirely - not counted as malformed.
fn parse_requests(contents: &str) -> (Vec<StageRequest>, usize) {
    let mut requests = Vec::new();
    let mut skipped = 0;
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<StageRequest>(line) {
            Ok(request) => requests.push(request),
            Err(_) => skipped += 1,
        }
    }
    (requests, skipped)
}

#[cfg(test)]
#[path = "tests/spool.rs"]
mod tests;
