use super::*;
use crate::commands::status::data::{
    ActivityStatus, MergeSummary, ProgressSummary, StageSummary, StageType, StatusData,
};
use crate::daemon::protocol::{CompletionSummary, DaemonConfig, StageCompletionInfo};
use crate::models::session::SessionBackendKind;
use crate::models::stage::StageStatus;
use std::io::{Cursor, Read};

struct CountingReader<R> {
    inner: R,
    bytes_read: usize,
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.inner.read(buffer)?;
        self.bytes_read += count;
        Ok(count)
    }
}

#[test]
fn request_round_trip_includes_preface() {
    let mut buffer = Vec::new();
    let request = Request::Ping {
        auth_token: "test-token".to_string(),
    };
    write_message(&mut buffer, &request).unwrap();

    let decoded: Request = read_message(&mut Cursor::new(buffer)).unwrap();

    assert!(matches!(decoded, Request::Ping { auth_token } if auth_token == "test-token"));
}

#[test]
fn response_round_trip_uses_bounded_frame() {
    let mut buffer = Vec::new();
    write_message(&mut buffer, &Response::Pong).unwrap();

    let decoded: Response = read_message(&mut Cursor::new(buffer)).unwrap();

    assert!(matches!(decoded, Response::Pong));
}

#[test]
fn status_update_round_trip() {
    let response = Response::StatusUpdate {
        data: Box::new(status_data()),
    };
    let mut buffer = Vec::new();
    write_message(&mut buffer, &response).unwrap();

    let decoded: Response = read_message(&mut Cursor::new(buffer)).unwrap();

    match decoded {
        Response::StatusUpdate { data } => {
            assert_eq!(data.stages[0].id, "stage-1");
            assert_eq!(data.stages[0].status, StageStatus::Executing);
            // The four fields the ledger dashboard added. Asserted with
            // non-default values: at their defaults a field that never
            // reached the wire at all would decode identically.
            assert_eq!(data.stages[0].execution_models, ["opus", "terra"]);
            assert_eq!(data.stages[0].dispute_count, 3);
            assert_eq!(data.stages[0].judge_heartbeat_secs, Some(42));
            assert_eq!(
                data.stages[0].session_backend,
                Some(SessionBackendKind::Tmux)
            );
            assert_eq!(data.stages[1].id, "stage-2");
            assert_eq!(data.stages[1].status, StageStatus::WaitingForDeps);
        }
        _ => panic!("expected status update"),
    }
}

fn stage_summary(id: &str, status: StageStatus) -> StageSummary {
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
        activity_status: ActivityStatus::Idle,
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
        model: "test".to_string(),
        session_type: None,
        incoherence: None,
        execution_models: vec![],
        dispute_count: 0,
        judge_heartbeat_secs: None,
        session_backend: None,
    }
}

fn status_data() -> StatusData {
    let mut executing = stage_summary("stage-1", StageStatus::Executing);
    executing.execution_models = vec!["opus".to_string(), "terra".to_string()];
    executing.dispute_count = 3;
    executing.judge_heartbeat_secs = Some(42);
    executing.session_backend = Some(SessionBackendKind::Tmux);

    StatusData {
        stages: vec![
            executing,
            stage_summary("stage-2", StageStatus::WaitingForDeps),
        ],
        merge: MergeSummary::default(),
        progress: ProgressSummary {
            total: 2,
            executing: 1,
            pending: 1,
            ..Default::default()
        },
        plan_name: Some("Test plan".to_string()),
        quota: crate::quota::QuotaSnapshot::default(),
    }
}

#[test]
fn invalid_preface_is_rejected_before_huge_frame_length_is_read() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"NOPE");
    bytes.extend_from_slice(&[REQUEST_PREFACE_VERSION, 1, 0, 1]);
    bytes.push(b'x');
    bytes.extend_from_slice(&u32::MAX.to_be_bytes());
    let mut reader = CountingReader {
        inner: Cursor::new(bytes),
        bytes_read: 0,
    };

    let error = read_request_preface(&mut reader).unwrap_err();

    assert!(error.to_string().contains("Invalid request"));
    assert_eq!(reader.bytes_read, REQUEST_PREFACE_HEADER_BYTES);
}

