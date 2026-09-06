use super::tests::{make_test_stage, temp_work_dir};
use super::*;
use crate::commands::status::data::ActivityStatus;
use crate::models::constants::DEFAULT_CONTEXT_CEILING_TOKENS;
use crate::models::session::SessionStatus;

#[test]
fn test_build_stage_summary_with_session() {
    let (_tmp, work_dir) = temp_work_dir();

    let mut stage = make_test_stage("test-stage", StageStatus::Executing);
    stage.dependencies = vec!["dep-1".to_string()];
    stage.started_at = Some(Utc::now());
    let mut session = Session::new();
    session.assign_to_stage("test-stage".to_string());
    session.context_tokens = 50000;

    let summary = build_stage_summary(&stage, &[session], &work_dir);

    assert_eq!(summary.id, "test-stage");
    assert_eq!(summary.status, StageStatus::Executing);
    assert_eq!(summary.dependencies, vec!["dep-1"]);
    assert_eq!(summary.context_tokens, Some(50_000));
    // The stage declares no ceiling, so the summary shows the built-in
    // default — asserted through the constant, not through its current value.
    assert_eq!(
        summary.context_ceiling_tokens,
        Some(DEFAULT_CONTEXT_CEILING_TOKENS)
    );
    assert!(summary.elapsed_secs.is_some());
    // New fields
    assert_eq!(summary.activity_status, ActivityStatus::Working);
    assert!(summary.staleness_secs.is_none()); // No heartbeat file
}

#[test]
fn test_build_stage_summary_without_session() {
    let (_tmp, work_dir) = temp_work_dir();

    let stage = make_test_stage("test-stage", StageStatus::WaitingForDeps);

    let summary = build_stage_summary(&stage, &[], &work_dir);

    assert_eq!(summary.id, "test-stage");
    assert_eq!(summary.status, StageStatus::WaitingForDeps);
    assert!(summary.dependencies.is_empty());
    assert_eq!(summary.context_tokens, None);
    // An unstarted stage has no elapsed time.
    assert!(summary.elapsed_secs.is_none());
    // New fields
    assert_eq!(summary.activity_status, ActivityStatus::Idle);
}

#[test]
fn judge_heartbeat_secs_reads_adjudication_file() {
    let temp = tempfile::tempdir().unwrap();
    let work_dir = WorkDir::new(temp.path().join(".loom/work")).unwrap();
    let stage = make_test_stage("judge-stage", StageStatus::Executing);
    let heartbeat_path =
        crate::orchestrator::monitor::heartbeat::judge_heartbeat_path(work_dir.root(), &stage.id);
    std::fs::create_dir_all(heartbeat_path.parent().unwrap()).unwrap();
    let heartbeat = crate::orchestrator::monitor::heartbeat::Heartbeat {
        stage_id: stage.id.clone(),
        session_id: "judge-session".to_string(),
        timestamp: Utc::now(),
        context_tokens: None,
        transcript_path: None,
        last_tool: None,
        activity: None,
    };
    std::fs::write(heartbeat_path, serde_json::to_string(&heartbeat).unwrap()).unwrap();
    let summary = build_stage_summary(&stage, &[], &work_dir);
    assert!(matches!(summary.judge_heartbeat_secs, Some(seconds) if seconds <= 5));
    let missing = build_stage_summary(
        &make_test_stage("no-judge", StageStatus::Executing),
        &[],
        &work_dir,
    );
    assert_eq!(missing.judge_heartbeat_secs, None);
}
#[test]
fn stage_summary_reads_the_stages_own_session_not_a_corpse() {
    // A retried stage leaves every previous session file on disk with
    // `stage_id` still set. Picking the first match in `read_dir` order (as
    // the old code did) can surface a crashed corpse's frozen token count
    // rendered against the live stage's ceiling - a lie the dashboard would
    // tell every retried stage. The stage's own `session` claim must win.
    let (_tmp, work_dir) = temp_work_dir();

    let mut stage = make_test_stage("test-stage", StageStatus::Executing);
    stage.session = Some("live-session".to_string());

    let mut corpse = Session::new();
    corpse.id = "dead-session".to_string();
    corpse.stage_id = Some("test-stage".to_string());
    corpse.status = SessionStatus::Crashed;
    corpse.context_tokens = 200_000;

    let mut live = Session::new();
    live.id = "live-session".to_string();
    live.stage_id = Some("test-stage".to_string());
    live.status = SessionStatus::Running;
    live.context_tokens = 10_000;

    // Corpse first, so the old `find`-first-match logic would pick it.
    let sessions = vec![corpse, live];

    let summary = build_stage_summary(&stage, &sessions, &work_dir);

    assert_eq!(summary.context_tokens, Some(10_000));
}

#[test]
fn stage_summary_hides_a_session_that_has_not_reported_a_reading() {
    // A freshly spawned agent has not sent a heartbeat with a context reading
    // yet. Rendering `0 / 150000` is a confident lie; the column should be
    // blank instead, while the stage still shows as actively worked.
    let (_tmp, work_dir) = temp_work_dir();

    let mut stage = make_test_stage("test-stage", StageStatus::Executing);
    stage.session = Some("live-session".to_string());

    let mut live = Session::new();
    live.id = "live-session".to_string();
    live.stage_id = Some("test-stage".to_string());
    live.status = SessionStatus::Running;
    live.context_tokens = 0;

    let summary = build_stage_summary(&stage, &[live], &work_dir);

    assert_eq!(summary.context_tokens, None);
    assert_eq!(summary.context_ceiling_tokens, None);
    assert_eq!(summary.activity_status, ActivityStatus::Working);
}

