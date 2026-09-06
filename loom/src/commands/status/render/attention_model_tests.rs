use super::*;
use crate::commands::status::data::{ActivityStatus, StageType};
use crate::models::failure::FailureType;
use crate::models::stage::StageStatus;

fn make_stage_summary(id: &str, status: StageStatus) -> StageSummary {
    StageSummary {
        id: id.to_string(),
        name: id.to_string(),
        status,
        stage_type: StageType::Standard,
        dependencies: vec![],
        context_tokens: None,
        elapsed_secs: None,
        execution_secs: None,
        base_branch: None,
        base_merged_from: vec![],
        failure_info: None,
        activity_status: ActivityStatus::default(),
        last_tool: None,
        last_activity: None,
        staleness_secs: None,
        context_ceiling_tokens: None,
        review_reason: None,
        merged: false,
        merge_assumed: false,
        cleanup_warning: None,
        held: false,
        retry_count: 0,
        max_retries: None,
        pid: None,
        session_alive: false,
        model: "opus".to_string(),
        session_type: None,
        incoherence: None,
        execution_models: vec![],
        dispute_count: 0,
        judge_heartbeat_secs: None,
        session_backend: None,
    }
}

fn stages_for_all_statuses() -> Vec<StageSummary> {
    let statuses = [
        StageStatus::WaitingForDeps,
        StageStatus::Queued,
        StageStatus::Executing,
        StageStatus::WaitingForInput,
        StageStatus::Blocked,
        StageStatus::Completed,
        StageStatus::NeedsHandoff,
        StageStatus::Skipped,
        StageStatus::MergeConflict,
        StageStatus::CompletedWithFailures,
        StageStatus::MergeBlocked,
        StageStatus::NeedsHumanReview,
        StageStatus::NeedsAdjudication,
    ];
    statuses
        .into_iter()
        .enumerate()
        .map(|(index, status)| make_stage_summary(&format!("stage-{index}"), status))
        .collect()
}

#[test]
fn entries_cover_adjudication_and_input() {
    let mut stages = stages_for_all_statuses();
    stages[12].dispute_count = 2;
    stages[12].judge_heartbeat_secs = Some(30);

    let entries = attention_entries(&stages);
    let details = entries
        .iter()
        .map(|entry| (entry.label, entry.hint.as_str()))
        .collect::<Vec<_>>();

    assert_eq!(
        details,
        vec![
            ("NEEDS INPUT", "loom stage resume stage-3"),
            ("BLOCKED", "loom stage retry stage-4"),
            ("MERGE CONFLICT", "loom stage merge stage-8"),
            ("ACCEPTANCE FAILED", "loom stage retry stage-9"),
            ("MERGE ERROR", "loom stage merge stage-10"),
            ("NEEDS REVIEW", "loom stage human-review stage-11"),
            ("ADJUDICATING", "loom status --verbose"),
        ]
    );
    assert!(entries.iter().any(|entry| entry.label == "NEEDS INPUT"));
    assert!(entries.iter().any(|entry| entry.label == "ADJUDICATING"));
    let adjudication = entries
        .iter()
        .find(|entry| entry.label == "ADJUDICATING")
        .expect("adjudication entry");
    assert_eq!(adjudication.dispute_count, Some(2));
    assert_eq!(adjudication.judge_heartbeat_secs, Some(30));
}

#[test]
fn healthy_statuses_need_no_attention() {
    let stages = [
        StageStatus::Executing,
        StageStatus::Completed,
        StageStatus::Queued,
        StageStatus::WaitingForDeps,
        StageStatus::Skipped,
        StageStatus::NeedsHandoff,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, status)| make_stage_summary(&format!("stage-{index}"), status))
    .collect::<Vec<_>>();

    assert!(attention_entries(&stages).is_empty());
}

#[test]
fn cleanup_warning_wins_over_completed_status() {
    let mut stage = make_stage_summary("cleanup-stage", StageStatus::Completed);
    stage.cleanup_warning = Some("could not remove worktree".to_string());

    let entries = attention_entries(&[stage]);

    assert_eq!(entries[0].label, "CLEANUP FAILED");
    assert_eq!(entries[0].hint, "loom worktree remove cleanup-stage");
}

#[test]
fn failure_labels_are_short_and_stable() {
    assert_eq!(failure_label(&FailureType::TestFailure), "test");
    assert_eq!(failure_label(&FailureType::SandboxSetupFailure), "sandbox");
}

#[test]
fn entries_follow_input_order() {
    let stages = vec![
        make_stage_summary("second", StageStatus::MergeConflict),
        make_stage_summary("first", StageStatus::Blocked),
        make_stage_summary("third", StageStatus::WaitingForInput),
    ];

    let ids = attention_entries(&stages)
        .into_iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>();

    assert_eq!(ids, ["second", "first", "third"]);
}
