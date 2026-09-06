//! Applying a written verdict to the stage it judged.
//!
//! The daemon scans for `verdict.md` files with no sibling `applied.marker`
//! and acts on each one exactly once. Which way a verdict moves the stage is
//! the whole autonomy contract:
//!
//! * `Accept` — the criterion was wrong. Amend the plan, clear the feedback,
//!   then re-queue the stage, or hold it in `NeedsAdjudication` while other
//!   disputes on it are unanswered.
//! * `NeedsMoreEvidence` — ask the agent the adjudicator's questions, then
//!   re-queue (or hold, per the same rule) up to
//!   [`MAX_EVIDENCE_ROUNDS`](super::MAX_EVIDENCE_ROUNDS).
//! * `Reject` — the criterion was right and the implementation is wrong.
//!   This is a DEADLOCK, not a retry: the agent already judged the criterion
//!   impossible and the adjudicator has now upheld it, so re-queueing would
//!   loop the same disagreement. It escalates to `NeedsHumanReview` — the one
//!   outcome of adjudication that is designed to need a human.

use anyhow::{Context, Result};
use std::path::Path;

use crate::models::dispute::{applied_marker, dispute_dir, DisputeVerdict, DisputeVerdictRecord};
use crate::models::stage::{Stage, StageStatus};
use crate::plan::amendment::apply_amendment;
use crate::verify::transitions::{load_stage, update_stage};

use super::plan_patch::build_amendment_request;
use super::scan::read_verdict_record;
use super::{feedback, resolve_plan_path, AdjudicatorRegistry, MAX_EVIDENCE_ROUNDS};

impl AdjudicatorRegistry {
    /// Apply verdict files that haven't been applied yet (no
    /// `applied.marker`). Idempotent under crash recovery: a `.applying`
    /// marker is written before mutating stage state and removed only
    /// after `applied.marker` is in place.
    pub fn apply_pending_verdicts(&self, work_dir: &Path) -> Result<()> {
        for (stage_id, dispute_id) in self.pending_verdicts(work_dir)? {
            if let Err(e) = self.apply_verdict(work_dir, &stage_id, dispute_id) {
                tracing::warn!(
                    target: "loom::adjudication",
                    stage = %stage_id,
                    dispute = dispute_id,
                    error = %e,
                    "failed to apply verdict",
                );
            }
        }
        Ok(())
    }

    /// Apply a single verdict to the stage. Public so tests can drive a
    /// verdict file end-to-end. Wraps `apply_verdict_once` with a
    /// bounded retry: after [`super::MAX_APPLY_ATTEMPTS`] failures the
    /// verdict is a wedge, not a transient, and gets escalated instead.
    pub fn apply_verdict(&self, work_dir: &Path, stage_id: &str, dispute_id: u32) -> Result<()> {
        match self.apply_verdict_once(work_dir, stage_id, dispute_id) {
            Ok(()) => Ok(()),
            Err(e) => {
                let failures = super::record_apply_failure(work_dir, stage_id, dispute_id);
                if failures >= super::MAX_APPLY_ATTEMPTS {
                    super::escalate_apply_cap(work_dir, stage_id, dispute_id, failures, &e);
                    // Write applied.marker so pending_verdicts stops
                    // returning this dispute: that is what stops both the
                    // retry loop AND the retire_disputing_agents kill that
                    // runs before every attempt.
                    write_applied_marker_after_apply_cap(work_dir, stage_id, dispute_id);
                    Ok(())
                } else {
                    Err(e)
                }
            }
        }
    }

    /// One apply attempt. Idempotent under crash recovery: a `.applying`
    /// marker guards the mutation and is removed once `applied.marker` lands.
    fn apply_verdict_once(&self, work_dir: &Path, stage_id: &str, dispute_id: u32) -> Result<()> {
        let disputes_root = work_dir.join("disputes");
        let verdict_path =
            crate::models::dispute::verdict_file(&disputes_root, stage_id, dispute_id);
        let applied = applied_marker(&disputes_root, stage_id, dispute_id);
        if applied.exists() {
            return Ok(());
        }
        let applying = dispute_dir(&disputes_root, stage_id, dispute_id).join(".applying");

        let record = read_verdict_record(&verdict_path)?;
        let mut stage = load_stage(stage_id, work_dir)?;

        // Write the .applying marker BEFORE mutating any state so a
        // crash mid-apply is recoverable (the next tick re-enters here
        // and re-applies the same verdict; the work is idempotent).
        if let Some(parent) = applying.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let _ = std::fs::write(&applying, b"");

        let result = self.apply_verdict_inner(work_dir, &mut stage, &record);
        let final_result = persist_verdict_result(work_dir, stage_id, &stage, &applied, result);
        // Always remove the .applying marker — success means the
        // applied.marker now exists, failure means we'll retry on
        // the next tick and re-write the marker.
        let _ = std::fs::remove_file(&applying);
        final_result
    }

