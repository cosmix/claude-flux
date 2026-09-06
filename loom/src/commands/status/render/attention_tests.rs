use super::*;
use crate::commands::status::data::{ActivityStatus, StageType};
use crate::models::failure::{FailureInfo, FailureType};
use crate::models::stage::StageStatus;
use chrono::Utc;

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

#[test]
fn needs_human_review_renders_the_human_review_hint_and_the_three_choices() {
    let stages = vec![make_stage_summary(
        "integration-verify",
        StageStatus::NeedsHumanReview,
    )];
    let mut output = Vec::new();
    render_attention(&mut output, &stages, false).unwrap();
    let output_str = String::from_utf8(output).unwrap();

    assert!(
        output_str.contains("loom stage human-review integration-verify"),
        "output: {output_str}"
    );
    assert!(output_str.contains("--approve"), "output: {output_str}");
    assert!(
        output_str.contains("queue a fresh session with fresh fix attempts"),
        "output: {output_str}"
    );
    assert!(
        output_str.contains("--force-complete"),
        "output: {output_str}"
    );
    assert!(
        output_str.contains("skip acceptance and mark completed"),
        "output: {output_str}"
    );
    assert!(
        output_str.contains("--reject <reason>"),
        "output: {output_str}"
    );
    assert!(
        output_str.contains("block the stage"),
        "output: {output_str}"
    );
}

#[test]
fn blocked_renders_the_retry_hint_and_no_human_review_choices() {
    let stages = vec![make_stage_summary("stuck-stage", StageStatus::Blocked)];
    let mut output = Vec::new();
    render_attention(&mut output, &stages, false).unwrap();
    let output_str = String::from_utf8(output).unwrap();

    assert!(
        output_str.contains("loom stage retry stuck-stage"),
        "output: {output_str}"
    );
    assert!(
        !output_str.contains("--approve"),
        "a Blocked stage must not show the human-review choices: {output_str}"
    );
    assert!(
        !output_str.contains("--force-complete"),
        "a Blocked stage must not show the human-review choices: {output_str}"
    );
}

#[test]
fn evidence_listing_is_gated_behind_verbose() {
    let mut stage = make_stage_summary("blocked-stage", StageStatus::Blocked);
    stage.failure_info = Some(FailureInfo {
        failure_type: FailureType::TestFailure,
        detected_at: Utc::now(),
        evidence: vec![
            "first evidence line".to_string(),
            "second evidence line".to_string(),
        ],
    });
    let stages = vec![stage];

    let mut quiet = Vec::new();
    render_attention(&mut quiet, &stages, false).unwrap();
    let quiet_str = String::from_utf8(quiet).unwrap();
    assert!(
        !quiet_str.contains("Evidence:"),
        "non-verbose output must not show the Evidence: listing: {quiet_str}"
    );
    assert!(
        !quiet_str.contains("first evidence line"),
        "non-verbose output must not show evidence lines: {quiet_str}"
    );

    let mut verbose = Vec::new();
    render_attention(&mut verbose, &stages, true).unwrap();
    let verbose_str = String::from_utf8(verbose).unwrap();
    assert!(verbose_str.contains("Evidence:"), "output: {verbose_str}");
    assert!(
        verbose_str.contains("first evidence line"),
        "output: {verbose_str}"
    );
    assert!(
        verbose_str.contains("second evidence line"),
        "output: {verbose_str}"
    );
}

#[test]
fn adjudication_reason_includes_the_judge_heartbeat_when_known() {
    let mut stage = make_stage_summary("s-adjudicate", StageStatus::NeedsAdjudication);
    stage.dispute_count = 2;
    stage.judge_heartbeat_secs = Some(42);
    let stages = vec![stage];
    let mut output = Vec::new();
    render_attention(&mut output, &stages, false).unwrap();
    let output_str = String::from_utf8(output).unwrap();

    assert!(
        output_str.contains("2 disputes filed; judge heartbeat 42s ago"),
        "output: {output_str}"
    );
}

#[test]
fn cleanup_warning_renders_as_a_single_line_with_a_stage_file_hint() {
    // `sanitize_stage_summary` always flattens `cleanup_warning` to one line
    // before it reaches a real `AttentionEntry`, but this test bypasses that
    // and feeds an embedded newline directly to prove `render_cleanup_warning`
    // itself no longer special-cases multiple lines - the dead `.lines()`
    // loop this pins the removal of would otherwise indent each line again.
    // The assertion below is split in two because `warning.yellow()` inserts
    // an escape sequence between the label and the text when stdout is a
    // terminal (as in the pre-push hook), and a substring spanning that
    // boundary only matches under a pipe.
    let mut stage = make_stage_summary("cleanup-stage", StageStatus::Completed);
    stage.cleanup_warning = Some("failed: worktree busy\nretrying next cycle".to_string());
    let stages = vec![stage];
    let mut output = Vec::new();
    render_attention(&mut output, &stages, false).unwrap();
    let output_str = String::from_utf8(output).unwrap();

    assert!(
        output_str.contains("Cleanup warning: "),
        "output: {output_str}"
    );
    assert!(
        output_str.contains("failed: worktree busy\nretrying next cycle"),
        "output: {output_str}"
    );
    assert!(
        output_str.contains("full text in the stage file"),
        "output: {output_str}"
    );
}

#[test]
fn adjudication_reason_omits_the_heartbeat_when_unknown() {
    let mut stage = make_stage_summary("s-adjudicate", StageStatus::NeedsAdjudication);
    stage.dispute_count = 2;
    stage.judge_heartbeat_secs = None;
    let stages = vec![stage];
    let mut output = Vec::new();
    render_attention(&mut output, &stages, false).unwrap();
    let output_str = String::from_utf8(output).unwrap();

    assert!(
        output_str.contains("2 disputes filed") && !output_str.contains("judge heartbeat"),
        "output: {output_str}"
    );
}
