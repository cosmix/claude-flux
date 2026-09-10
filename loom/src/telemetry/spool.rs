//! Per-worktree fallback spool for telemetry emitted by sandboxed sessions.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use fs2::FileExt;

use super::{append_record, TelemetryRecord};
use crate::fs::safe_read::open_regular_no_follow;

/// Telemetry spool location relative to a worktree root.
pub const TELEMETRY_SPOOL_RELPATH: &str = ".loom/telemetry-spool.jsonl";

const SPOOL_MAX_BYTES: u64 = 1024 * 1024;

/// What a telemetry drain pass accomplished.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrainOutcome {
    pub drained: usize,
    pub skipped_malformed: usize,
}

/// Absolute path of a worktree's telemetry spool.
pub fn spool_path(worktree_root: &Path) -> PathBuf {
    worktree_root.join(TELEMETRY_SPOOL_RELPATH)
}

/// Append one timestamped record under an exclusive lock.
pub fn append_to_spool(worktree_root: &Path, record: &TelemetryRecord) -> Result<()> {
    let path = spool_path(worktree_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create telemetry spool directory: {}",
                parent.display()
            )
        })?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("Failed to open telemetry spool: {}", path.display()))?;
    file.lock_exclusive()
        .with_context(|| format!("Failed to lock telemetry spool: {}", path.display()))?;
    let size = file
        .metadata()
        .with_context(|| format!("Failed to stat telemetry spool: {}", path.display()))?
        .len();
    if size >= SPOOL_MAX_BYTES {
        anyhow::bail!(
            "Telemetry spool {} has reached its {SPOOL_MAX_BYTES}-byte cap; the loom daemon has not drained it yet",
            path.display()
        );
    }
    let line = serde_json::to_string(record).context("Failed to serialize telemetry record")?;
    writeln!(file, "{line}")
        .with_context(|| format!("Failed to append to telemetry spool: {}", path.display()))?;
    Ok(())
}

/// Read pending well-formed records without removing them. Exercised only by
/// tests - production code learns about spooled telemetry solely through
/// [`drain_into_events`].
#[cfg(test)]
pub(crate) fn read_pending(worktree_root: &Path) -> Result<Vec<TelemetryRecord>> {
    let path = spool_path(worktree_root);
    let opened = open_regular_no_follow(worktree_root, TELEMETRY_SPOOL_RELPATH, libc::O_RDONLY)
        .with_context(|| format!("Failed to open telemetry spool: {}", path.display()))?;
    let Some(file) = opened else {
        return Ok(Vec::new());
    };
    file.lock_shared()
        .with_context(|| format!("Failed to lock telemetry spool: {}", path.display()))?;
    let mut contents = String::new();
    (&file)
        .read_to_string(&mut contents)
        .with_context(|| format!("Failed to read telemetry spool: {}", path.display()))?;
    Ok(parse_records(&contents).0)
}

/// Drain every valid record into the canonical event file, then truncate the
/// spool while still holding its exclusive lock. Malformed lines are counted
/// and discarded. A spool the session replaced with a symlink (at the spool or
/// its `.loom/` directory), a hard link, or a FIFO is refused with an `Err` and
/// left untouched, so the trusted daemon never reads or truncates a file
/// outside the worktree.
pub fn drain_into_events(work_dir: &Path, worktree_root: &Path) -> Result<DrainOutcome> {
    let path = spool_path(worktree_root);
    let opened = open_regular_no_follow(worktree_root, TELEMETRY_SPOOL_RELPATH, libc::O_RDWR)
        .with_context(|| format!("Telemetry spool {} was not drained", path.display()))?;
    let Some(mut file) = opened else {
        return Ok(DrainOutcome::default());
    };
    file.lock_exclusive()
        .with_context(|| format!("Failed to lock telemetry spool: {}", path.display()))?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .with_context(|| format!("Failed to read telemetry spool: {}", path.display()))?;
    let (records, skipped_malformed) = parse_records(&contents);
    for record in &records {
        append_record(work_dir, record)?;
    }
    file.set_len(0)
        .with_context(|| format!("Failed to truncate telemetry spool: {}", path.display()))?;
    Ok(DrainOutcome {
        drained: records.len(),
        skipped_malformed,
    })
}

pub(super) fn is_write_denied(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io_err| {
                io_err.kind() == std::io::ErrorKind::PermissionDenied
                    || io_err.raw_os_error() == Some(libc::EROFS)
            })
    })
}

fn parse_records(contents: &str) -> (Vec<TelemetryRecord>, usize) {
    let mut records = Vec::new();
    let mut skipped = 0;
    for line in contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        match serde_json::from_str(line) {
            Ok(record) => records.push(record),
            Err(_) => skipped += 1,
        }
    }
    (records, skipped)
}