    fn apply_verdict_inner(
        &self,
        work_dir: &Path,
        stage: &mut Stage,
        record: &DisputeVerdictRecord,
    ) -> Result<()> {
        match &record.verdict {
            DisputeVerdict::Accept { plan_patch, .. } => {
                apply_accept(work_dir, stage, plan_patch, record.id)
            }
            DisputeVerdict::Reject {
                reasoning,
                citations,
            } => apply_reject(work_dir, stage, record.id, reasoning, citations),
            DisputeVerdict::NeedsMoreEvidence { questions } => {
                apply_needs_more_evidence(work_dir, stage, questions)
            }
        }
    }
}

/// The persistence half of `apply_verdict`, split out because setup
/// (reading the verdict, taking the `.applying` lock) and persisting the
/// result are two different concerns sharing one function only because they
/// run in sequence.
///
/// Re-applies ONLY the verdict-owned fields onto the FRESH on-disk stage so a
/// concurrent dispute-thread write (e.g. dispute_count for a parallel filing)
/// or CLI write is not reverted (A-5). The verdict owns: status (+
/// review_reason via try_request_human_review), evidence_rounds,
/// amendments_applied, and acceptance/wiring (the latter two were already
/// written to disk by apply_amendment's own locked update; re-applying the
/// in-memory copy keeps them coherent under this lock).
///
/// The status write itself is skipped entirely when the on-disk status
/// already matches (the common "hold in NeedsAdjudication while a sibling
/// dispute is unanswered" no-op — logging a forced transition for a status
/// that never changed is pure noise), and goes through the validated
/// `try_transition` when the on-disk status legally allows it (e.g.
/// `NeedsAdjudication -> Queued`, which the transition table already knows
/// about). `force_status_with_reason` remains the fallback for the case the
/// on-disk status does NOT recognize the move — the in-memory `stage` already
/// holds the verdict's resolved status and the on-disk status is re-read
/// here, so re-validating unconditionally would risk a spurious refusal
/// against an intermediate on-disk state.
fn persist_verdict_result(
    work_dir: &Path,
    stage_id: &str,
    stage: &Stage,
    applied: &Path,
    inner_result: Result<()>,
) -> Result<()> {
    inner_result?;
    let verdict_status = stage.status.clone();
    let verdict_review_reason = stage.review_reason.clone();
    let verdict_evidence_rounds = stage.evidence_rounds;
    let verdict_amendments_applied = stage.amendments_applied;
    let verdict_acceptance = stage.acceptance.clone();
    let verdict_wiring = stage.wiring.clone();
    update_stage(stage_id, work_dir, |s| {
        if s.status != verdict_status {
            if s.status.can_transition_to(&verdict_status) {
                let _ = s.try_transition(verdict_status.clone());
            } else {
                s.force_status_with_reason(verdict_status.clone(), "adjudicator verdict applied");
            }
        }
        s.review_reason = verdict_review_reason.clone();
        s.evidence_rounds = verdict_evidence_rounds;
        s.amendments_applied = verdict_amendments_applied;
        s.acceptance = verdict_acceptance.clone();
        s.wiring = verdict_wiring.clone();
        Ok(())
    })
    .context("save amended stage after verdict apply")?;
    if let Some(parent) = applied.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(applied, b"")
        .with_context(|| format!("Failed to write {}", applied.display()))?;
    Ok(())
}

