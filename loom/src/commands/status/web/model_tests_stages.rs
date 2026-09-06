//! Fixture stage builders for [`super::fixture_snapshot`].
use super::*;
use crate::commands::status::data::ActivityStatus;
use crate::models::failure::FailureInfo;
use crate::models::session::{SessionBackendKind, SessionType};
use crate::models::stage::StageType;

macro_rules! stage {
    ($id:expr, $name:expr, $status:expr, $stage_type:expr, $dependencies:expr, $context_tokens:expr, $elapsed_secs:expr, $execution_secs:expr, $failure_info:expr, $activity_status:expr, $last_tool:expr, $last_activity:expr, $staleness_secs:expr, $context_ceiling_tokens:expr, $review_reason:expr, $merged:expr, $cleanup_warning:expr, $held:expr, $retry_count:expr, $max_retries:expr, $pid:expr, $session_alive:expr, $model:expr, $session_type:expr, $incoherence:expr, $execution_models:expr, $dispute_count:expr, $judge_heartbeat_secs:expr, $session_backend:expr) => {
        StageSummary {
            id: $id.to_owned(),
            name: $name.to_owned(),
            status: $status,
            stage_type: $stage_type,
            dependencies: $dependencies
                .iter()
                .map(|dependency| (*dependency).to_owned())
                .collect(),
            context_tokens: $context_tokens,
            elapsed_secs: $elapsed_secs,
            execution_secs: $execution_secs,
            base_branch: None,
            base_merged_from: Vec::new(),
            failure_info: $failure_info,
            activity_status: $activity_status,
            last_tool: $last_tool.map(str::to_owned),
            last_activity: $last_activity.map(str::to_owned),
            staleness_secs: $staleness_secs,
            context_ceiling_tokens: $context_ceiling_tokens,
            review_reason: $review_reason.map(str::to_owned),
            merged: $merged,
            merge_assumed: false,
            cleanup_warning: $cleanup_warning.map(str::to_owned),
            held: $held,
            retry_count: $retry_count,
            max_retries: $max_retries,
            pid: $pid,
            session_alive: $session_alive,
            model: $model.to_owned(),
            session_type: $session_type,
            incoherence: $incoherence.map(str::to_owned),
            execution_models: $execution_models
                .iter()
                .map(|model| (*model).to_owned())
                .collect(),
            dispute_count: $dispute_count,
            judge_heartbeat_secs: $judge_heartbeat_secs,
            session_backend: $session_backend,
        }
    };
}

fn stage_knowledge_bootstrap() -> StageSummary {
    let empty: &[&str] = &[];
    stage!(
        "knowledge-bootstrap",
        "Bootstrap Knowledge",
        StageStatus::Completed,
        StageType::Knowledge,
        empty,
        None,
        Some(412),
        Some(380),
        None,
        ActivityStatus::Idle,
        None,
        None,
        None,
        None,
        None,
        false,
        None,
        false,
        0,
        None,
        None,
        false,
        "opus",
        None,
        None,
        empty,
        0,
        None,
        None
    )
}

fn stage_server() -> StageSummary {
    stage!(
        "server",
        "Rust server",
        StageStatus::Executing,
        StageType::Standard,
        &["knowledge-bootstrap"],
        Some(312000),
        Some(905),
        Some(640),
        None,
        ActivityStatus::Working,
        Some("Bash"),
        Some("cargo test"),
        Some(3),
        Some(800000),
        None,
        false,
        None,
        false,
        0,
        None,
        Some(4242),
        true,
        "opus",
        Some(SessionType::Stage),
        None,
        &["sonnet", "gpt-5.6-terra"],
        0,
        None,
        Some(SessionBackendKind::Native)
    )
}

fn stage_client(detected_at: DateTime<Utc>) -> StageSummary {
    let empty: &[&str] = &[];
    stage!(
        "client",
        "TypeScript client",
        StageStatus::CompletedWithFailures,
        StageType::Standard,
        &["knowledge-bootstrap"],
        None,
        Some(700),
        Some(655),
        Some(FailureInfo {
            failure_type: FailureType::TestFailure,
            detected_at,
            evidence: vec![
                "cargo test failed".to_owned(),
                "1 test failed: schema::parses_fixture".to_owned(),
                "see loom stage retry client".to_owned()
            ]
        }),
        ActivityStatus::Error,
        None,
        None,
        None,
        None,
        None,
        false,
        None,
        false,
        1,
        Some(3),
        None,
        false,
        "opus",
        None,
        None,
        empty,
        0,
        None,
        None
    )
}

fn stage_design() -> StageSummary {
    let empty: &[&str] = &[];
    stage!(
        "design",
        "Visual design",
        StageStatus::WaitingForDeps,
        StageType::Standard,
        &["server", "client"],
        None,
        Some(0),
        None,
        None,
        ActivityStatus::Idle,
        None,
        None,
        None,
        None,
        None,
        false,
        None,
        false,
        0,
        None,
        None,
        false,
        "opus",
        None,
        None,
        empty,
        0,
        None,
        None
    )
}

fn stage_docs() -> StageSummary {
    let empty: &[&str] = &[];
    stage!(
        "docs",
        "Documentation",
        StageStatus::MergeConflict,
        StageType::Standard,
        &["knowledge-bootstrap"],
        None,
        Some(300),
        Some(290),
        None,
        ActivityStatus::Idle,
        None,
        None,
        None,
        None,
        None,
        false,
        None,
        false,
        0,
        None,
        None,
        false,
        "opus",
        None,
        None,
        empty,
        0,
        None,
        None
    )
}

fn stage_integration_verify() -> StageSummary {
    let empty: &[&str] = &[];
    stage!(
        "integration-verify",
        "Integration Verification",
        StageStatus::NeedsHumanReview,
        StageType::IntegrationVerify,
        &["design", "docs"],
        None,
        Some(120),
        None,
        None,
        ActivityStatus::Idle,
        None,
        None,
        None,
        None,
        Some("acceptance criterion 3 disputed twice"),
        false,
        None,
        true,
        0,
        None,
        None,
        false,
        "opus",
        None,
        None,
        empty,
        2,
        Some(40),
        None
    )
}

fn stage_knowledge_distill() -> StageSummary {
    let empty: &[&str] = &[];
    stage!(
        "knowledge-distill",
        "Knowledge Distillation",
        StageStatus::WaitingForDeps,
        StageType::KnowledgeDistill,
        &["integration-verify"],
        None,
        None,
        None,
        None,
        ActivityStatus::Idle,
        None,
        None,
        None,
        None,
        None,
        false,
        None,
        false,
        0,
        None,
        None,
        false,
        "sonnet",
        None,
        None,
        empty,
        0,
        None,
        None
    )
}

/// The stages behind [`super::fixture_snapshot`], in stage order.
pub(super) fn fixture_stages() -> Vec<StageSummary> {
    let detected_at = fixed_time();
    vec![
        stage_knowledge_bootstrap(),
        stage_server(),
        stage_client(detected_at),
        stage_design(),
        stage_docs(),
        stage_integration_verify(),
        stage_knowledge_distill(),
    ]
}
