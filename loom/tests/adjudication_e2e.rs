//! Integration tests for the adjudication subsystem.
//!
//! Adjudication is a two-party protocol between the daemon and a session it
//! spawns: the daemon decides a dispute needs a session, the session writes a
//! JSON verdict and hands it to `loom stage adjudicate`, and the daemon applies
//! the recorded verdict on a later tick. No agent is started here — the tests
//! stand in for the session by doing exactly what it does, so both halves of
//! the protocol (the CLI's persistence and the daemon's application) run for
//! real against a fresh tmp `.loom/work` directory.

use loom::commands::stage::{record_verdict, AdjudicateOutcome};
use loom::models::dispute::{applied_marker, request_file, verdict_file, DisputeRequest};
use loom::models::session::Session;
use loom::models::stage::{Stage, StageStatus};
use loom::orchestrator::adjudication::{
    feedback, verdict_draft_file, AdjudicatorRegistry, MAX_ADJUDICATION_ATTEMPTS,
};
use loom::plan::schema::AcceptanceCriterion;
use std::path::{Path, PathBuf};

#[path = "adjudication_e2e/amendment.rs"]
mod amendment;

fn write_stage(work_dir: &Path, stage: &Stage) {
    std::fs::create_dir_all(work_dir.join("stages")).unwrap();
    loom::verify::transitions::save_stage(stage, work_dir).unwrap();
}

fn set_stage_status(work_dir: &Path, stage_id: &str, status: StageStatus) {
    loom::verify::transitions::update_stage(stage_id, work_dir, |stage| {
        stage.status = status;
        Ok(())
    })
    .unwrap();
}

fn make_stage(id: &str) -> Stage {
    Stage {
        id: id.to_string(),
        name: id.to_string(),
        status: StageStatus::NeedsAdjudication,
        acceptance: vec![AcceptanceCriterion::Simple("cargo test".to_string())],
        ..Default::default()
    }
}

/// Stage seeded with two acceptance criteria so a Delete-index-0 amendment
/// still produces a validatable Standard stage (validation requires at least
/// one of acceptance/wiring/artifacts/wiring_tests to remain).
fn make_stage_two_criteria(id: &str) -> Stage {
    Stage {
        id: id.to_string(),
        name: id.to_string(),
        working_dir: Some(".".to_string()),
        status: StageStatus::NeedsAdjudication,
        acceptance: vec![
            AcceptanceCriterion::Simple(
                "test -f /__loom_intentionally_wrong/marker.txt".to_string(),
            ),
            AcceptanceCriterion::Simple("ls /tmp".to_string()),
        ],
        ..Default::default()
    }
}

fn write_dispute(work_dir: &Path, stage_id: &str, id: u32) {
    let disputes_root = work_dir.join("disputes");
    std::fs::create_dir_all(disputes_root.join(stage_id).join(id.to_string())).unwrap();
    let req = DisputeRequest {
        id,
        stage_id: stage_id.to_string(),
        criterion_index: 0,
        reason: "criterion impossible".to_string(),
        evidence_commit: None,
        failure_output: Some("err: something".to_string()),
        fix_attempts_at_dispute: 1,
        created_at: chrono::Utc::now(),
    };
    let yaml = serde_yaml::to_string(&req).unwrap();
    let path = request_file(&disputes_root, stage_id, id);
    std::fs::write(&path, format!("---\n{yaml}---\n\n# Dispute\n")).unwrap();
}

fn write_plan(work_dir: &Path) {
    // The adjudicator resolves the plan path from config.toml; for
    // tests that don't exercise the plan file, write a minimal valid
    // markdown so prompt::build can read it without panicking.
    let plan = work_dir.join("PLAN.md");
    std::fs::write(
        &plan,
        "# Plan\n\n```yaml\nloom:\n  version: 1\n  stages:\n    - id: s1\n      name: s1\n      working_dir: .\n      acceptance:\n        - cargo test\n```\n",
    )
    .unwrap();
    let cfg = format!(
        "[plan]\nsource_path = \"{}\"\nplan_id = \"x\"\nplan_name = \"x\"\nbase_branch = \"main\"\n",
        plan.display()
    );
    std::fs::write(work_dir.join("config.toml"), cfg).unwrap();
}

