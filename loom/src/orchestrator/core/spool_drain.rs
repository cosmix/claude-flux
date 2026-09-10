//! Drains the spools a sandboxed worktree writes when it cannot reach the
//! daemon: memory entries into the canonical journal, telemetry into its
//! event log, and stage-control requests (`loom stage block`, `loom stage
//! dispute-criteria`) into the daemon's own handlers.
//!
//! `loom memory note` writes directly to `.loom/work/memory/<stage>.md` via
//! [`crate::fs::memory::append_entry`]. Inside a sandboxed worktree that
//! write is refused (`.loom/work` is a symlink out of the worktree; the sandbox
//! grants no write there), so the CLI falls back to appending the entry to
//! `<worktree>/.loom/memory-spool.jsonl` instead — see [`crate::fs::memory`]
//! for the full spool design. This module is the daemon side: on every poll
//! tick it drains every stage's pending spool into the real journal.
//!
//! Attribution is derived ONLY from which worktree a spool file was found
//! in, never from the spool payload itself (which deliberately carries no
//! stage id). Memory files are injected into OTHER stages' prompts by
//! `orchestrator/signals/generate.rs`, so mis-attributing an entry would be a
//! prompt-injection channel between stages. The stage-request spool
//! (`crate::fs::stage_request`) is attributed the same way and for a sharper
//! reason still: a misattributed request would block or dispute the wrong
//! stage.

use std::path::Path;

use crate::fs::memory::{self, DrainOutcome};
use crate::fs::stage_request;
use crate::models::worktree::Worktree;
use crate::telemetry::spool::{self as telemetry_spool, DrainOutcome as TelemetryDrainOutcome};

use super::persistence::Persistence;
use super::Orchestrator;