#[test]
fn oversized_preface_credential_does_not_read_credential_or_frame() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&REQUEST_PREFACE_MAGIC);
    bytes.extend_from_slice(&[
        REQUEST_PREFACE_VERSION,
        1,
        ((MAX_CREDENTIAL_BYTES + 1) >> 8) as u8,
        (MAX_CREDENTIAL_BYTES + 1) as u8,
    ]);
    bytes.resize(
        REQUEST_PREFACE_HEADER_BYTES + MAX_CREDENTIAL_BYTES + 1,
        b'x',
    );
    bytes.extend_from_slice(&u32::MAX.to_be_bytes());
    let mut reader = CountingReader {
        inner: Cursor::new(bytes),
        bytes_read: 0,
    };

    let error = read_request_preface(&mut reader).unwrap_err();

    assert!(error.to_string().contains("credential length"));
    assert_eq!(reader.bytes_read, REQUEST_PREFACE_HEADER_BYTES);
}

#[test]
fn oversized_request_body_length_is_rejected_before_allocation() {
    let mut buffer = Vec::new();
    write_request_preface(&mut buffer, Capability::User, "token").unwrap();
    buffer.extend_from_slice(&((MAX_REQUEST_BYTES + 1) as u32).to_be_bytes());
    let mut cursor = Cursor::new(buffer);

    let _preface = read_request_preface(&mut cursor).unwrap();
    let error = read_request_length(&mut cursor).unwrap_err();

    assert!(error.to_string().contains("configured limit"));
}

#[test]
fn oversized_credential_and_frames_are_rejected_on_write() {
    let oversized_credential = Request::Ping {
        auth_token: "x".repeat(MAX_CREDENTIAL_BYTES + 1),
    };
    assert!(write_message(&mut Vec::new(), &oversized_credential).is_err());

    let oversized_request = Request::DisputeCriteria {
        auth_token: "token".to_string(),
        stage_id: "stage".to_string(),
        session_id: "session-1".to_string(),
        criterion_index: 0,
        reason: "x".repeat(MAX_REQUEST_BYTES),
        evidence_commit: None,
        failure_output: None,
    };
    assert!(write_message(&mut Vec::new(), &oversized_request).is_err());

    let oversized_response = Response::LogLine {
        line: "x".repeat(MAX_RESPONSE_BYTES),
    };
    assert!(write_message(&mut Vec::new(), &oversized_response).is_err());
}

#[test]
fn request_debug_output_redacts_credentials_and_evidence() {
    let request = Request::DisputeCriteria {
        auth_token: "proof-secret".to_string(),
        stage_id: "stage".to_string(),
        session_id: "session-1".to_string(),
        criterion_index: 0,
        reason: "private reason".to_string(),
        evidence_commit: None,
        failure_output: Some("private output".to_string()),
    };

    let output = format!("{request:?}");
    assert!(!output.contains("proof-secret"));
    assert!(!output.contains("private reason"));
    assert!(!output.contains("private output"));
    assert!(output.contains("[REDACTED]"));
    assert!(output.contains("session-1"), "ids stay legible: {output}");
}

/// A block request carries a free-text reason and a credential; only the ids
/// belong in a daemon log line.
#[test]
fn block_stage_debug_output_redacts_credential_and_reason() {
    let request = Request::BlockStage {
        auth_token: "user-secret".to_string(),
        stage_id: "stage".to_string(),
        session_id: "session-1".to_string(),
        reason: "private reason".to_string(),
    };

    let output = format!("{request:?}");
    assert!(!output.contains("user-secret"));
    assert!(!output.contains("private reason"));
    assert!(output.contains("stage"));
    assert!(output.contains("session-1"));
}

#[test]
fn daemon_config_default_is_safe() {
    let config = DaemonConfig::default();

    assert!(!config.manual_mode);
    assert!(config.max_parallel.is_none());
    assert!(config.watch_mode);
    assert!(config.auto_merge);
}

#[test]
fn orchestration_complete_round_trip() {
    let response = Response::OrchestrationComplete {
        summary: CompletionSummary {
            total_duration_secs: 120,
            stages: vec![StageCompletionInfo {
                id: "stage-1".to_string(),
                name: "First Stage".to_string(),
                status: StageStatus::Completed,
                duration_secs: Some(60),
                execution_secs: None,
                retry_count: 0,
                merged: true,
                dependencies: vec![],
            }],
            success_count: 1,
            failure_count: 0,
            plan_path: "doc/plans/PLAN-test.md".to_string(),
        },
    };
    let mut buffer = Vec::new();
    write_message(&mut buffer, &response).unwrap();

    let decoded: Response = read_message(&mut Cursor::new(buffer)).unwrap();

    match decoded {
        Response::OrchestrationComplete { summary } => {
            assert_eq!(summary.total_duration_secs, 120);
            assert_eq!(summary.stages[0].id, "stage-1");
            assert_eq!(summary.plan_path, "doc/plans/PLAN-test.md");
        }
        _ => panic!("expected orchestration complete"),
    }
}
