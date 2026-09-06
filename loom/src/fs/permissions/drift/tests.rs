use super::*;
use crate::fs::permissions::hooks::{configure_loom_hooks_with_dir, install_loom_hooks_to};
use crate::fs::permissions::settings::ensure_loom_hooks_local;
use std::os::unix::fs::PermissionsExt;

/// Hooks directory used by the drift tests below. Deliberately matches
/// the tilde-prefixed pattern `is_loom_hook` checks for, so a triple
/// built from it is recognized as a loom hook regardless of the actual
/// host home directory the test happens to run under.
const TEST_HOOKS_DIR: &str = "~/.claude/hooks/loom";

/// Whatever the fixer writes, the detector must call clean — otherwise
/// `repair --fix` loops forever reporting the same issue.
#[test]
fn hook_drift_is_empty_for_a_freshly_configured_settings_document() {
    let mut settings_obj = serde_json::Map::new();
    configure_loom_hooks_with_dir(&mut settings_obj, TEST_HOOKS_DIR).unwrap();
    let settings = Value::Object(settings_obj);
    assert!(hook_drift_for_dir(&settings, TEST_HOOKS_DIR).is_empty());
}

#[test]
fn hook_drift_reports_every_registration_when_hooks_are_absent() {
    let canonical_count = flatten_hook_triples(&loom_hooks_config_for_dir(TEST_HOOKS_DIR)).len();
    let drift = hook_drift_for_dir(&json!({}), TEST_HOOKS_DIR);
    assert_eq!(drift.missing.len(), canonical_count);
    assert!(drift.obsolete.is_empty());
}

/// Pins the GLOBAL SessionStart registration for the knowledge-orient nudge:
/// unlike session-start.sh/pre-compact.sh/session-end.sh (written only at
/// worktree-creation time by `crate::hooks::config::HooksConfig`),
/// knowledge-orient.sh must appear in the CANONICAL config every settings
/// document is diffed against, since it is meant to run in every repository
/// and every session, not just loom stage worktrees.
#[test]
fn canonical_config_carries_session_start_knowledge_orient() {
    let canonical = flatten_hook_triples(&loom_hooks_config_for_dir(TEST_HOOKS_DIR));
    assert!(
        canonical.iter().any(|(event, matcher, command)| {
            event == "SessionStart" && matcher == "*" && command.ends_with("/knowledge-orient.sh")
        }),
        "expected a canonical SessionStart:*:.../knowledge-orient.sh triple, got: {canonical:?}"
    );
}

#[test]
fn hook_drift_reports_a_partial_hooks_block() {
    let mut settings_obj = serde_json::Map::new();
    configure_loom_hooks_with_dir(&mut settings_obj, TEST_HOOKS_DIR).unwrap();
    let mut settings = Value::Object(settings_obj);

    // Keep only the Stop event.
    let hooks_obj = settings["hooks"].as_object_mut().unwrap();
    let stop = hooks_obj.remove("Stop");
    hooks_obj.clear();
    if let Some(stop) = stop {
        hooks_obj.insert("Stop".to_string(), stop);
    }

    let drift = hook_drift_for_dir(&settings, TEST_HOOKS_DIR);
    assert!(drift.obsolete.is_empty());
    for prefix in ["PreToolUse:", "PostToolUse:", "UserPromptSubmit:"] {
        assert!(
            drift.missing.iter().any(|m| m.starts_with(prefix)),
            "expected a missing registration for {prefix}"
        );
    }
}

#[test]
fn hook_drift_reports_a_script_loom_no_longer_ships() {
    let mut settings_obj = serde_json::Map::new();
    configure_loom_hooks_with_dir(&mut settings_obj, TEST_HOOKS_DIR).unwrap();
    let mut settings = Value::Object(settings_obj);

    let stop = settings["hooks"]["Stop"].as_array_mut().unwrap();
    stop.push(json!({
        "matcher": "*",
        "hooks": [{"type": "command", "command": format!("{TEST_HOOKS_DIR}/obsolete-ghost.sh")}],
    }));

    let drift = hook_drift_for_dir(&settings, TEST_HOOKS_DIR);
    assert!(drift.missing.is_empty());
    assert!(drift
        .obsolete
        .iter()
        .any(|o| o.contains("obsolete-ghost.sh")));
}

#[test]
fn hook_drift_ignores_a_non_loom_user_hook() {
    let mut settings_obj = serde_json::Map::new();
    configure_loom_hooks_with_dir(&mut settings_obj, TEST_HOOKS_DIR).unwrap();
    let mut settings = Value::Object(settings_obj);

    let stop = settings["hooks"]["Stop"].as_array_mut().unwrap();
    stop.push(json!({
        "matcher": "*",
        "hooks": [{"type": "command", "command": "/usr/local/bin/my-own-hook.sh"}],
    }));

    assert!(hook_drift_for_dir(&settings, TEST_HOOKS_DIR).is_empty());
}

#[test]
fn hook_drift_does_not_flag_worktree_session_hooks() {
    let mut settings_obj = serde_json::Map::new();
    configure_loom_hooks_with_dir(&mut settings_obj, TEST_HOOKS_DIR).unwrap();
    let mut settings = Value::Object(settings_obj);

    // Written by `crate::hooks::config::HooksConfig::to_settings_hooks` at
    // worktree-creation time, not by `configure_loom_hooks`.
    let hooks_obj = settings["hooks"].as_object_mut().unwrap();
    for (event, script) in [
        ("SessionStart", "session-start.sh"),
        ("PreCompact", "pre-compact.sh"),
        ("SessionEnd", "session-end.sh"),
    ] {
        hooks_obj.insert(
            event.to_string(),
            json!([{
                "matcher": "*",
                "hooks": [{"type": "command", "command": format!("{TEST_HOOKS_DIR}/{script}")}],
            }]),
        );
    }
    let stop = hooks_obj.get_mut("Stop").unwrap().as_array_mut().unwrap();
    stop.push(json!({
        "matcher": "*",
        "hooks": [{"type": "command", "command": format!("{TEST_HOOKS_DIR}/learning-validator.sh")}],
    }));

    let drift = hook_drift_for_dir(&settings, TEST_HOOKS_DIR);
    assert!(drift.obsolete.is_empty());
}

