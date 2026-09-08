//! Pure terminal route and resolver invariants.

use chrono::{DateTime, Duration, Utc};

use crate::models::session::{Session, SessionBackendKind, SessionStatus};

use super::resolve::{attach_args, resolve, Refusal, Target};
use super::token;
use super::upgrade::origin_matches_host;
use super::Mode;

fn session(id: &str, stage_id: &str, created_at: DateTime<Utc>) -> Session {
    let mut session = Session::new();
    session.id = id.to_owned();
    session.assign_to_stage(stage_id.to_owned());
    session.backend = SessionBackendKind::Tmux;
    session.status = SessionStatus::Running;
    session.created_at = created_at;
    session
}

#[test]
fn resolve_unknown_stage_refuses_4004() {
    let result = resolve("missing", false, &[], &[], |_, _| true);
    assert_eq!(result, Err(Refusal::UnknownStage));
}

#[test]
fn resolve_known_stage_without_session_refuses_4009() {
    let result = resolve("stage", true, &[], &[], |_, _| true);
    assert_eq!(result, Err(Refusal::NoSession));
}

#[test]
fn resolve_native_backend_refuses_4008() {
    let now = Utc::now();
    let mut native = session("native", "stage", now);
    native.backend = SessionBackendKind::Native;
    let result = resolve("stage", true, &[native], &[], |_, _| true);
    assert_eq!(result, Err(Refusal::NativeBackend));
}

#[test]
fn resolve_rejects_a_corrupt_identifier() {
    let mut live = session("session-a", "stage", Utc::now());
    live.tracking_key = "bad session".to_owned();
    let result = resolve(
        "stage",
        true,
        std::slice::from_ref(&live),
        std::slice::from_ref(&live),
        |_, _| true,
    );
    assert_eq!(result, Err(Refusal::Unrenderable));
}

#[test]
fn resolve_not_ready_refuses_4009() {
    let live = session("session-a", "stage", Utc::now());
    let result = resolve(
        "stage",
        true,
        std::slice::from_ref(&live),
        std::slice::from_ref(&live),
        |_, _| false,
    );
    assert_eq!(result, Err(Refusal::NotReady));
}

#[test]
fn resolve_returns_the_target_for_a_live_tmux_session() {
    let live = session("session-a", "stage", Utc::now());
    let target = resolve(
        "stage",
        true,
        std::slice::from_ref(&live),
        std::slice::from_ref(&live),
        |_, _| true,
    )
    .expect("live tmux target");
    assert_eq!(target.socket, "loom-session-a");
    assert_eq!(target.tmux_session, "loom-stage");
}

#[test]
fn resolve_newest_live_session_wins() {
    let now = Utc::now();
    let older = session("older", "stage", now - Duration::seconds(1));
    let newer = session("newer", "stage", now);
    let all = vec![older.clone(), newer.clone()];
    let target = resolve("stage", true, &all, &[older, newer], |_, _| true).expect("newest target");
    assert_eq!(target.socket, "loom-newer");
}

#[test]
fn resolve_paused_session_is_not_attachable() {
    let mut paused = session("paused", "stage", Utc::now());
    paused.status = SessionStatus::Paused;
    let result = resolve("stage", true, &[paused], &[], |_, _| true);
    assert_eq!(result, Err(Refusal::NoSession));
}

#[test]
fn refusal_close_code_table() {
    let table = [
        (Refusal::UnknownStage, 4004, "no stage with that id"),
        (
            Refusal::NativeBackend,
            4008,
            "this stage's session runs in a native terminal window; terminals need loom run --backend tmux",
        ),
        (
            Refusal::Unrenderable,
            4008,
            "this session's identifier cannot name a tmux session",
        ),
        (Refusal::NoSession, 4009, "no live session for this stage yet"),
        (Refusal::NotReady, 4009, "still spawning, or just ended"),
    ];
    for (refusal, code, reason) in table {
        assert_eq!(refusal.close_code(), code);
        assert_eq!(refusal.reason(), reason);
    }
}

#[test]
fn attach_args_view_mode_is_read_only() {
    let target = Target {
        socket: "loom-session".to_owned(),
        tmux_session: "loom-stage".to_owned(),
    };
    let view = attach_args(&target, Mode::View);
    let control = attach_args(&target, Mode::Control);
    assert!(view
        .windows(2)
        .any(|pair| pair[0] == "-f" && pair[1] == "read-only"));
    assert!(view
        .windows(2)
        .any(|pair| pair[0] == "read-only" && pair[1] == "-t"));
    assert!(!control.iter().any(|arg| arg == "read-only"));
}

#[test]
fn origin_must_match_the_request_host() {
    assert!(!origin_matches_host(None, Some("127.0.0.1:7373")));
    assert!(!origin_matches_host(
        Some("file://127.0.0.1:7373"),
        Some("127.0.0.1:7373")
    ));
    assert!(!origin_matches_host(
        Some("http://127.0.0.1:7374"),
        Some("127.0.0.1:7373")
    ));
    assert!(!origin_matches_host(
        Some("http://localhost:7373"),
        Some("127.0.0.1:7373")
    ));
    assert!(origin_matches_host(
        Some("https://127.0.0.1:7373"),
        Some("127.0.0.1:7373")
    ));
}

#[test]
fn cookie_name_is_scoped_to_the_bound_port() {
    assert_eq!(token::cookie_name(7373), "loom_dashboard_7373");
}
