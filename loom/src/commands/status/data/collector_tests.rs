use super::*;

pub(super) fn make_test_stage(id: &str, status: StageStatus) -> Stage {
    Stage {
        id: id.to_string(),
        name: id.to_string(),
        status,
        ..Stage::default()
    }
}

/// A fresh, initialized `.loom/work`-style temp directory for tests that
/// call `build_stage_summary` and need a real `WorkDir` to read from.
pub(super) fn temp_work_dir() -> (tempfile::TempDir, WorkDir) {
    let tmp = tempfile::TempDir::new().unwrap();
    let work_dir = WorkDir::new(tmp.path()).unwrap();
    work_dir.initialize().unwrap();
    (tmp, work_dir)
}
#[test]
fn test_calculate_progress() {
    let stages = vec![
        make_test_stage("stage-1", StageStatus::Completed),
        make_test_stage("stage-2", StageStatus::Executing),
        make_test_stage("stage-3", StageStatus::WaitingForDeps),
        make_test_stage("stage-4", StageStatus::Queued),
        make_test_stage("stage-5", StageStatus::Blocked),
    ];

    let progress = calculate_progress(&stages);

    assert_eq!(progress.total, 5);
    assert_eq!(progress.completed, 1);
    assert_eq!(progress.executing, 1);
    assert_eq!(progress.pending, 2); // WaitingForDeps + Queued
    assert_eq!(progress.blocked, 1);
}
#[test]
fn test_calculate_progress_with_needs_handoff() {
    let stages = vec![
        make_test_stage("stage-1", StageStatus::NeedsHandoff),
        make_test_stage("stage-2", StageStatus::WaitingForInput),
    ];

    let progress = calculate_progress(&stages);

    assert_eq!(progress.total, 2);
    assert_eq!(progress.executing, 2); // Both count as executing
}

#[test]
fn test_calculate_progress_with_failures() {
    let stages = vec![
        make_test_stage("stage-1", StageStatus::CompletedWithFailures),
        make_test_stage("stage-2", StageStatus::MergeConflict),
        make_test_stage("stage-3", StageStatus::MergeBlocked),
    ];

    let progress = calculate_progress(&stages);

    assert_eq!(progress.total, 3);
    assert_eq!(progress.blocked, 3); // All count as blocked
}

#[test]
fn test_build_session_summary() {
    let mut session = Session::new();
    session.assign_to_stage("test-stage".to_string());
    session.pid = Some(12345);
    session.context_tokens = 100000;

    let summary = build_session_summary(&session);

    assert_eq!(summary.stage_id, Some("test-stage".to_string()));
    assert_eq!(summary.pid, Some(12345));
    assert_eq!(summary.context_tokens, 100000);
    assert!(summary.uptime_secs >= 0);
}

#[test]
fn test_build_merge_summary_from_report() {
    let mut report = crate::commands::status::merge_status::MergeStatusReport::new();
    report.merged.push("stage-1".to_string());
    report.pending.push("stage-2".to_string());
    report.conflicts.push("stage-3".to_string());

    let summary = build_merge_summary_from_report(&report);

    assert_eq!(summary.merged, vec!["stage-1"]);
    assert_eq!(summary.pending, vec!["stage-2"]);
    assert_eq!(summary.conflicts, vec!["stage-3"]);
}

#[test]
fn test_parse_session_from_markdown() {
    let content = r#"---
id: test-session
status: running
context_tokens: 1000
created_at: "2024-01-01T00:00:00Z"
last_active: "2024-01-01T00:00:00Z"
---

# Session content"#;

    let result: Result<Session> = parse_from_markdown(content, "Session");
    assert!(result.is_ok());
    let session = result.unwrap();
    assert_eq!(session.id, "test-session");
}

#[test]
fn test_parse_session_from_markdown_missing_delimiter() {
    let content = r#"id: test
status: executing"#;

    let result: Result<Session> = parse_from_markdown(content, "Session");
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("No frontmatter delimiter"));
}
