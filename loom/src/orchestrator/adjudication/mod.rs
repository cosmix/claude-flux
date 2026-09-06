//! Adjudication subsystem.
//!
//! Disputes filed by agents land in `.loom/work/disputes/<stage>/<n>/request.md`.
//! The orchestrator polls these files every tick:
//!
//! * [`AdjudicatorRegistry::disputes_awaiting_session`] returns the disputes
//!   that need an adjudication session started, having applied every guard
//!   (a verdict already written, a stage that has left `NeedsAdjudication`,
//!   the evidence-round cap, a session already live for the stage, the
//!   spawn-attempt cap) and escalated the stage where a guard is terminal.
//!   The daemon spawns the session; see `orchestrator/core/orchestrator.rs`.
//! * [`AdjudicatorRegistry::apply_pending_verdicts`] scans for verdict
//!   files that haven't been applied (no `applied.marker`) and mutates
//!   stage state accordingly (see `apply.rs`).
//!
//! The adjudicator is a real loom session, not a subprocess the daemon waits
//! on: it is spawned into a terminal in the MAIN REPOSITORY, judges the
//! dispute with the full tool surface, and records its verdict by running
//! `loom stage adjudicate`. The daemon never blocks on it — it observes
//! `verdict.md` appearing on a later tick, exactly as merge resolution
//! observes `loom stage merge --resolved`.
//!
//! The registry therefore holds no state at all: liveness comes from the
//! session record and the spawn budget from the dispute directory, so a
//! daemon restart mid-adjudication neither loses a running session nor
//! resets its budget.

mod apply;
pub mod feedback;
mod plan_patch;
pub mod prompt;
mod scan;
pub mod session;
pub mod verdict;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_verdicts;

use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::models::dispute::{dispute_dir, request_file, verdict_file};
use crate::models::stage::StageStatus;
use crate::verify::transitions::{load_stage, update_stage};

use scan::{read_dispute_request, scan_pending_requests};

pub use session::{
    attempt_count, persist_verdict, read_request, resolve_model, verdict_draft_file,
    AdjudicationJob, DEFAULT_ADJUDICATION_MODEL, MAX_ADJUDICATION_ATTEMPTS,
};

/// Maximum evidence-loop rounds. After this, the stage escalates to
/// `NeedsHumanReview` instead of looping forever.
///
/// Five, not three: each round is a real exchange — the adjudicator asks
/// specific questions, the stage agent answers them in its next attempt — and
/// three cut that conversation off while it was still converging. The loop is
/// bounded because an adjudicator that cannot decide after five rounds of its
/// own questions is not going to decide on the sixth, not because rounds are
/// expensive.
pub const MAX_EVIDENCE_ROUNDS: u32 = 5;

/// Apply attempts a single dispute's verdict may fail before the daemon gives
/// up and escalates instead of retrying forever.
///
/// A verdict that still cannot apply after this many ticks is a wedge, not a
/// transient failure a retry would clear — a malformed `plan_patch`, a
/// corrupt `verdict.md`, or a stage file `load_stage` cannot parse all fail
/// identically on every attempt. Left uncapped, every failed attempt also
/// re-triggers `retire_disputing_agents` before the next one (see
/// `orchestrator/core/verdict_apply.rs`), which kills any new session on the
/// stage within seconds of it starting.
pub const MAX_APPLY_ATTEMPTS: u32 = 3;

/// The daemon's entry points into the dispute lifecycle.
///
/// Deliberately stateless — see the module docs. It is owned by the
/// [`Orchestrator`](crate::orchestrator::Orchestrator) and lives for the
/// entire daemon run.
#[derive(Debug, Default, Clone, Copy)]
pub struct AdjudicatorRegistry;

impl AdjudicatorRegistry {
    pub fn new() -> Self {
        Self
    }