#[test]
fn stage_summary_ignores_a_named_session_that_belongs_to_another_stage() {
    // `stage.session` is a claim, not proof. A session id repeated or reused
    // across stages would otherwise let another stage's agent report its
    // tokens here - the same wrong-row attribution the corpse case makes, with
    // a live session doing the lying. The named session must also name this
    // stage back.
    let (_tmp, work_dir) = temp_work_dir();

    let mut stage = make_test_stage("test-stage", StageStatus::Executing);
    stage.session = Some("shared-id".to_string());

    let mut elsewhere = Session::new();
    elsewhere.id = "shared-id".to_string();
    elsewhere.stage_id = Some("other-stage".to_string());
    elsewhere.status = SessionStatus::Running;
    elsewhere.context_tokens = 200_000;

    let summary = build_stage_summary(&stage, &[elsewhere], &work_dir);

    assert_eq!(
        summary.context_tokens, None,
        "a session executing another stage must not report tokens here"
    );
    assert_eq!(summary.activity_status, ActivityStatus::Orphaned);
}

#[test]
fn stage_summary_reports_a_crashed_only_session_without_its_frozen_reading() {
    // A stage whose only session crashed has to render as `Error`: it is a
    // stage with a dead agent, not a stage the daemon lost track of, and
    // narrowing the pick to live sessions alone would silently downgrade it to
    // `Orphaned`. The corpse still speaks for the ACTIVITY - but not for the
    // reading, which stopped tracking the stage when the agent died.
    let (_tmp, work_dir) = temp_work_dir();

    let stage = make_test_stage("test-stage", StageStatus::Executing);

    let mut corpse = Session::new();
    corpse.id = "dead-session".to_string();
    corpse.stage_id = Some("test-stage".to_string());
    corpse.status = SessionStatus::Crashed;
    corpse.context_tokens = 120_000;

    let summary = build_stage_summary(&stage, &[corpse], &work_dir);

    assert_eq!(summary.activity_status, ActivityStatus::Error);
    assert_eq!(
        summary.context_tokens, None,
        "a dead agent's frozen count must not render against the live ceiling"
    );
    assert_eq!(summary.context_ceiling_tokens, None);
}

#[test]
fn stage_summary_reports_idle_for_a_completed_stage_with_a_running_session_record() {
    // A finished stage has no agent, whatever its last session file says - a
    // recent heartbeat on a stale Running record must not read as `Working`.
    let (_tmp, work_dir) = temp_work_dir();

    let stage = make_test_stage("test-stage", StageStatus::Completed);

    let mut session = Session::new();
    session.id = "leftover-session".to_string();
    session.stage_id = Some("test-stage".to_string());
    session.status = SessionStatus::Running;
    session.context_tokens = 50_000;

    let summary = build_stage_summary(&stage, &[session], &work_dir);

    assert_eq!(summary.activity_status, ActivityStatus::Idle);
}

#[test]
fn stage_summary_reports_idle_when_the_only_session_ended_without_crashing() {
    // A session that completed normally is neither working nor hung, even if
    // the stage itself still claims `Executing` (e.g. a retry spawned a fresh
    // session that has not yet been picked up as `stage.session`).
    let (_tmp, work_dir) = temp_work_dir();

    let stage = make_test_stage("test-stage", StageStatus::Executing);

    let mut session = Session::new();
    session.id = "finished-session".to_string();
    session.stage_id = Some("test-stage".to_string());
    session.status = SessionStatus::Completed;
    session.context_tokens = 10_000;

    let summary = build_stage_summary(&stage, &[session], &work_dir);

    assert_eq!(summary.activity_status, ActivityStatus::Idle);
}

#[test]
fn stage_summary_still_reports_error_for_a_blocked_stage_with_a_crashed_session() {
    // Guards the match-arm ordering: `Crashed` must be judged before the
    // finished-stage and terminal-session arms, or a blocked stage with a
    // dead agent would silently downgrade to `Idle`.
    let (_tmp, work_dir) = temp_work_dir();

    let stage = make_test_stage("test-stage", StageStatus::Blocked);

    let mut session = Session::new();
    session.id = "dead-session".to_string();
    session.stage_id = Some("test-stage".to_string());
    session.status = SessionStatus::Crashed;
    session.context_tokens = 120_000;

    let summary = build_stage_summary(&stage, &[session], &work_dir);

    assert_eq!(summary.activity_status, ActivityStatus::Error);
}

#[test]
fn test_build_stage_summary_orphaned_when_executing_without_session() {
    let (_tmp, work_dir) = temp_work_dir();

    // Stage claims Executing but no session names it - a killed daemon
    // or lost session file, not a quiet stage.
    let stage = make_test_stage("orphan-stage", StageStatus::Executing);

    let summary = build_stage_summary(&stage, &[], &work_dir);

    assert_eq!(summary.activity_status, ActivityStatus::Orphaned);
}

#[test]
fn test_build_stage_summary_flags_an_adjudication_session_adopted_as_worker() {
    // The bug this pins: a stage's own `session` pointer can end up naming an
    // adjudication session instead of its worker. The summary must surface
    // both the wrong session kind and the incoherence verdict so the
    // deadlock is visible in `loom status`.
    let (_tmp, work_dir) = temp_work_dir();

    let mut stage = make_test_stage("test-stage", StageStatus::Executing);
    let adjudication = Session::new_adjudication("test-stage");
    stage.session = Some(adjudication.id.clone());

    let summary = build_stage_summary(&stage, &[adjudication], &work_dir);

    assert_eq!(summary.session_type, Some(SessionType::Adjudication));
    assert!(summary.incoherence.is_some());
}