/// The criterion was wrong: amend the plan and re-queue the stage.
fn apply_accept(
    work_dir: &Path,
    stage: &mut Stage,
    plan_patch: &crate::models::dispute::PlanPatch,
    dispute_id: u32,
) -> Result<()> {
    let plan_path =
        resolve_plan_path(work_dir).ok_or_else(|| anyhow::anyhow!("plan source_path missing"))?;
    let request = build_amendment_request(stage.id.clone(), plan_patch, dispute_id)?;
    match apply_amendment(&plan_path, work_dir, request) {
        Ok(_) => {}
        Err(e) if crate::plan::amendment::is_amendment_cap_error(&e) => {
            escalate_amendment_cap(work_dir, stage, &e);
            return Ok(());
        }
        Err(e) => return Err(e).context("apply plan amendment from accept verdict"),
    }
    resync_after_amendment(work_dir, stage);
    // Accept verdict closes the evidence loop: clear feedback and re-queue
    // the stage so the agent can retry (unless another dispute is still
    // unanswered).
    let _ = feedback::clear_feedback(work_dir, &stage.id);
    requeue_or_hold_for_remaining_disputes(work_dir, stage)
}

/// Cap exceeded: a further accepted dispute would exceed the per-stage
/// amendment budget. Escalate to human review and let the caller still write
/// applied.marker so we don't loop this verdict forever on the next tick.
fn escalate_amendment_cap(work_dir: &Path, stage: &mut Stage, error: &anyhow::Error) {
    let reason = format!("{error:#}");
    if let Err(transition_err) = stage.try_request_human_review(reason) {
        tracing::warn!(
            target: "loom::adjudication",
            stage = %stage.id,
            error = %transition_err,
            "amendment cap exceeded but stage could not be escalated to NeedsHumanReview",
        );
    }
    let _ = feedback::clear_feedback(work_dir, &stage.id);
}

/// Resync the stage's acceptance/wiring from disk (the amendment also rewrites
/// the stage file).
fn resync_after_amendment(work_dir: &Path, stage: &mut Stage) {
    let Ok(reloaded) = load_stage(&stage.id, work_dir) else {
        return;
    };
    stage.acceptance = reloaded.acceptance;
    stage.wiring = reloaded.wiring;
    // Derive amendments_applied from the audit log (the source of truth used
    // by the cap check). Bumping the in-memory field by +1 here would
    // double-count on a crash-mid-apply retry: apply_amendment is idempotent
    // (returns the prior result), but the increment-on-reload would have
    // re-bumped each pass.
    stage.amendments_applied =
        crate::plan::amendment::count_amendments_for_stage(work_dir, &stage.id)
            .unwrap_or_else(|_| reloaded.amendments_applied.saturating_add(1));
}

/// The criterion stands and the implementation is wrong — the deadlock case.
fn apply_reject(
    work_dir: &Path,
    stage: &mut Stage,
    dispute_id: u32,
    reasoning: &str,
    citations: &[crate::models::dispute::Citation],
) -> Result<()> {
    // The reasoning and citations are what a human needs to arbitrate, so they
    // are still written for the record.
    feedback::append_rejection(work_dir, &stage.id, reasoning, citations)?;
    let verdict_path =
        crate::models::dispute::verdict_file(&work_dir.join("disputes"), &stage.id, dispute_id);
    let reason = format!(
        "Adjudicator upheld the disputed acceptance criterion (dispute {dispute_id}): \
         the criterion stands and the implementation is what must change. The stage \
         agent judged it impossible, so it cannot make progress unaided — read \
         {} — then decide with: loom stage human-review {}",
        verdict_path.display(),
        stage.id
    );
    // A Reject MUST leave the stage in NeedsHumanReview: re-queueing is the
    // loop this arm exists to break, and leaving it in NeedsAdjudication with
    // an applied verdict would strand it silently. A refused transition
    // (unexpected on-disk drift) is therefore forced rather than propagated.
    if let Err(error) = stage.try_request_human_review(reason.clone()) {
        tracing::warn!(
            target: "loom::adjudication",
            stage = %stage.id,
            status = %stage.status,
            %error,
            "reject verdict could not transition the stage to NeedsHumanReview; forcing it",
        );
        stage.force_status_with_reason(StageStatus::NeedsHumanReview, &reason);
        stage.review_reason = Some(reason);
    }
    Ok(())
}

