//! Per-worktree spool for memory entries a sandboxed stage cannot write directly.
//!
//! `loom memory note` (and its siblings) normally write straight to
//! `.loom/work/memory/<stage>.md` via [`super::append_entry`]. Inside a
//! sandboxed worktree that write fails: `.loom/work` is a symlink to the main
//! repo's `.loom/work`, the generated sandbox grants
//! `Read(.loom/work/memory/**)` but no matching `Edit`, and the kernel
//! refuses the write with EROFS or `PermissionDenied`.
//!
//! This module gives the sandboxed path somewhere writable to land instead:
//! `<worktree_root>/.loom/memory-spool.jsonl`, which sits inside the
//! worktree's own write boundary and needs no new sandbox grant. The loom
//! daemon runs outside the sandbox, so on its poll loop it calls
//! [`drain_spool`] to move every pending entry into the real journal file
//! and empties the spool.
//!
//! Deliberately absent from the spool payload: a stage id. Attribution of a
//! drained entry comes from *which worktree* the daemon drained it from, not
//! from anything the entry claims about itself - a sandboxed agent cannot
//! forge the worktree it is running in, but it could trivially forge a
//! field. Keeping the payload stage-less makes that the only path.

use super::persistence::{validate_content, validate_evidence};
use super::storage::append_entry;
use super::types::MemoryEntry;
use anyhow::{Context, Result};
use fs2::FileExt;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Spool location relative to a worktree root.
pub const SPOOL_RELPATH: &str = ".loom/memory-spool.jsonl";

/// Refuse further appends past this size (bytes). Bounds a runaway agent
/// that keeps recording while the daemon (for whatever reason) isn't
/// draining - without this a stuck spool grows without limit.
pub const SPOOL_MAX_BYTES: u64 = 1024 * 1024;

/// What a drain pass accomplished.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrainOutcome {
    pub drained: usize,
    pub skipped_malformed: usize,
}

/// Absolute path of a worktree's spool file.
pub fn spool_path(worktree_root: &Path) -> PathBuf {
    worktree_root.join(SPOOL_RELPATH)
}

/// Append one entry as a single JSON line, creating `.loom/` if needed.
///
/// Holds an exclusive lock for the whole read-size/write operation:
/// `O_APPEND` alone only guarantees atomicity up to the platform's atomic
/// write size (commonly 4096 bytes), and a `MemoryEntry` with a 2000-char
/// content plus a 2000-char context can exceed that, so two concurrent
/// appends could otherwise interleave their bytes on one line.
pub fn append_to_spool(worktree_root: &Path, entry: &MemoryEntry) -> Result<()> {
    let path = spool_path(worktree_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create spool directory: {}", parent.display()))?;
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("Failed to open memory spool: {}", path.display()))?;
    file.lock_exclusive()
        .with_context(|| format!("Failed to lock memory spool: {}", path.display()))?;

    let size = file
        .metadata()
        .with_context(|| format!("Failed to stat memory spool: {}", path.display()))?
        .len();
    if size >= SPOOL_MAX_BYTES {
        anyhow::bail!(
            "Memory spool {} has reached its {SPOOL_MAX_BYTES}-byte cap; \
             the loom daemon has not drained it yet",
            path.display()
        );
    }

    // `to_string` (not `to_string_pretty`) so the payload is guaranteed to
    // be a single line - serde_json escapes any newline inside content/context.
    let line = serde_json::to_string(entry).context("Failed to serialize memory entry")?;
    writeln!(file, "{line}")
        .with_context(|| format!("Failed to append to memory spool: {}", path.display()))?;

    Ok(())
}

/// Pending entries, WITHOUT removing them. Empty vec when no spool exists.
///
/// Missing-spool is the overwhelmingly common case (every daemon tick, for
/// every stage that has spooled nothing) so this must stay cheap and must
/// never create the file or the `.loom/` directory.
pub fn read_pending(worktree_root: &Path) -> Result<Vec<MemoryEntry>> {
    let path = spool_path(worktree_root);
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = fs::File::open(&path)
        .with_context(|| format!("Failed to open memory spool: {}", path.display()))?;
    file.lock_shared()
        .with_context(|| format!("Failed to lock memory spool: {}", path.display()))?;

    let mut contents = String::new();
    (&file)
        .read_to_string(&mut contents)
        .with_context(|| format!("Failed to read memory spool: {}", path.display()))?;

    Ok(parse_entries(&contents).0)
}