impl Orchestrator {
    /// Drain every stage's pending spools: memory entries into the canonical
    /// journal, telemetry into its event log, and queued control requests into
    /// the daemon's handlers.
    ///
    /// Stages are enumerated by a disk scan of `.loom/work/stages/`, mirroring
    /// `spawn_merge_resolution_sessions` — disk is the source of truth and
    /// this survives daemon restarts, unlike `active_worktrees` (never
    /// repopulated for stages recovered as still-Executing) or
    /// `active_sessions` (misses recovered stages).
    ///
    /// Deliberately NOT filtered by stage status: `loom stage complete` is
    /// the last act of a stage session, so entries recorded moments before
    /// completion are still pending when the stage leaves `Executing`. A
    /// status filter would strand exactly the most valuable notes.
    pub(super) fn drain_stage_spools(&mut self) {
        let stages_dir = self.config.work_dir.join("stages");
        let entries = match std::fs::read_dir(&stages_dir) {
            Ok(entries) => entries,
            Err(_) => return, // no stages directory yet - nothing to drain
        };

        let stage_ids: Vec<String> = entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("md") {
                    return None;
                }
                let filename = path.file_name().and_then(|s| s.to_str())?;
                crate::fs::stage_files::extract_stage_id(filename)
            })
            .collect();

        for stage_id in stage_ids {
            // Enumerating the stage file already validates the id maps to a
            // real stage, which is the property we actually need; a failed
            // load just means a transiently unreadable/corrupt file, not a
            // reason to skip draining forever, so try again next tick.
            if self.load_stage(&stage_id).is_err() {
                continue;
            }
            let worktree_root = Worktree::worktree_path(&self.config.repo_root, &stage_id);
            // Memory first: an agent's last notes are usually recorded just
            // before the block it then queues, and a queued block can change
            // the stage's status out from under a later pass.
            self.drain_one_stage_spool(&stage_id, &worktree_root);
            self.drain_one_stage_telemetry(&stage_id, &worktree_root);
            self.drain_one_stage_requests(&stage_id, &worktree_root);
        }
    }

    /// Drain one stage's spool (if any) into its canonical journal.
    ///
    /// The validation and attribution logic itself lives in
    /// [`memory::drain_into_journal`], shared with the final pre-teardown
    /// drain in `git::cleanup::batch::cleanup_after_merge`; this method adds
    /// only the daemon-tick logging and error bookkeeping on top.
    ///
    /// Cheap no-op when the stage has no spool file: `memory::drain_into_journal`
    /// returns `DrainOutcome::default()` without touching disk, so calling
    /// this unconditionally for every stage on every tick is fine.
    pub(super) fn drain_one_stage_spool(
        &mut self,
        stage_id: &str,
        worktree_root: &Path,
    ) -> DrainOutcome {
        let work_dir = self.config.work_dir.clone();
        let result = memory::drain_into_journal(&work_dir, stage_id, worktree_root);

        self.record_drain_result(stage_id, worktree_root, result)
    }

    /// Log and account for the outcome of one stage's memory drain pass.
    fn record_drain_result(
        &mut self,
        stage_id: &str,
        worktree_root: &Path,
        result: anyhow::Result<DrainOutcome>,
    ) -> DrainOutcome {
        match result {
            Ok(outcome) => {
                // Silence when nothing was pending - this runs every 5
                // seconds for every stage, and the common case is an
                // absent spool file.
                if outcome.drained > 0 || outcome.skipped_malformed > 0 {
                    tracing::info!(
                        stage_id = %stage_id,
                        appended = outcome.drained,
                        skipped_malformed = outcome.skipped_malformed,
                        "Drained memory spool"
                    );
                }
                self.clear_drain_error(MEMORY_SPOOL, stage_id);
                outcome
            }
            Err(e) => {
                self.report_drain_error(MEMORY_SPOOL, stage_id, worktree_root, &e);
                DrainOutcome::default()
            }
        }
    }

    /// Drain one stage's telemetry spool into the canonical event log.
    pub(super) fn drain_one_stage_telemetry(
        &mut self,
        stage_id: &str,
        worktree_root: &Path,
    ) -> TelemetryDrainOutcome {
        let work_dir = self.config.work_dir.clone();
        let result = telemetry_spool::drain_into_events(&work_dir, worktree_root);
        self.record_telemetry_drain_result(stage_id, worktree_root, result)
    }

    fn record_telemetry_drain_result(
        &mut self,
        stage_id: &str,
        worktree_root: &Path,
        result: anyhow::Result<TelemetryDrainOutcome>,
    ) -> TelemetryDrainOutcome {
        match result {
            Ok(outcome) => {
                if outcome.drained > 0 || outcome.skipped_malformed > 0 {
                    tracing::info!(
                        stage_id = %stage_id,
                        appended = outcome.drained,
                        skipped_malformed = outcome.skipped_malformed,
                        "Drained telemetry spool"
                    );
                }
                self.clear_drain_error(TELEMETRY_SPOOL, stage_id);
                outcome
            }
            Err(e) => {
                self.report_drain_error(TELEMETRY_SPOOL, stage_id, worktree_root, &e);
                TelemetryDrainOutcome::default()
            }
        }
    }

    /// Apply one stage's spooled control requests through the daemon handlers
    /// that own them (see [`stage_request::drain_requests`]).
    ///
    /// Failure isolation matches the memory drain: one stage's unreadable or
    /// unappliable spool leaves every other stage's drain untouched, and its
    /// own requests stay pending for the next tick.
    ///
    /// Cheap no-op when the stage has no spool file: `drain_requests` returns
    /// `DrainOutcome::default()` without touching disk, so calling this
    /// unconditionally for every stage on every tick is fine.
    pub(super) fn drain_one_stage_requests(
        &mut self,
        stage_id: &str,
        worktree_root: &Path,
    ) -> stage_request::DrainOutcome {
        let work_dir = self.config.work_dir.clone();
        match stage_request::drain_requests(&work_dir, stage_id, worktree_root) {
            Ok(outcome) => {
                // Louder than the memory drain's aggregate, because a stage
                // that changes status from a spool has no session attached to
                // account for it; `fs::stage_request::apply` logs the detail.
                if outcome.applied > 0 || outcome.skipped > 0 {
                    tracing::info!(
                        stage_id = %stage_id,
                        applied = outcome.applied,
                        skipped = outcome.skipped,
                        "Applied spooled stage requests"
                    );
                }
                self.clear_drain_error(REQUEST_SPOOL, stage_id);
                outcome
            }
            Err(e) => {
                self.report_drain_error(REQUEST_SPOOL, stage_id, worktree_root, &e);
                stage_request::DrainOutcome::default()
            }
        }
    }

    /// Forget a stage's remembered drain failure for `kind`, so a later
    /// recurrence is reported again - mirroring `verified_merged`'s
    /// invalidate-on-change pattern.
    fn clear_drain_error(&mut self, kind: &str, stage_id: &str) {
        self.spool_drain_error_logged
            .remove(&drain_error_key(kind, stage_id));
    }

    /// Warn about a drain failure once per (stage, spool kind), so a stuck
    /// spool - a permissions problem under `.loom/work/memory`, a stage file that
    /// won't load - doesn't flood the logs on every 5-second poll.
    fn report_drain_error(
        &mut self,
        kind: &str,
        stage_id: &str,
        worktree_root: &Path,
        error: &anyhow::Error,
    ) {
        if self
            .spool_drain_error_logged
            .insert(drain_error_key(kind, stage_id))
        {
            tracing::warn!(
                stage_id = %stage_id,
                spool = kind,
                worktree_root = %worktree_root.display(),
                error = %error,
                "Failed to drain spool; entries remain pending and will retry next tick"
            );
        }
    }
}

/// Spool kinds, for the log-once bookkeeping below.
const MEMORY_SPOOL: &str = "memory";
const TELEMETRY_SPOOL: &str = "telemetry";
const REQUEST_SPOOL: &str = "stage-request";

/// Key under which a drain failure is remembered. Namespaced by spool kind so
/// the spools a single stage can have are tracked independently in the one
/// `spool_drain_error_logged` set - a stuck memory spool must not silence the
/// first report of a stuck request spool.
fn drain_error_key(kind: &str, stage_id: &str) -> String {
    format!("{kind}:{stage_id}")
}

#[cfg(test)]
#[path = "spool_drain_tests.rs"]
mod tests;