#[test]
fn hook_scripts_needing_install_detects_missing_drifted_and_non_executable() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let hooks_dir = temp_dir.path();

    install_loom_hooks_to(hooks_dir).unwrap();
    assert!(hook_scripts_needing_install(hooks_dir).is_empty());

    let (filename, content) = LOOM_HOOKS[0];
    let script_path = hooks_dir.join(filename);

    // Missing entirely.
    fs::remove_file(&script_path).unwrap();
    assert!(hook_scripts_needing_install(hooks_dir).contains(&filename));

    // Restored and executable -> clean again.
    fs::write(&script_path, content).unwrap();
    let mut perms = fs::metadata(&script_path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script_path, perms).unwrap();
    assert!(hook_scripts_needing_install(hooks_dir).is_empty());

    // Content-drifted.
    fs::write(&script_path, "# tampered\n").unwrap();
    assert!(hook_scripts_needing_install(hooks_dir).contains(&filename));

    // Restored content but not executable.
    fs::write(&script_path, content).unwrap();
    let mut perms = fs::metadata(&script_path).unwrap().permissions();
    perms.set_mode(0o644);
    fs::set_permissions(&script_path, perms).unwrap();
    assert!(hook_scripts_needing_install(hooks_dir).contains(&filename));
}

/// A loom-owned event value of the wrong shape is corruption with nothing
/// salvageable in it (Claude Code itself rejects it) — `--fix` must replace
/// it, not bail and leave `repair` looping on the same drift forever.
#[test]
fn configure_loom_hooks_replaces_a_non_array_event_value() {
    let mut settings_obj = serde_json::Map::new();
    settings_obj.insert("hooks".to_string(), json!({"PreToolUse": "not-an-array"}));

    configure_loom_hooks_with_dir(&mut settings_obj, TEST_HOOKS_DIR).unwrap();

    let settings = Value::Object(settings_obj);
    let canonical_pre_tool_use = flatten_hook_triples(&loom_hooks_config_for_dir(TEST_HOOKS_DIR))
        .into_iter()
        .filter(|(event, _, _)| event == "PreToolUse")
        .count();
    let healed_pre_tool_use = settings["hooks"]["PreToolUse"]
        .as_array()
        .expect("PreToolUse must be an array after healing")
        .len();
    assert_eq!(healed_pre_tool_use, canonical_pre_tool_use);
}

#[test]
fn configure_loom_hooks_replaces_a_non_object_hooks_block() {
    let mut settings_obj = serde_json::Map::new();
    settings_obj.insert("hooks".to_string(), json!(42));

    configure_loom_hooks_with_dir(&mut settings_obj, TEST_HOOKS_DIR).unwrap();

    let settings = Value::Object(settings_obj);
    let hooks_obj = settings["hooks"]
        .as_object()
        .expect("hooks must be an object after healing");
    let canonical_events: std::collections::BTreeSet<String> =
        flatten_hook_triples(&loom_hooks_config_for_dir(TEST_HOOKS_DIR))
            .into_iter()
            .map(|(event, _, _)| event)
            .collect();
    for event in canonical_events {
        assert!(hooks_obj.contains_key(&event), "missing event {event}");
    }
}

#[test]
fn ensure_loom_hooks_local_heals_a_non_object_env_and_worktree() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let repo_root = temp_dir.path();
    let claude_dir = repo_root.join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    fs::write(
        claude_dir.join("settings.local.json"),
        json!({"env": "nope", "worktree": []}).to_string(),
    )
    .unwrap();

    ensure_loom_hooks_local(repo_root).unwrap();

    let content = fs::read_to_string(claude_dir.join("settings.local.json")).unwrap();
    let settings: Value = serde_json::from_str(&content).unwrap();
    assert_eq!(settings["env"]["CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS"], "1");
    assert_eq!(settings["worktree"]["bgIsolation"], "none");
}

/// The property the defect violated: whatever `ensure_loom_hooks_local`
/// writes must satisfy `settings_local_hook_drift`, or `repair --fix` loops
/// forever reporting the same issue.
#[test]
fn repair_converges_on_a_malformed_hooks_block() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let repo_root = temp_dir.path();
    let claude_dir = repo_root.join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    fs::write(
        claude_dir.join("settings.local.json"),
        json!({"hooks": {"PreToolUse": "not-an-array"}}).to_string(),
    )
    .unwrap();

    ensure_loom_hooks_local(repo_root).unwrap();

    assert!(settings_local_hook_drift(repo_root).is_empty());
}

/// Out of scope: a whole document of the wrong shape is NOT loom-owned data
/// that can be discarded, unlike the four containers above — healing it
/// would mean silently throwing away the user's entire file, so it must
/// keep erroring.
#[test]
fn ensure_loom_hooks_local_still_errors_on_a_non_object_document() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let repo_root = temp_dir.path();
    let claude_dir = repo_root.join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    fs::write(claude_dir.join("settings.local.json"), "[]").unwrap();

    assert!(ensure_loom_hooks_local(repo_root).is_err());
}
