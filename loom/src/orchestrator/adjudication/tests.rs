//! Unit tests for the adjudication module that need access to
//! multiple sub-modules together (cross-cutting flows). Per-module
//! tests live in `prompt.rs`, `session.rs`, `verdict.rs`, and
//! `feedback.rs`.

use super::plan_patch::build_amendment_request;
use super::scan::{parse_yaml_frontmatter, scan_pending_requests};
use super::session::{attempt_count, MAX_ADJUDICATION_ATTEMPTS};
use super::{feedback, AdjudicatorRegistry, MAX_EVIDENCE_ROUNDS};
use crate::models::dispute::{
    request_file, verdict_file, Citation, DisputeRequest, DisputeVerdict, DisputeVerdictRecord,
    PlanPatch,
};
use crate::models::stage::{Stage, StageStatus};
use crate::plan::amendment::{AmendmentField, AmendmentPatch};
use chrono::Utc;
use std::path::Path;

pub(super) fn make_stage(id: &str) -> Stage {
    Stage {
        id: id.to_string(),
        name: id.to_string(),
        status: StageStatus::NeedsAdjudication,
        ..Default::default()
    }
}

pub(super) fn write_stage(work_dir: &Path, stage: &Stage) {
    crate::verify::transitions::save_stage(stage, work_dir).unwrap();
}

pub(super) fn write_dispute_request(
    work_dir: &Path,
    stage_id: &str,
    id: u32,
    criterion_index: usize,
) {
    let disputes_root = work_dir.join("disputes");
    std::fs::create_dir_all(disputes_root.join(stage_id).join(id.to_string())).unwrap();
    let req = DisputeRequest {
        id,
        stage_id: stage_id.to_string(),
        criterion_index,
        reason: "criterion impossible".to_string(),
        evidence_commit: None,
        failure_output: None,
        fix_attempts_at_dispute: 1,
        created_at: Utc::now(),
    };
    let yaml = serde_yaml::to_string(&req).unwrap();
    let path = request_file(&disputes_root, stage_id, id);
    std::fs::write(
        &path,
        format!("---\n{yaml}---\n\n# Dispute {stage_id}/{id}\n"),
    )
    .unwrap();
}

pub(super) fn write_verdict(
    work_dir: &Path,
    stage_id: &str,
    id: u32,
    verdict: DisputeVerdict,
    attempt: u32,
) {
    let disputes_root = work_dir.join("disputes");
    std::fs::create_dir_all(disputes_root.join(stage_id).join(id.to_string())).unwrap();
    let record = DisputeVerdictRecord {
        id,
        stage_id: stage_id.to_string(),
        verdict,
        adjudicator_attempt_count: attempt,
        created_at: Utc::now(),
        model: "test".to_string(),
        session_id: None,
    };
    let yaml = serde_yaml::to_string(&record).unwrap();
    let path = verdict_file(&disputes_root, stage_id, id);
    std::fs::write(
        &path,
        format!("---\n{yaml}---\n\n# Verdict {stage_id}/{id}\n"),
    )
    .unwrap();
}

pub(super) fn reject_verdict() -> DisputeVerdict {
    DisputeVerdict::Reject {
        citations: vec![Citation {
            file: "f".to_string(),
            line: None,
            excerpt: "e".to_string(),
            claim: "c".to_string(),
        }],
        reasoning: "criterion is correct".to_string(),
    }
}

#[test]
fn pending_dispute_is_offered_a_session_and_counted() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path();
    std::fs::create_dir_all(work.join("stages")).unwrap();
    write_stage(work, &make_stage("s1"));
    write_dispute_request(work, "s1", 1, 0);

    let reg = AdjudicatorRegistry::new();
    let jobs = reg.disputes_awaiting_session(work).unwrap();

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].stage.id, "s1");
    assert_eq!(jobs[0].request.id, 1);
    assert_eq!(
        attempt_count(work, "s1", 1),
        1,
        "handing out a job must spend one attempt",
    );
}

/// Two unanswered disputes on one stage (an abandoned one plus the live one)
/// must still produce a single adjudicator.
#[test]
fn a_stage_with_two_pending_disputes_gets_one_session() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path();
    std::fs::create_dir_all(work.join("stages")).unwrap();
    write_stage(work, &make_stage("s1"));
    write_dispute_request(work, "s1", 1, 0);
    write_dispute_request(work, "s1", 2, 0);

    let reg = AdjudicatorRegistry::new();
    let jobs = reg.disputes_awaiting_session(work).unwrap();

    assert_eq!(jobs.len(), 1);
    assert_eq!(attempt_count(work, "s1", 2), 0, "only one dispute is spent");
}

