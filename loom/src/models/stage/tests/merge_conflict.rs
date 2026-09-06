use crate::models::stage::StageStatus;

use super::create_test_stage;

#[test]
fn test_executing_can_transition_to_merge_conflict() {
    let status = StageStatus::Executing;
    assert!(status.can_transition_to(&StageStatus::MergeConflict));
}

#[test]
fn test_merge_conflict_can_transition_to_completed() {
    let status = StageStatus::MergeConflict;
    assert!(status.can_transition_to(&StageStatus::Completed));
}

#[test]
fn test_merge_conflict_can_transition_to_blocked() {
    let status = StageStatus::MergeConflict;
    assert!(status.can_transition_to(&StageStatus::Blocked));
}

#[test]
fn test_merge_conflict_cannot_transition_to_other_states() {
    let status = StageStatus::MergeConflict;
    assert!(!status.can_transition_to(&StageStatus::WaitingForDeps));
    assert!(!status.can_transition_to(&StageStatus::Queued));
    assert!(!status.can_transition_to(&StageStatus::Executing));
    assert!(!status.can_transition_to(&StageStatus::NeedsHandoff));
    assert!(!status.can_transition_to(&StageStatus::WaitingForInput));
    assert!(!status.can_transition_to(&StageStatus::Skipped));
}

#[test]
fn test_stage_try_mark_merge_conflict_valid() {
    let mut stage = create_test_stage(StageStatus::Executing);
    let result = stage.try_mark_merge_conflict();
    assert!(result.is_ok());
    assert_eq!(stage.status, StageStatus::MergeConflict);
    assert!(stage.merge_conflict); // merge_conflict flag should be set
}

#[test]
fn test_stage_try_mark_merge_conflict_invalid() {
    let mut stage = create_test_stage(StageStatus::WaitingForDeps);
    let result = stage.try_mark_merge_conflict();
    assert!(result.is_err());
    assert_eq!(stage.status, StageStatus::WaitingForDeps);
    assert!(!stage.merge_conflict);
}

#[test]
fn test_stage_try_complete_merge_valid() {
    let mut stage = create_test_stage(StageStatus::MergeConflict);
    stage.merge_conflict = true;
    let result = stage.try_complete_merge();
    assert!(result.is_ok());
    assert_eq!(stage.status, StageStatus::Completed);
    assert!(!stage.merge_conflict); // merge_conflict flag should be cleared
    assert!(stage.merged); // merged flag should be set
    assert!(stage.completed_at.is_some());
}

#[test]
fn test_stage_try_complete_merge_from_completed_keeps_completed_at() {
    // A failed auto-merge leaves a stage at Completed + !merged; a later
    // retry (`try_complete_merge` on an already-Completed stage) must not
    // reset `completed_at` to now.
    let mut stage = create_test_stage(StageStatus::Completed);
    stage.merged = false;
    let preset_completed_at = chrono::Utc::now() - chrono::Duration::hours(1);
    stage.completed_at = Some(preset_completed_at);

    let result = stage.try_complete_merge();

    assert!(result.is_ok());
    assert!(stage.merged);
    assert_eq!(stage.status, StageStatus::Completed);
    assert_eq!(stage.completed_at, Some(preset_completed_at));
}

#[test]
fn test_stage_try_complete_merge_invalid() {
    // try_complete_merge should fail from states that can't transition to Completed
    let mut stage = create_test_stage(StageStatus::WaitingForDeps);
    let result = stage.try_complete_merge();
    assert!(result.is_err());
    assert_eq!(stage.status, StageStatus::WaitingForDeps);
    assert!(!stage.merged); // merged flag should not be set on failure
}

#[test]
fn test_merge_conflict_recovery_workflow() {
    // Scenario: Stage is executing, merge has conflicts, resolution succeeds
    let mut stage = create_test_stage(StageStatus::Executing);

    // Executing -> MergeConflict (merge detected conflicts)
    assert!(stage.try_mark_merge_conflict().is_ok());
    assert_eq!(stage.status, StageStatus::MergeConflict);
    assert!(stage.merge_conflict);

    // MergeConflict -> Completed (conflicts resolved)
    assert!(stage.try_complete_merge().is_ok());
    assert_eq!(stage.status, StageStatus::Completed);
    assert!(!stage.merge_conflict);
    assert!(stage.merged);
}

#[test]
fn test_merge_conflict_failure_workflow() {
    // Scenario: Stage has merge conflicts, resolution fails
    let mut stage = create_test_stage(StageStatus::Executing);

    // Executing -> MergeConflict
    assert!(stage.try_mark_merge_conflict().is_ok());
    assert_eq!(stage.status, StageStatus::MergeConflict);

    // MergeConflict -> Blocked (resolution failed)
    assert!(stage.try_mark_blocked().is_ok());
    assert_eq!(stage.status, StageStatus::Blocked);
}

#[test]
fn test_merge_conflict_display() {
    assert_eq!(format!("{}", StageStatus::MergeConflict), "MergeConflict");
}

#[test]
fn test_same_status_transition_includes_merge_conflict() {
    let status = StageStatus::MergeConflict;
    assert!(status.can_transition_to(&status.clone()));
}
