//! Tests for `loom hook pre-compact`.
//!
//! `reset_for_payload` is exercised directly with a literal JSON string
//! rather than through real stdin — the same idiom `tests_user_prompt_e2e.rs`
//! uses for `retrieve_for_prompt`. Tests that resolve a target through the
//! environment mutate process state and are therefore `#[serial]`.

use super::*;
use crate::context::local_overlay::local_overlay_key;
use crate::models::stage::Stage;
use chrono::Utc;
use serial_test::serial;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn payload(session_id: &str) -> String {
    format!(r#"{{"session_id":"{session_id}"}}"#)
}

fn delivered(recipient_id: &str, epoch: &str, ids: &[(&str, &str)]) -> delivery::DeliveryRecord {
    delivery::DeliveryRecord {
        recipient_id: recipient_id.to_string(),
        launch_id: format!("launch-{recipient_id}"),
        context_epoch: epoch.to_string(),
        delivered: ids
            .iter()
            .map(|(id, hash)| delivery::DeliveredNode {
                node_id: (*id).to_string(),
                content_hash: (*hash).to_string(),
            })
            .collect(),
        written_at: Utc::now(),
    }
}

/// A checkout with a `.loom/work/` directory and no stage — an ordinary Claude
/// Code session in a mapped repository.
fn mapped_checkout() -> TempDir {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir_all(temp.path().join(".loom").join("work")).unwrap();
    temp
}

fn enter_checkout(root: &Path) {
    std::env::remove_var("LOOM_STAGE_ID");
    std::env::set_var("LOOM_WORK_DIR", root);
}

fn leave() {
    std::env::remove_var("LOOM_STAGE_ID");
    std::env::remove_var("LOOM_WORK_DIR");
}

/// Fixture for `resets_only_the_named_sessions_own_record_in_checkout_scope`:
/// a mapped checkout, entered as the active environment, with three delivery
/// records already filed under one stage — two ordinary sessions
/// (`session-a`, `session-b`) and one stage spawn record — so the test can
/// reset one and assert the other two survive untouched.
struct ThreeRecipients {
    work_dir: PathBuf,
    plan: String,
    stage_id: String,
    session_a: String,
    session_b: String,
    spawn_recipient: String,
}

fn checkout_with_three_recipients(checkout_root: &Path) -> ThreeRecipients {
    let work_dir = checkout_root.join(".loom").join("work");
    enter_checkout(checkout_root);

    let (plan, stage_id) = local_overlay_key(checkout_root);
    let session_a = delivery::hook_recipient_id(&stage_id, Some("session-a"));
    let session_b = delivery::hook_recipient_id(&stage_id, Some("session-b"));
    let spawn_recipient = "spawn-session-xyz".to_string();

    delivery::record_delivery(
        &work_dir,
        &plan,
        &stage_id,
        &delivered(&session_a, "epoch-a", &[("first", "h1")]),
    )
    .unwrap();
    delivery::record_delivery(
        &work_dir,
        &plan,
        &stage_id,
        &delivered(&session_b, "epoch-a", &[("second", "h2")]),
    )
    .unwrap();
    delivery::record_delivery(
        &work_dir,
        &plan,
        &stage_id,
        &delivered(&spawn_recipient, "epoch-a", &[("spawned", "h3")]),
    )
    .unwrap();

    ThreeRecipients {
        work_dir,
        plan,
        stage_id,
        session_a,
        session_b,
        spawn_recipient,
    }
}

#[test]
fn parse_session_id_extracts_a_trimmed_non_blank_id() {
    assert_eq!(
        parse_session_id(r#"{"session_id":"  abc-123  "}"#).as_deref(),
        Some("abc-123")
    );
}

#[test]
fn parse_session_id_fails_open_on_anything_dishonest() {
    assert!(parse_session_id("").is_none(), "empty stdin");
    assert!(
        parse_session_id("{ not json").is_none(),
        "unparseable payload"
    );
    assert!(parse_session_id("[]").is_none(), "not an object");
    assert!(
        parse_session_id(r#"{"prompt":"hi"}"#).is_none(),
        "no session_id field"
    );
    assert!(
        parse_session_id(r#"{"session_id":"   "}"#).is_none(),
        "whitespace-only id"
    );
    assert!(
        parse_session_id(r#"{"session_id":123}"#).is_none(),
        "a non-string id is not a usable session id"
    );
}

#[test]
fn malformed_or_absent_stdin_resets_nothing() {
    // parse_session_id fails first for all of these, so HookTarget
    // resolution (and the filesystem) are never reached — no env mutation
    // needed, safe to run unserialized alongside every other test.
    reset_for_payload("");
    reset_for_payload("{ not json");
    reset_for_payload("[]");
}

#[test]
fn pre_compact_always_returns_ok() {
    assert!(pre_compact().is_ok());
}

#[test]
#[serial]
fn resets_only_the_named_sessions_own_record_in_checkout_scope() {
    let temp = mapped_checkout();
    let fixture = checkout_with_three_recipients(temp.path());

    reset_for_payload(&payload("session-a"));

    leave();
    let remaining: Vec<String> =
        delivery::load_deliveries(&fixture.work_dir, &fixture.plan, &fixture.stage_id)
            .unwrap()
            .into_iter()
            .map(|record| record.recipient_id)
            .collect();
    assert!(
        !remaining.contains(&fixture.session_a),
        "the compacting session's own record must be gone: {remaining:?}"
    );
    assert!(
        remaining.contains(&fixture.session_b),
        "a sibling session's record must survive untouched"
    );
    assert!(
        remaining.contains(&fixture.spawn_recipient),
        "the spawn record must survive — it is not this session's own record"
    );
}

#[test]
#[serial]
fn resets_a_stage_sessions_record_keyed_to_its_stage() {
    let temp = TempDir::new().unwrap();
    let work_dir = temp.path().join(".work");
    let stage = Stage {
        id: "pre-compact-stage".to_string(),
        name: "Pre Compact Stage".to_string(),
        plan_id: Some("test-plan".to_string()),
        ..Stage::default()
    };
    crate::verify::transitions::create_stage(&stage, &work_dir).unwrap();

    std::env::set_var("LOOM_WORK_DIR", &work_dir);
    std::env::set_var("LOOM_STAGE_ID", &stage.id);

    let recipient = delivery::hook_recipient_id(&stage.id, Some("session-a"));
    delivery::record_delivery(
        &work_dir,
        "test-plan",
        &stage.id,
        &delivered(&recipient, "epoch-a", &[("first", "h1")]),
    )
    .unwrap();

    reset_for_payload(&payload("session-a"));

    leave();
    assert!(
        delivery::load_deliveries(&work_dir, "test-plan", &stage.id)
            .unwrap()
            .is_empty(),
        "the stage session's own record must be gone"
    );
}

#[test]
#[serial]
fn an_environment_naming_no_work_dir_at_all_resets_nothing_and_creates_nothing() {
    let temp = TempDir::new().unwrap();
    // No state directory under this checkout, and no stage — the environment names
    // a valid directory but no delivery scope actually lives there.
    enter_checkout(temp.path());

    reset_for_payload(&payload("session-a"));

    leave();
    assert!(
        !temp.path().join(".loom").join("work").exists(),
        "resetting must never create a state directory tree as a side effect"
    );
}