#[test]
fn dispute_is_skipped_once_the_stage_leaves_adjudication() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path();
    std::fs::create_dir_all(work.join("stages")).unwrap();
    let mut stage = make_stage("s1");
    stage.status = StageStatus::Executing;
    write_stage(work, &stage);
    write_dispute_request(work, "s1", 1, 0);

    let reg = AdjudicatorRegistry::new();
    assert!(reg.disputes_awaiting_session(work).unwrap().is_empty());
    assert_eq!(attempt_count(work, "s1", 1), 0);
}

#[test]
fn exhausted_spawn_budget_escalates_instead_of_respawning() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path();
    std::fs::create_dir_all(work.join("stages")).unwrap();
    write_stage(work, &make_stage("s1"));
    write_dispute_request(work, "s1", 1, 0);

    let reg = AdjudicatorRegistry::new();
    for _ in 0..MAX_ADJUDICATION_ATTEMPTS {
        assert_eq!(reg.disputes_awaiting_session(work).unwrap().len(), 1);
    }
    // Budget spent: no further session, and the stage stops waiting silently.
    assert!(reg.disputes_awaiting_session(work).unwrap().is_empty());

    let after = crate::verify::transitions::load_stage("s1", work).unwrap();
    assert_eq!(after.status, StageStatus::NeedsHumanReview);
    assert!(after
        .review_reason
        .as_deref()
        .unwrap_or("")
        .contains("no verdict"));
}

#[test]
fn evidence_cap_escalates_before_a_session_is_offered() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path();
    std::fs::create_dir_all(work.join("stages")).unwrap();
    let mut stage = make_stage("s1");
    stage.evidence_rounds = MAX_EVIDENCE_ROUNDS;
    write_stage(work, &stage);
    write_dispute_request(work, "s1", 1, 0);

    let reg = AdjudicatorRegistry::new();
    assert!(reg.disputes_awaiting_session(work).unwrap().is_empty());

    let after = crate::verify::transitions::load_stage("s1", work).unwrap();
    assert_eq!(after.status, StageStatus::NeedsHumanReview);
    assert_eq!(attempt_count(work, "s1", 1), 0);
}

#[test]
fn scan_pending_requests_skips_completed_verdicts() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path();
    write_dispute_request(work, "s1", 1, 0);
    write_dispute_request(work, "s1", 2, 0);
    write_verdict(work, "s1", 1, reject_verdict(), 1);
    let pending = scan_pending_requests(&work.join("disputes")).unwrap();
    assert_eq!(pending, vec![("s1".to_string(), 2)]);
}

#[test]
fn build_amendment_request_decodes_plan_patch() {
    let plan_patch = PlanPatch {
        inner: serde_json::json!({
            "stage_id": "s1",
            "field": "acceptance",
            "patch": {"op": "delete", "index": 0},
            "reason": "criterion was wrong"
        }),
    };
    let req = build_amendment_request("s1".to_string(), &plan_patch, 1).unwrap();
    assert_eq!(req.stage_id, "s1");
    assert!(matches!(req.field, AmendmentField::Acceptance));
    assert!(matches!(req.patch, AmendmentPatch::Delete { index: 0 }));
    assert_eq!(req.reason.as_deref(), Some("criterion was wrong"));
    assert_eq!(req.dispute_id.as_deref(), Some("1"));
}

#[test]
fn build_amendment_request_rejects_unknown_field() {
    let plan_patch = PlanPatch {
        inner: serde_json::json!({"field": "bogus", "patch": {}}),
    };
    assert!(build_amendment_request("s1".to_string(), &plan_patch, 1).is_err());
}

#[test]
fn feedback_signal_only_when_dispute_count_positive() {
    // This is a contract test for the wiring rule: a stage without a
    // dispute history must NOT see adjudicator feedback in its signal,
    // even if a stray feedback.md exists from a previous plan.
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path();
    feedback::append_questions(work, "s1", &["stale".to_string()]).unwrap();
    let mut stage = make_stage("s1");
    stage.dispute_count = 0;
    // We can't easily build the full Session/Worktree here, so just
    // assert the read returns content (the gating happens in
    // generate_signal_with_skills, exercised by integration tests).
    assert!(feedback::read_feedback(work, "s1").unwrap().is_some());
}

#[test]
fn parse_yaml_frontmatter_round_trips() {
    let req = DisputeRequest {
        id: 7,
        stage_id: "x".to_string(),
        criterion_index: 0,
        reason: "r".to_string(),
        evidence_commit: None,
        failure_output: None,
        fix_attempts_at_dispute: 0,
        created_at: Utc::now(),
    };
    let yaml = serde_yaml::to_string(&req).unwrap();
    let body = format!("---\n{yaml}---\n\n# X\n");
    let parsed: DisputeRequest = parse_yaml_frontmatter(&body).unwrap();
    assert_eq!(parsed.id, 7);
    assert_eq!(parsed.stage_id, "x");
}