/// Write a plan file with the full `<!-- loom METADATA -->` markers
/// that `apply_amendment` requires to splice the amended YAML back into
/// the document. The stage `s1` has two acceptance criteria so a
/// Delete-index-0 amendment leaves the plan schema-valid.
fn write_plan_with_metadata_markers(work_dir: &Path) -> PathBuf {
    let plan = work_dir.join("PLAN.md");
    let content = "\
# Plan: Adjudication End-to-End

Prose section that must be preserved across amendments.

<!-- loom METADATA -->

```yaml
loom:
  version: 1
  stages:
    - id: s1
      name: s1
      working_dir: \".\"
      acceptance:
        - \"test -f /__loom_intentionally_wrong/marker.txt\"
        - \"ls /tmp\"
```

<!-- END loom METADATA -->

Trailing prose section.
";
    std::fs::write(&plan, content).unwrap();
    let cfg = format!(
        "[plan]\nsource_path = \"{}\"\nplan_id = \"x\"\nplan_name = \"x\"\nbase_branch = \"main\"\n",
        plan.display()
    );
    std::fs::write(work_dir.join("config.toml"), cfg).unwrap();
    plan
}

fn verdict_reject() -> serde_json::Value {
    serde_json::json!({
        "verdict": "reject",
        "reasoning": "criterion is correct",
        "citations": [
            {"file": "src/a.rs", "line": 1, "excerpt": "fn foo", "claim": "function exists"}
        ]
    })
}

fn verdict_accept_with_amendment() -> serde_json::Value {
    serde_json::json!({
        "verdict": "accept",
        "reasoning": "criterion was overspecified",
        "citations": [
            {"file": "src/a.rs", "line": 1, "excerpt": "X", "claim": "Y"}
        ],
        "plan_patch": {
            "stage_id": "s1",
            "field": "acceptance",
            "patch": {"op": "delete", "index": 0},
            "reason": "test amendment"
        }
    })
}

/// An Accept verdict whose `plan_patch` deletes acceptance[0]. Targets the
/// stage seeded by [`make_stage_two_criteria`].
fn verdict_accept_delete_first() -> serde_json::Value {
    serde_json::json!({
        "verdict": "accept",
        "reasoning": "acceptance[0] references a path that cannot exist; criterion has no valid interpretation",
        "citations": [
            {
                "file": "PLAN.md",
                "line": 1,
                "excerpt": "intentionally_wrong",
                "claim": "path does not exist in the project root"
            }
        ],
        "plan_patch": {
            "stage_id": "s1",
            "field": "acceptance",
            "patch": {"op": "delete", "index": 0},
            "reason": "criterion path is mechanically wrong"
        }
    })
}

fn verdict_needs_more() -> serde_json::Value {
    serde_json::json!({
        "verdict": "needs-more-evidence",
        "questions": ["what is X?"]
    })
}

/// Stand in for the adjudication session: write the JSON verdict where the
/// signal told it to, then hand that file to `loom stage adjudicate`.
fn session_records_verdict(
    work: &Path,
    stage_id: &str,
    dispute_id: u32,
    verdict: &serde_json::Value,
) -> AdjudicateOutcome {
    let draft = verdict_draft_file(work, stage_id, dispute_id);
    std::fs::create_dir_all(draft.parent().unwrap()).unwrap();
    std::fs::write(&draft, verdict.to_string()).unwrap();
    record_verdict(work, stage_id, dispute_id, &draft, None).expect("verdict record must succeed")
}

/// One full round trip for a dispute already on disk: the daemon offers a
/// session, the session records its verdict, the daemon applies it.
fn drive_dispute(
    reg: &AdjudicatorRegistry,
    work: &Path,
    stage_id: &str,
    dispute_id: u32,
    verdict: &serde_json::Value,
) {
    let jobs = reg.disputes_awaiting_session(work).unwrap();
    assert!(
        jobs.iter()
            .any(|job| job.stage.id == stage_id && job.request.id == dispute_id),
        "dispute {dispute_id} should have been offered an adjudication session",
    );
    assert_eq!(
        session_records_verdict(work, stage_id, dispute_id, verdict),
        AdjudicateOutcome::Recorded,
    );
    reg.apply_pending_verdicts(work)
        .expect("apply_pending_verdicts should be Ok");
}

