use super::*;
use crate::commands::status::data::{ActivityStatus, MergeSummary, ProgressSummary, StageType};
use crate::models::session::SessionType;

fn make_stage_summary(id: &str, deps: Vec<&str>, status: StageStatus) -> StageSummary {
    StageSummary {
        id: id.to_string(),
        name: id.to_string(),
        status,
        stage_type: StageType::Standard,
        dependencies: deps.into_iter().map(String::from).collect(),
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

fn make_status_data(stages: Vec<StageSummary>) -> StatusData {
    StatusData {
        stages,
        merge: MergeSummary {
            merged: vec![],
            pending: vec![],
            conflicts: vec![],
        },
        progress: ProgressSummary {
            total: 0,
            completed: 0,
            executing: 0,
            pending: 0,
            blocked: 0,
        },
        plan_name: None,
        quota: crate::quota::QuotaSnapshot::default(),
    }
}

#[test]
fn test_compute_stage_levels_empty() {
    let stages: Vec<StageSummary> = vec![];
    let levels = compute_stage_levels(&stages);
    assert!(levels.is_empty());
}

#[test]
fn test_compute_stage_levels_linear() {
    let stages = vec![
        make_stage_summary("a", vec![], StageStatus::Completed),
        make_stage_summary("b", vec!["a"], StageStatus::Completed),
        make_stage_summary("c", vec!["b"], StageStatus::Completed),
    ];
    let levels = compute_stage_levels(&stages);
    assert_eq!(levels.get("a"), Some(&0));
    assert_eq!(levels.get("b"), Some(&1));
    assert_eq!(levels.get("c"), Some(&2));
}

#[test]
fn test_compute_stage_levels_diamond() {
    let stages = vec![
        make_stage_summary("a", vec![], StageStatus::Completed),
        make_stage_summary("b", vec!["a"], StageStatus::Completed),
        make_stage_summary("c", vec!["a"], StageStatus::Completed),
        make_stage_summary("d", vec!["b", "c"], StageStatus::Completed),
    ];
    let levels = compute_stage_levels(&stages);
    assert_eq!(levels.get("a"), Some(&0));
    assert_eq!(levels.get("b"), Some(&1));
    assert_eq!(levels.get("c"), Some(&1));
    assert_eq!(levels.get("d"), Some(&2));
}

#[test]
fn test_render_graph_empty() {
    let data = make_status_data(vec![]);
    let mut output = Vec::new();
    render_graph(&mut output, &data).unwrap();
    let output_str = String::from_utf8(output).unwrap();
    assert!(output_str.contains("no stages found"));
}

#[test]
fn test_render_graph_linear() {
    let data = make_status_data(vec![
        make_stage_summary("bootstrap", vec![], StageStatus::Completed),
        make_stage_summary("implement", vec!["bootstrap"], StageStatus::Executing),
        make_stage_summary("verify", vec!["implement"], StageStatus::WaitingForDeps),
    ]);
    let mut output = Vec::new();
    render_graph(&mut output, &data).unwrap();
    let output_str = String::from_utf8(output).unwrap();
    assert!(output_str.contains("bootstrap"));
    assert!(output_str.contains("implement"));
    assert!(output_str.contains("verify"));
    // Tree connectors should be present (Option E uses 2-char └─/├─ glyphs).
    assert!(output_str.contains("├─") || output_str.contains("└─"));
}

#[test]
fn test_render_graph_with_context() {
    let mut stage = make_stage_summary("executing", vec![], StageStatus::Executing);
    stage.context_tokens = Some(45_000);
    stage.context_ceiling_tokens = Some(150_000);
    stage.elapsed_secs = Some(120);

    let data = make_status_data(vec![stage]);
    let mut output = Vec::new();
    render_graph(&mut output, &data).unwrap();
    let output_str = String::from_utf8(output).unwrap();
    assert!(output_str.contains("45000/150000"));
    assert!(output_str.contains("2m0s"));
}

#[test]
fn test_status_indicators() {
    // Just verify they don't panic
    let _ = status_indicator(&StageStatus::Completed);
    let _ = status_indicator(&StageStatus::Executing);
    let _ = status_indicator(&StageStatus::Blocked);
    let _ = status_indicator(&StageStatus::NeedsHandoff);
}

#[test]
fn test_completed_unmerged_standard_shows_hint() {
    // A standard stage that is Completed but not yet merged should show "unmerged"
    // and a merge hint line.
    let mut stage = make_stage_summary("my-stage", vec![], StageStatus::Completed);
    stage.merged = false;
    stage.stage_type = StageType::Standard;

    let data = make_status_data(vec![stage]);
    let mut output = Vec::new();
    render_graph(&mut output, &data).unwrap();
    let output_str = String::from_utf8(output).unwrap();

    assert!(
        output_str.contains("unmerged"),
        "Expected 'unmerged' label in output"
    );
    assert!(
        output_str.contains("loom stage merge my-stage"),
        "Expected merge hint in output"
    );
}

#[test]
fn test_completed_merged_standard_shows_merged() {
    // A standard stage that is Completed and merged should show "merged", no hint.
    let mut stage = make_stage_summary("my-stage", vec![], StageStatus::Completed);
    stage.merged = true;
    stage.stage_type = StageType::Standard;

    let data = make_status_data(vec![stage]);
    let mut output = Vec::new();
    render_graph(&mut output, &data).unwrap();
    let output_str = String::from_utf8(output).unwrap();

    assert!(
        output_str.contains("merged"),
        "Expected 'merged' label in output"
    );
    assert!(
        !output_str.contains("loom stage merge"),
        "Should not show merge hint when already merged"
    );
    assert!(
        !output_str.contains("unmerged"),
        "Should not show 'unmerged' when stage is merged"
    );
}

#[test]
fn test_completed_merge_assumed_standard_shows_assumed() {
    // A merge asserted through --assume-merged reads "assumed", not "merged" -
    // the dashboard must not claim loom performed a merge it never ran.
    let mut stage = make_stage_summary("my-stage", vec![], StageStatus::Completed);
    stage.merged = true;
    stage.merge_assumed = true;
    stage.stage_type = StageType::Standard;

    let data = make_status_data(vec![stage]);
    let mut output = Vec::new();
    render_graph(&mut output, &data).unwrap();
    let output_str = String::from_utf8(output).unwrap();

    assert!(
        output_str.contains("assumed"),
        "Expected 'assumed' label in output"
    );
}

#[test]
fn test_completed_unmerged_knowledge_no_hint() {
    // A knowledge stage that is Completed but not merged should NOT show "unmerged"
    // or a merge hint (knowledge stages have different merge semantics).
    let mut stage = make_stage_summary("knowledge-bootstrap", vec![], StageStatus::Completed);
    stage.merged = false;
    stage.stage_type = StageType::Knowledge;

    let data = make_status_data(vec![stage]);
    let mut output = Vec::new();
    render_graph(&mut output, &data).unwrap();
    let output_str = String::from_utf8(output).unwrap();

    assert!(
        !output_str.contains("unmerged"),
        "Knowledge stages should not show 'unmerged'"
    );
    assert!(
        !output_str.contains("loom stage merge"),
        "Knowledge stages should not show merge hint"
    );
}

#[test]
fn test_completed_merged_with_cleanup_warning_shows_failure_and_hint() {
    // A merged stage whose post-merge cleanup failed or was refused should
    // surface the failure and a hint to retry it manually.
    let mut stage = make_stage_summary("my-stage", vec![], StageStatus::Completed);
    stage.merged = true;
    stage.cleanup_warning = Some("failed: git worktree remove refused for x".into());

    let data = make_status_data(vec![stage]);
    let mut output = Vec::new();
    render_graph(&mut output, &data).unwrap();
    let output_str = String::from_utf8(output).unwrap();

    assert!(
        output_str.contains("cleanup failed"),
        "Expected 'cleanup failed' marker in output"
    );
    assert!(
        output_str.contains("loom worktree remove my-stage"),
        "Expected cleanup retry hint in output"
    );
}

#[test]
fn test_orphaned_stage_shows_warning() {
    // A stage whose activity status is Orphaned (claims Executing, no session
    // record) must surface the one-line explanation and both ways out.
    let mut stage = make_stage_summary("my-stage", vec![], StageStatus::Executing);
    stage.activity_status = ActivityStatus::Orphaned;

    let data = make_status_data(vec![stage]);
    let mut output = Vec::new();
    render_graph(&mut output, &data).unwrap();
    let output_str = String::from_utf8(output).unwrap();

    assert!(
        output_str.contains("claims Executing with no session record"),
        "Expected orphaned warning text in output"
    );
    assert!(
        output_str.contains("loom repair"),
        "Expected repair hint in output"
    );
    assert!(
        output_str.contains("loom stage reset --kill-session my-stage"),
        "Expected reset hint in output"
    );
}

#[test]
fn test_non_orphaned_stage_has_no_orphaned_warning() {
    // A normally-executing stage (Working activity status) must not show the
    // orphaned warning text.
    let mut stage = make_stage_summary("my-stage", vec![], StageStatus::Executing);
    stage.activity_status = ActivityStatus::Working;

    let data = make_status_data(vec![stage]);
    let mut output = Vec::new();
    render_graph(&mut output, &data).unwrap();
    let output_str = String::from_utf8(output).unwrap();

    assert!(
        !output_str.contains("claims Executing with no session record"),
        "Should not show orphaned warning when activity status is not Orphaned"
    );
}

#[test]
fn test_executing_with_adjudication_session_shows_type_and_incoherence() {
    // A stage that adopted an adjudication session into its worker slot must
    // surface both that its session is the wrong kind and the incoherence
    // verdict, so the deadlock is visible in `loom status` instead of
    // reading as ordinary progress.
    let mut stage = make_stage_summary("my-stage", vec![], StageStatus::Executing);
    stage.pid = Some(56007);
    stage.session_alive = true;
    stage.session_type = Some(SessionType::Adjudication);
    stage.incoherence = Some(
        "session 'adj-1' is of kind adjudication, not the stage's worker kind stage".to_string(),
    );

    let data = make_status_data(vec![stage]);
    let mut output = Vec::new();
    render_graph(&mut output, &data).unwrap();
    let output_str = String::from_utf8(output).unwrap();

    assert!(
        output_str.contains("adjudication session"),
        "Expected the wrong-kind session type to be surfaced"
    );
    assert!(
        output_str.contains("INCOHERENT:"),
        "Expected the incoherence verdict to be surfaced"
    );
}

#[test]
fn test_completed_merged_without_cleanup_warning_has_no_marker() {
    // A merged stage with no cleanup warning must not show the failure marker.
    let mut stage = make_stage_summary("my-stage", vec![], StageStatus::Completed);
    stage.merged = true;
    stage.cleanup_warning = None;

    let data = make_status_data(vec![stage]);
    let mut output = Vec::new();
    render_graph(&mut output, &data).unwrap();
    let output_str = String::from_utf8(output).unwrap();

    assert!(
        !output_str.contains("cleanup failed"),
        "Should not show 'cleanup failed' marker when there is no cleanup warning"
    );
}