/// Undecidable on the evidence supplied: ask the agent the questions, unless
/// the evidence loop is spent.
fn apply_needs_more_evidence(
    work_dir: &Path,
    stage: &mut Stage,
    questions: &[String],
) -> Result<()> {
    feedback::append_questions(work_dir, &stage.id, questions)?;
    stage.evidence_rounds = stage.evidence_rounds.saturating_add(1);
    if stage.evidence_rounds >= MAX_EVIDENCE_ROUNDS {
        let reason = format!(
            "Adjudicator evidence loop exhausted ({} rounds)",
            stage.evidence_rounds
        );
        stage.try_request_human_review(reason).ok();
        Ok(())
    } else {
        requeue_or_hold_for_remaining_disputes(work_dir, stage)
    }
}

fn transition_to_queued(stage: &mut Stage) -> Result<()> {
    let target = StageStatus::Queued;
    // try_transition refuses NeedsAdjudication → Queued unless it knows
    // about it (the foundations stage added that transition). If the stage
    // is somehow in NeedsAdjudication but not recognized as a valid
    // transition source, fall back to a direct assignment with a warning so
    // we don't refuse to apply a verdict because of unrelated state drift.
    //
    // Any OTHER status must be left untouched: it means a verdict on an
    // earlier, sibling dispute already moved the stage (e.g. a Reject to
    // NeedsHumanReview), and forcing Queued here would silently erase that
    // escalation.
    if stage.status.can_transition_to(&target) {
        stage.try_transition(target)?;
    } else if stage.status == StageStatus::NeedsAdjudication {
        tracing::warn!(
            target: "loom::adjudication",
            stage = %stage.id,
            status = %stage.status,
            "stage not recognized as transitionable to Queued from NeedsAdjudication; forcing it",
        );
        stage.status = target;
        stage.updated_at = chrono::Utc::now();
    } else {
        tracing::warn!(
            target: "loom::adjudication",
            stage = %stage.id,
            status = %stage.status,
            "refusing to force stage to Queued from a non-NeedsAdjudication status",
        );
    }
    Ok(())
}

/// Re-queue the stage, unless another dispute on it still has no verdict.
///
/// `job_for_dispute` only schedules a dispute whose stage is
/// `NeedsAdjudication`, so the stage must stay there until the LAST
/// unanswered dispute is judged; only that verdict re-queues it. The
/// dispute currently being applied already has its `verdict.md` on disk, so
/// `scan_pending_requests` does not count it among the remainder.
fn requeue_or_hold_for_remaining_disputes(work_dir: &Path, stage: &mut Stage) -> Result<()> {
    if stage.status == StageStatus::NeedsHumanReview {
        // A Reject verdict on a sibling dispute already escalated this stage;
        // a later Accept/NeedsMoreEvidence verdict must not re-queue over it.
        tracing::warn!(
            target: "loom::adjudication",
            stage = %stage.id,
            "stage already NeedsHumanReview; not re-queueing or holding for remaining disputes",
        );
        return Ok(());
    }
    let remaining = AdjudicatorRegistry::new().unanswered_disputes(work_dir, &stage.id)?;
    if remaining == 0 {
        return transition_to_queued(stage);
    }
    tracing::info!(
        target: "loom::adjudication",
        stage = %stage.id,
        remaining,
        "holding stage in NeedsAdjudication: unanswered disputes remain",
    );
    if stage.status != StageStatus::NeedsAdjudication {
        stage.force_status_with_reason(
            StageStatus::NeedsAdjudication,
            "unanswered disputes remain after a verdict",
        );
    }
    Ok(())
}

/// Write `applied.marker` after an apply-cap escalation so the daemon stops
/// retrying; best-effort, since a write failure here just means a retry.
fn write_applied_marker_after_apply_cap(work_dir: &Path, stage_id: &str, dispute_id: u32) {
    let applied = applied_marker(&work_dir.join("disputes"), stage_id, dispute_id);
    if let Some(parent) = applied.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    if let Err(error) = std::fs::write(&applied, b"") {
        tracing::warn!(
            target: "loom::adjudication",
            stage = %stage_id,
            dispute = dispute_id,
            %error,
            "failed to write applied.marker after apply-cap escalation; verdict will be retried",
        );
    }
}