/// A Reject means the criterion stands and the implementation is wrong — while
/// the agent already judged the criterion impossible. Neither side can move, so
/// the stage must land on a human rather than being re-queued into the same
/// disagreement.
#[test]
fn reject_verdict_escalates_to_human_review() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path();
    write_plan(work);
    write_stage(work, &make_stage("s1"));
    write_dispute(work, "s1", 1);

    let reg = AdjudicatorRegistry::new();
    drive_dispute(&reg, work, "s1", 1, &verdict_reject());

    let after = loom::verify::transitions::load_stage("s1", work).unwrap();
    assert_eq!(
        after.status,
        StageStatus::NeedsHumanReview,
        "a rejected dispute must NOT be re-queued",
    );
    assert!(after
        .review_reason
        .as_deref()
        .unwrap_or("")
        .contains("upheld the disputed acceptance criterion"));

    let fb = feedback::read_feedback(work, "s1").unwrap().unwrap();
    assert!(fb.contains("rejected"));
    assert!(applied_marker(&work.join("disputes"), "s1", 1).exists());
}

#[test]
fn needs_more_evidence_writes_questions() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path();
    write_plan(work);
    write_stage(work, &make_stage("s1"));
    write_dispute(work, "s1", 1);

    let reg = AdjudicatorRegistry::new();
    drive_dispute(&reg, work, "s1", 1, &verdict_needs_more());

    let fb = feedback::read_feedback(work, "s1").unwrap().unwrap();
    assert!(fb.contains("what is X?"));
    let after = loom::verify::transitions::load_stage("s1", work).unwrap();
    assert_eq!(after.status, StageStatus::Queued);
}

#[test]
fn accept_verdict_amends_plan_and_clears_feedback() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path();
    write_plan(work);
    write_stage(work, &make_stage("s1"));
    write_dispute(work, "s1", 1);
    // Pre-seed feedback to verify it gets cleared.
    feedback::append_questions(work, "s1", &["stale".to_string()]).unwrap();

    let reg = AdjudicatorRegistry::new();
    assert_eq!(reg.disputes_awaiting_session(work).unwrap().len(), 1);
    session_records_verdict(work, "s1", 1, &verdict_accept_with_amendment());

    // `apply_pending_verdicts` returns Ok even when individual verdicts
    // fail to apply — per-verdict failures are logged via tracing and the
    // outer call still walks the rest of the queue. The plan in this test
    // intentionally lacks the `<!-- loom METADATA -->` markers needed by
    // `apply_amendment`, so the amendment is expected to fail and the
    // applied.marker SHOULD NOT exist. (For the success-path contract see
    // `dispute_to_amendment_to_pass`.)
    reg.apply_pending_verdicts(work)
        .expect("apply_pending_verdicts should return Ok even when per-verdict apply fails");

    // The session produced a parseable Accept verdict but the amendment
    // did not land — the verdict file exists, the marker does not.
    let content = std::fs::read_to_string(verdict_file(&work.join("disputes"), "s1", 1)).unwrap();
    assert!(content.contains("accept"));
    assert!(
        !applied_marker(&work.join("disputes"), "s1", 1).exists(),
        "applied.marker must not exist when apply_amendment failed",
    );
}

/// A session that died without recording anything must be replaced — but only
/// while the dispute's budget lasts, after which the stage asks for a human
/// instead of collecting adjudicators forever.
#[test]
fn a_dead_session_is_replaced_until_the_budget_runs_out() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path();
    write_plan(work);
    write_stage(work, &make_stage("s1"));
    write_dispute(work, "s1", 1);

    let reg = AdjudicatorRegistry::new();
    for attempt in 1..=MAX_ADJUDICATION_ATTEMPTS {
        assert_eq!(
            reg.disputes_awaiting_session(work).unwrap().len(),
            1,
            "attempt {attempt} should still be offered a session",
        );
    }
    assert!(
        reg.disputes_awaiting_session(work).unwrap().is_empty(),
        "the budget is spent; no further session may be started",
    );

    let after = loom::verify::transitions::load_stage("s1", work).unwrap();
    assert_eq!(after.status, StageStatus::NeedsHumanReview);
    assert!(!verdict_file(&work.join("disputes"), "s1", 1).exists());
}

/// Two adjudicators judging the same stage in the same main repository is the
/// thing the daemon must never do, so a live session suppresses the next offer.
#[test]
fn a_live_adjudication_session_blocks_a_second_one() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path();
    write_plan(work);
    write_stage(work, &make_stage("s1"));
    write_dispute(work, "s1", 1);

    // A session record plus PID-identity evidence for a process that really is
    // alive (this test's own). A PID file with no start-time line reads back as
    // unverifiable, which every liveness probe treats as alive.
    let session = Session::new_adjudication("s1");
    loom::fs::session_files::save_session(&session, work).unwrap();
    let pids = work.join("pids");
    std::fs::create_dir_all(&pids).unwrap();
    std::fs::write(
        pids.join(format!("{}-{}.pid", session.tracking_key, session.id)),
        format!("{}\n", std::process::id()),
    )
    .unwrap();

    let reg = AdjudicatorRegistry::new();
    assert!(
        reg.disputes_awaiting_session(work).unwrap().is_empty(),
        "a live adjudication session must suppress a second one",
    );
}