    /// Disputes that need an adjudication session started on this tick.
    ///
    /// Every returned job has already been counted against the dispute's
    /// spawn budget: the caller is expected to try the spawn immediately, and
    /// [`AdjudicatorRegistry::start_pending_adjudications`] escalates the
    /// stage when it cannot.
    pub fn disputes_awaiting_session(&self, work_dir: &Path) -> Result<Vec<AdjudicationJob>> {
        let disputes_root = work_dir.join("disputes");
        if !disputes_root.exists() {
            return Ok(Vec::new());
        }
        let mut jobs: Vec<AdjudicationJob> = Vec::new();
        for (stage_id, dispute_id) in scan_pending_requests(&disputes_root)? {
            // One adjudicator per stage per pass. A stage can carry more than
            // one unanswered dispute (an earlier one abandoned when the stage
            // was escalated and then resumed), and the live-session guard
            // below only sees sessions started on an EARLIER tick — so
            // without this, both would be handed a session at once and two
            // adjudicators would judge the same stage in the same repository.
            if jobs.iter().any(|job| job.stage.id == stage_id) {
                continue;
            }
            if let Some(job) = self.job_for_dispute(work_dir, &stage_id, dispute_id) {
                jobs.push(job);
            }
        }
        Ok(jobs)
    }

    /// The guards, in order, for one pending dispute. `None` means "not this
    /// tick" — either because the dispute is already handled, or because a
    /// terminal condition escalated the stage instead.
    fn job_for_dispute(
        &self,
        work_dir: &Path,
        stage_id: &str,
        dispute_id: u32,
    ) -> Option<AdjudicationJob> {
        let disputes_root = work_dir.join("disputes");
        if verdict_file(&disputes_root, stage_id, dispute_id).exists() {
            return None;
        }
        let request = read_dispute_request(&request_file(&disputes_root, stage_id, dispute_id))
            .map_err(|error| {
                tracing::warn!(target: "loom::adjudication", stage = %stage_id, dispute = dispute_id, %error, "skipping unparseable dispute request");
            })
            .ok()?;

        let stage = load_stage(stage_id, work_dir)
            .map_err(|error| {
                tracing::warn!(target: "loom::adjudication", stage = %stage_id, %error, "could not load stage; skipping dispute");
            })
            .ok()?;
        if stage.status != StageStatus::NeedsAdjudication {
            return None;
        }
        if stage.evidence_rounds >= MAX_EVIDENCE_ROUNDS {
            escalate_evidence_cap(work_dir, stage_id);
            return None;
        }
        if !self.claim_session_slot(work_dir, stage_id, dispute_id) {
            return None;
        }

        Some(AdjudicationJob {
            stage,
            request,
            plan_path: resolve_plan_path(work_dir).unwrap_or_else(|| PathBuf::from("PLAN.md")),
        })
    }
}

impl AdjudicatorRegistry {
    /// Whether a new adjudication session may be started for this dispute,
    /// spending one of its attempts if so.
    ///
    /// `false` means either that a session is already judging the stage — the
    /// reason not to start a second one, and one that holds across daemon
    /// restarts because it is answered from the session record — or that the
    /// dispute has used its budget, in which case the stage is escalated so it
    /// does not wait on a session that will never come.
    fn claim_session_slot(&self, work_dir: &Path, stage_id: &str, dispute_id: u32) -> bool {
        if let Some(session_id) = session::live_adjudication_session(work_dir, stage_id) {
            tracing::debug!(target: "loom::adjudication", stage = %stage_id, session = %session_id, "adjudication session already live");
            return false;
        }
        let attempts = session::attempt_count(work_dir, stage_id, dispute_id);
        if attempts >= MAX_ADJUDICATION_ATTEMPTS {
            escalate_attempt_cap(work_dir, stage_id, dispute_id, attempts);
            return false;
        }
        session::record_attempt(work_dir, stage_id, dispute_id);
        true
    }
}

/// Escalate a stage whose adjudication session could not be started at all.
///
/// A spawn failure is an environment problem (no terminal, no `claude` on
/// PATH, a backend that refuses), not something a retry fixes, and a dispute
/// that hangs silently is worse than one that asks for a human.
fn escalate_adjudicator_unavailable(work_dir: &Path, stage_id: &str, error: &str) {
    escalate(
        work_dir,
        stage_id,
        format!("No adjudication session could be started for this dispute: {error}"),
    );
}

