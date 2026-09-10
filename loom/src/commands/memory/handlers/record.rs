//! Recording handlers: `note`, `decision`, `change`, `question`.

use anyhow::{bail, Context, Result};
use colored::Colorize;
use std::path::Path;

use crate::daemon::DaemonServer;
use crate::fs::memory::{
    append_entry, append_to_spool, validate_content, validate_evidence, MemoryEntry,
    MemoryEntryType,
};
use crate::git::worktree::find_worktree_root_from_cwd;

use super::super::formatters::format_record_success;
use super::work_dir::{get_or_create_work_dir, validate_stage_id, AD_HOC_STAGE_ID};

/// Record a fully constructed entry through the shared direct/spool path.
pub(super) fn record(entry: MemoryEntry, stage_id: Option<String>) -> Result<()> {
    validate_content(&entry.content)?;
    if let Some(ref context) = entry.context {
        validate_content(context)?;
    }
    validate_evidence(&entry.evidence)?;
    reject_stage_forgery(&stage_id)?;

    let work_dir = get_or_create_work_dir()?;
    let stage = stage_id
        .or_else(|| std::env::var("LOOM_STAGE_ID").ok())
        .unwrap_or_else(|| AD_HOC_STAGE_ID.to_string());
    validate_stage_id(&stage)?;

    match append_entry(&work_dir, &stage, &entry) {
        Ok(()) => {}
        Err(error) if is_write_denied(&error) => {
            record_via_spool(error, &work_dir, &stage, &entry)?;
        }
        Err(error) => return Err(error),
    }

    if entry.entry_type != MemoryEntryType::Receipt {
        println!("{}", format_record_success(&entry, &stage));
    }
    Ok(())
}

/// Construct entries for the four ordinary recording commands.
fn record_kind(
    entry_type: MemoryEntryType,
    text: String,
    context: Option<String>,
    evidence: Vec<String>,
    stage_id: Option<String>,
) -> Result<()> {
    let entry = match context {
        Some(context) => MemoryEntry::with_context(entry_type, text, context),
        None => MemoryEntry::new(entry_type, text),
    }
    .with_evidence(evidence);
    record(entry, stage_id)
}

/// Refuse an explicit `--stage` that disagrees with `LOOM_STAGE_ID`.
///
/// Attribution must never be spoofable via a CLI flag, on EITHER write
/// path: this used to be checked only inside the spool fallback, so a
/// stage that can write the state directory directly (e.g. a main-repo knowledge stage,
/// which isn't sandboxed and never hits `is_write_denied`) could still pass
/// `--stage <someone-else>` and land an entry in another stage's journal.
/// Checking here, before either write is attempted, closes that regardless
/// of which path a given call ends up taking.
///
/// When `LOOM_STAGE_ID` is unset there is no session identity to forge -
/// an ad-hoc/interactive/operator session with no active loom stage - so
/// `--stage` remains a legitimate, freely usable affordance and this is a
/// no-op.
fn reject_stage_forgery(stage_id: &Option<String>) -> Result<()> {
    let (Some(id), Ok(env_stage)) = (stage_id, std::env::var("LOOM_STAGE_ID")) else {
        return Ok(());
    };
    if id != &env_stage {
        bail!(
            "a stage records only to its own memory journal (this session is stage '{env_stage}'); \
             --stage '{id}' does not match. Entry was NOT recorded."
        );
    }
    Ok(())
}

/// True when `error` (or something in its cause chain) is a filesystem
/// permission failure - what a sandboxed worktree sees when it tries to
/// write through the state directory symlink to the main repo, since the
/// generated sandbox grants read access to the state directory's `memory/**`
/// but no matching write.
fn is_write_denied(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io_err| {
                io_err.kind() == std::io::ErrorKind::PermissionDenied
                    || io_err.raw_os_error() == Some(libc::EROFS)
            })
    })
}

/// Fall back to the per-worktree spool (see `fs::memory::spool`) after a
/// direct write to `<state-dir>/memory/<stage>.md` was denied.
///
/// `original_err` is propagated unchanged whenever spooling isn't a valid
/// option for this call, so the caller never loses the real diagnostic.
/// `stage` has already cleared [`reject_stage_forgery`] by the time this
/// runs, so it does not need its own copy of that check.
fn record_via_spool(
    original_err: anyhow::Error,
    work_dir: &Path,
    stage: &str,
    entry: &MemoryEntry,
) -> Result<()> {
    let cwd = std::env::current_dir().context("Failed to get current directory")?;

    // Spooling only makes sense inside a worktree - that's the one place the
    // daemon knows to look for a pending entry to attribute to a stage. Off
    // a worktree, spooling would strand the entry with no owner, so surface
    // the original write failure instead.
    let Some(worktree_root) = find_worktree_root_from_cwd(&cwd) else {
        return Err(original_err);
    };

    append_to_spool(&worktree_root, entry)?;

    eprintln!(
        "{} Recorded to the stage spool; the loom daemon will commit it to the state \
         directory's memory/{stage}.md",
        "ℹ".blue()
    );
    if !DaemonServer::is_running(work_dir) {
        eprintln!(
            "{} No loom daemon is currently running; this entry stays pending until one drains the spool",
            "⚠".yellow()
        );
    }

    Ok(())
}

/// Record a note with evidence references in the memory journal.
pub fn note(text: String, evidence: Vec<String>, stage_id: Option<String>) -> Result<()> {
    record_kind(MemoryEntryType::Note, text, None, evidence, stage_id)
}

/// Record a decision in the memory journal
pub fn decision(
    text: String,
    context: Option<String>,
    evidence: Vec<String>,
    stage_id: Option<String>,
) -> Result<()> {
    record_kind(MemoryEntryType::Decision, text, context, evidence, stage_id)
}

/// Record a file change in the memory journal
pub fn change(text: String, evidence: Vec<String>, stage_id: Option<String>) -> Result<()> {
    record_kind(MemoryEntryType::Change, text, None, evidence, stage_id)
}

/// Record a question in the memory journal
pub fn question(text: String, evidence: Vec<String>, stage_id: Option<String>) -> Result<()> {
    record_kind(MemoryEntryType::Question, text, None, evidence, stage_id)
}