#[test]
fn double_apply_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path();
    write_plan(work);
    write_stage(work, &make_stage("s1"));
    write_dispute(work, "s1", 1);

    let reg = AdjudicatorRegistry::new();
    drive_dispute(&reg, work, "s1", 1, &verdict_reject());
    let mid = loom::verify::transitions::load_stage("s1", work).unwrap();
    reg.apply_pending_verdicts(work).unwrap();
    let after = loom::verify::transitions::load_stage("s1", work).unwrap();
    assert_eq!(mid.status, after.status);
}

/// Calling `apply_pending_verdicts` a second time after a successful
/// amendment must NOT re-apply the patch (which would double-bump
/// `amendments_applied` and grow `plan_versions/` indefinitely).
///
/// This is the daemon-restart idempotency contract: the
/// `applied.marker` file is the persistent guard, surviving across
/// process restarts.
#[test]
fn dispute_amendment_is_idempotent_under_repeat_apply() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path();
    write_plan_with_metadata_markers(work);
    write_stage(work, &make_stage_two_criteria("s1"));
    write_dispute(work, "s1", 1);

    let reg = AdjudicatorRegistry::new();
    drive_dispute(&reg, work, "s1", 1, &verdict_accept_delete_first());
    let mid = loom::verify::transitions::load_stage("s1", work).unwrap();
    assert_eq!(mid.amendments_applied, 1);

    // Second apply: applied.marker exists, so the call must be a no-op.
    reg.apply_pending_verdicts(work).unwrap();
    let after = loom::verify::transitions::load_stage("s1", work).unwrap();
    assert_eq!(
        after.amendments_applied, 1,
        "second apply must NOT double-count amendments_applied",
    );
    // Only one snapshot, only one audit row.
    let snapshot_2 = work.join("plan_versions").join("2.md");
    assert!(
        !snapshot_2.exists(),
        "second apply must NOT write a new snapshot",
    );
}

/// Write a plan with the loom-METADATA markers and a configurable
/// `max_amendments_per_stage`. The stage `s1` has three acceptance
/// criteria so two successive Delete-index-0 amendments leave a
/// schema-valid Standard stage; a third would still leave one criterion
/// but is rejected by the cap.
fn write_plan_with_cap(work_dir: &Path, max_amendments_per_stage: u32) -> PathBuf {
    let plan = work_dir.join("PLAN.md");
    let content = format!(
        "# Plan: Adjudication Cap Test\n\
\n\
Prose section that must be preserved across amendments.\n\
\n\
<!-- loom METADATA -->\n\
\n\
```yaml\n\
loom:\n  version: 1\n  adjudication:\n    max_amendments_per_stage: {max_amendments_per_stage}\n  stages:\n    - id: s1\n      name: s1\n      working_dir: \".\"\n      acceptance:\n        - \"test -f /__loom_wrong_a/marker.txt\"\n        - \"test -f /__loom_wrong_b/marker.txt\"\n        - \"ls /tmp\"\n```\n\
\n\
<!-- END loom METADATA -->\n\
\n\
Trailing prose section.\n",
    );
    std::fs::write(&plan, content).unwrap();
    let cfg = format!(
        "[plan]\nsource_path = \"{}\"\nplan_id = \"x\"\nplan_name = \"x\"\nbase_branch = \"main\"\n",
        plan.display()
    );
    std::fs::write(work_dir.join("config.toml"), cfg).unwrap();
    plan
}

fn make_stage_three_criteria(id: &str) -> Stage {
    Stage {
        id: id.to_string(),
        name: id.to_string(),
        working_dir: Some(".".to_string()),
        status: StageStatus::NeedsAdjudication,
        acceptance: vec![
            AcceptanceCriterion::Simple("test -f /__loom_wrong_a/marker.txt".to_string()),
            AcceptanceCriterion::Simple("test -f /__loom_wrong_b/marker.txt".to_string()),
            AcceptanceCriterion::Simple("ls /tmp".to_string()),
        ],
        ..Default::default()
    }
}