fn escalate_evidence_cap(work_dir: &Path, stage_id: &str) {
    escalate(
        work_dir,
        stage_id,
        format!("Evidence loop exhausted at {MAX_EVIDENCE_ROUNDS} rounds"),
    );
}

fn escalate_attempt_cap(work_dir: &Path, stage_id: &str, dispute_id: u32, attempts: u32) {
    escalate(
        work_dir,
        stage_id,
        format!(
            "Adjudication of dispute {dispute_id} produced no verdict after {attempts} session(s)"
        ),
    );
}

/// Escalate a dispute whose verdict has now failed to apply
/// [`MAX_APPLY_ATTEMPTS`] times. The operator's only other clue is a log
/// line, so the reason carries the apply error text itself.
fn escalate_apply_cap(
    work_dir: &Path,
    stage_id: &str,
    dispute_id: u32,
    failures: u32,
    error: &anyhow::Error,
) {
    escalate(
        work_dir,
        stage_id,
        format!(
            "Applying the verdict for dispute {dispute_id} failed {failures} time(s): {error:#}"
        ),
    );
}

/// File name for the per-dispute apply-failure counter, mirroring
/// `session.rs`'s `attempts` file for adjudication spawn attempts.
const APPLY_FAILURES_FILENAME: &str = "apply_failures";

fn apply_failures_file(work_dir: &Path, stage_id: &str, dispute_id: u32) -> PathBuf {
    dispute_dir(&work_dir.join("disputes"), stage_id, dispute_id).join(APPLY_FAILURES_FILENAME)
}

/// How many times applying this dispute's verdict has already failed.
fn apply_failure_count(work_dir: &Path, stage_id: &str, dispute_id: u32) -> u32 {
    std::fs::read_to_string(apply_failures_file(work_dir, stage_id, dispute_id))
        .ok()
        .and_then(|raw| raw.trim().parse::<u32>().ok())
        .unwrap_or(0)
}

/// Count one more apply failure and return the new total.
///
/// Best-effort like [`session::record_attempt`]: a directory that cannot be
/// created or written is warned about, never fatal — the caller still gets a
/// count to compare against [`MAX_APPLY_ATTEMPTS`] even when it could not be
/// persisted.
fn record_apply_failure(work_dir: &Path, stage_id: &str, dispute_id: u32) -> u32 {
    let path = apply_failures_file(work_dir, stage_id, dispute_id);
    let next = apply_failure_count(work_dir, stage_id, dispute_id).saturating_add(1);
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            tracing::warn!(
                target: "loom::adjudication",
                stage = %stage_id,
                dispute = dispute_id,
                %error,
                "could not create the dispute directory; apply failures are not being counted",
            );
            return next;
        }
    }
    if let Err(error) = std::fs::write(&path, next.to_string()) {
        tracing::warn!(
            target: "loom::adjudication",
            stage = %stage_id,
            dispute = dispute_id,
            %error,
            "could not persist the apply failure count",
        );
    }
    next
}

/// Single locked read-modify-write: re-apply only the human-review transition
/// onto the fresh on-disk stage (A-5). Best effort — a refused transition is
/// logged inside the closure and ignored.
fn escalate(work_dir: &Path, stage_id: &str, reason: String) {
    let result = update_stage(stage_id, work_dir, |s| {
        s.try_request_human_review(reason.clone()).ok();
        Ok(())
    });
    if let Err(error) = result {
        tracing::warn!(
            target: "loom::adjudication",
            stage = %stage_id,
            %error,
            "failed to escalate stage to NeedsHumanReview",
        );
    }
}

fn resolve_plan_path(work_dir: &Path) -> Option<PathBuf> {
    let cfg = crate::fs::work_dir::load_config(work_dir).ok().flatten()?;
    let path = cfg.source_path()?;
    if path.is_absolute() {
        Some(path)
    } else {
        let root = work_dir
            .canonicalize()
            .ok()
            .and_then(|wd| wd.parent().map(|p| p.to_path_buf()))?;
        Some(root.join(path))
    }
}
