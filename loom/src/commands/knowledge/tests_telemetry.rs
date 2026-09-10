use super::super::tests::setup_test_env;
use super::*;
use crate::context::{Channel, Freshness, OmissionSummary};
use crate::telemetry::{emit, TelemetryEvent};
use serial_test::serial;

#[test]
fn telemetry_command_prints_no_telemetry_recorded_without_a_work_dir() {
    let temp = tempfile::tempdir().unwrap();
    let work_dir = temp.path().join(".loom").join("work");
    let mut output = Vec::new();

    render_telemetry(&work_dir, None, false, &mut output).unwrap();

    assert_eq!(
        String::from_utf8(output).unwrap(),
        "no telemetry recorded\n"
    );
    assert!(!temp.path().join(".loom").exists());
}

#[test]
fn telemetry_command_prints_the_documented_json_shape() {
    let work_dir = tempfile::tempdir().unwrap();
    emit(
        work_dir.path(),
        &TelemetryEvent::ContextPulled {
            stage_id: Some("stage-a".to_string()),
            session_id: Some("session-1".to_string()),
            query_chars: 20,
            budget_tokens: 800,
            items: 4,
            estimated_tokens: 500,
            unmet_required: 1,
        },
    )
    .unwrap();
    let mut output = Vec::new();

    render_telemetry(work_dir.path(), Some("stage-a"), true, &mut output).unwrap();

    let payload: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let stages = payload["stages"].as_array().unwrap();
    assert_eq!(stages.len(), 1);
    assert!(stages[0]["last_at"].as_str().is_some());
    assert_eq!(
        stages[0],
        serde_json::json!({
            "stage_id": "stage-a",
            "spawn_briefs": 0,
            "spawn_items": 0,
            "prompt_briefs": 0,
            "prompt_abstained": { "total": 0, "by_reason": {} },
            "pulls": 1,
            "pull_avg_budget": 800,
            "pull_avg_items": 4,
            "pull_unmet": 1,
            "last_at": stages[0]["last_at"].clone(),
        })
    );
}

#[test]
#[serial]
fn context_pull_without_a_work_dir_creates_no_state_dir() {
    let (_temp_dir, test_dir) = setup_test_env();
    let original_dir = std::env::current_dir().expect("Failed to get current dir");
    std::env::set_current_dir(&test_dir).expect("Failed to change dir");

    let pack = ContextPack {
        query: "query".to_string(),
        scope: vec![Channel::Knowledge],
        budget_tokens: 100,
        estimated_tokens: 0,
        structural_freshness: Freshness::default(),
        semantic_freshness: Freshness::default(),
        items: Vec::new(),
        unmet_required: Vec::new(),
        omitted: OmissionSummary::default(),
        dropped_terms: Vec::new(),
        degraded: None,
    };
    emit_context_pulled(&None, 12, 100, &pack);

    std::env::set_current_dir(original_dir).expect("Failed to restore dir");
    assert!(
        !test_dir.join(".loom").exists(),
        "a read-only pull in an uninitialized checkout must not create a state dir"
    );
}