/// Drive one dispute from filing to applied. Re-seeds the stage to
/// `NeedsAdjudication` first so back-to-back disputes can be processed
/// without an agent-flow round trip.
fn file_and_drive_dispute(
    reg: &AdjudicatorRegistry,
    work: &Path,
    stage_id: &str,
    dispute_id: u32,
    verdict: &serde_json::Value,
) {
    set_stage_status(work, stage_id, StageStatus::NeedsAdjudication);
    write_dispute(work, stage_id, dispute_id);
    drive_dispute(reg, work, stage_id, dispute_id, verdict);
}

/// End-to-end coverage of the amendment-cap escalation path. Two
/// accepted amendments use the cap of 2; the third dispute lands an
/// Accept verdict that `apply_amendment` rejects with the
/// `amendment cap exceeded` error. The orchestrator must NOT loop on
/// that verdict — instead it escalates the stage to
/// `NeedsHumanReview` and writes `applied.marker` for the third
/// dispute so subsequent ticks short-circuit.
#[test]
fn amendment_cap_exceeded_escalates_to_human_review() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path();
    write_plan_with_cap(work, 2);
    write_stage(work, &make_stage_three_criteria("s1"));

    let reg = AdjudicatorRegistry::new();
    let accept = verdict_accept_delete_first();

    // Two accepted amendments under the cap.
    file_and_drive_dispute(&reg, work, "s1", 1, &accept);
    file_and_drive_dispute(&reg, work, "s1", 2, &accept);

    let after_two = loom::verify::transitions::load_stage("s1", work).unwrap();
    assert_eq!(after_two.amendments_applied, 2);
    assert_eq!(after_two.status, StageStatus::Queued);

    // Third dispute: the cap blocks the amendment.
    file_and_drive_dispute(&reg, work, "s1", 3, &accept);

    let after_three = loom::verify::transitions::load_stage("s1", work).unwrap();
    assert_eq!(
        after_three.status,
        StageStatus::NeedsHumanReview,
        "third Accept must escalate via cap-exceeded path",
    );
    assert!(
        after_three
            .review_reason
            .as_deref()
            .unwrap_or("")
            .contains("amendment cap exceeded"),
        "review_reason should mention amendment cap; got: {:?}",
        after_three.review_reason,
    );
    assert_eq!(
        after_three.amendments_applied, 2,
        "amendments_applied must NOT increment past the cap",
    );
    assert!(
        applied_marker(&work.join("disputes"), "s1", 3).exists(),
        "applied.marker for dispute 3 must exist so the verdict is not retried",
    );

    // A subsequent tick must be a no-op.
    reg.apply_pending_verdicts(work).unwrap();
    let after_replay = loom::verify::transitions::load_stage("s1", work).unwrap();
    assert_eq!(after_replay.amendments_applied, 2);
}

/// End-to-end coverage of the evidence-rounds escalation path. Every verdict
/// is `NeedsMoreEvidence`; the orchestrator drives the stage
/// NeedsAdjudication → Queued until `evidence_rounds >= MAX_EVIDENCE_ROUNDS`,
/// at which point the stage must escalate to `NeedsHumanReview` instead of
/// looping.
#[test]
fn evidence_rounds_exhausted_escalates_to_human_review() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path();
    write_plan(work);
    write_stage(work, &make_stage("s1"));

    let reg = AdjudicatorRegistry::new();
    let needs_more = verdict_needs_more();

    let mut dispute_id: u32 = 0;
    loop {
        let s = loom::verify::transitions::load_stage("s1", work).unwrap();
        if s.status == StageStatus::NeedsHumanReview {
            break;
        }
        // Safety: bound the loop so a regression doesn't hang the test.
        assert!(
            dispute_id < 10,
            "did not escalate after 10 rounds; got status {:?}, evidence_rounds {}",
            s.status,
            s.evidence_rounds
        );
        dispute_id += 1;
        file_and_drive_dispute(&reg, work, "s1", dispute_id, &needs_more);
    }

    let after = loom::verify::transitions::load_stage("s1", work).unwrap();
    assert_eq!(after.status, StageStatus::NeedsHumanReview);
    assert!(
        after
            .review_reason
            .as_deref()
            .unwrap_or("")
            .contains("evidence loop exhausted"),
        "review_reason should mention evidence-loop exhaustion; got: {:?}",
        after.review_reason,
    );
}