/// Hand every pending entry to `sink`, then truncate the spool.
///
/// Delivery is at-least-once: entries are only removed after `sink` has
/// returned `Ok` for every one of them and the file is truncated, all under
/// one exclusive lock. If any `sink` call errors, the error propagates and
/// the file is left untouched - including entries `sink` already accepted -
/// so the whole batch redelivers on the next pass. A crash between the last
/// successful `sink` call and the truncate has the same effect. That is an
/// accepted tradeoff: a duplicated memory entry is harmless, a lost one
/// isn't recoverable.
///
/// Malformed lines are skipped (counted, not retried) rather than blocking
/// the entries around them; they are discarded on the truncate that follows
/// a successful pass, since a line that couldn't parse this time never will.
pub fn drain_spool(
    worktree_root: &Path,
    sink: &mut dyn FnMut(&MemoryEntry) -> Result<()>,
) -> Result<DrainOutcome> {
    let path = spool_path(worktree_root);
    if !path.exists() {
        return Ok(DrainOutcome::default());
    }

    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("Failed to open memory spool: {}", path.display()))?;
    file.lock_exclusive()
        .with_context(|| format!("Failed to lock memory spool: {}", path.display()))?;

    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .with_context(|| format!("Failed to read memory spool: {}", path.display()))?;

    let (entries, skipped_malformed) = parse_entries(&contents);
    for entry in &entries {
        sink(entry)?;
    }

    file.set_len(0)
        .with_context(|| format!("Failed to truncate memory spool: {}", path.display()))?;

    Ok(DrainOutcome {
        drained: entries.len(),
        skipped_malformed,
    })
}

/// Drain a worktree's spool into the stage's canonical journal.
///
/// The one sink shared by both drain callers - the daemon's per-tick pass
/// (`orchestrator/core/spool_drain.rs`) and the final drain a caller
/// runs right before the worktree is removed. Both need identical
/// validation and attribution behavior, so this is the single implementation
/// they call instead of each carrying its own copy.
///
/// Re-validates `entry.content` (and `entry.context`, if present) with
/// [`validate_content`] before writing - defense in depth, since the spool is
/// agent-written and the CLI already validated on the way in. An entry that
/// fails validation is skipped rather than propagated as an error: returning
/// `Err` here would stop [`drain_spool`] from truncating the file, so one
/// poison entry would redeliver forever, wedged in front of every good entry
/// behind it. `Err` is reserved for a genuine I/O failure from
/// [`append_entry`], where blocking the truncate and retrying later is the
/// correct behavior.
///
/// [`DrainOutcome::drained`] counts only entries actually written to the
/// journal. [`DrainOutcome::skipped_malformed`] covers both unparseable
/// lines (caught by [`drain_spool`] before the sink ever sees them) and
/// entries this sink rejected on validation - both are malformed in the same
/// sense to a caller reporting on the drain.
pub fn drain_into_journal(
    work_dir: &Path,
    stage_id: &str,
    worktree_root: &Path,
) -> Result<DrainOutcome> {
    let mut skipped_invalid = 0usize;
    let outcome = drain_spool(worktree_root, &mut |entry| {
        sink_into_journal(work_dir, stage_id, entry, &mut skipped_invalid)
    })?;
    Ok(DrainOutcome {
        drained: outcome.drained.saturating_sub(skipped_invalid),
        skipped_malformed: outcome.skipped_malformed + skipped_invalid,
    })
}

/// Validate then append one spooled entry to the canonical journal, or skip
/// it (never `Err`) when validation fails. See [`drain_into_journal`] for why.
fn sink_into_journal(
    work_dir: &Path,
    stage_id: &str,
    entry: &MemoryEntry,
    skipped_invalid: &mut usize,
) -> Result<()> {
    if let Err(e) = validate_spooled_entry(entry) {
        *skipped_invalid += 1;
        tracing::debug!(
            stage_id = %stage_id,
            error = %e,
            "Discarding spooled memory entry that failed validation"
        );
        return Ok(());
    }
    append_entry(work_dir, stage_id, entry)
}

/// Validate a spooled entry's content, and its context if present.
fn validate_spooled_entry(entry: &MemoryEntry) -> Result<()> {
    validate_content(&entry.content)?;
    if let Some(context) = &entry.context {
        validate_content(context)?;
    }
    validate_evidence(&entry.evidence)?;
    Ok(())
}

/// Parse spool contents into (successfully-parsed entries, malformed-line count).
/// Blank lines are ignored entirely - not counted as malformed.
fn parse_entries(contents: &str) -> (Vec<MemoryEntry>, usize) {
    let mut entries = Vec::new();
    let mut skipped = 0;
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<MemoryEntry>(line) {
            Ok(entry) => entries.push(entry),
            Err(_) => skipped += 1,
        }
    }
    (entries, skipped)
}

#[cfg(test)]
#[path = "tests/spool.rs"]
mod tests;
