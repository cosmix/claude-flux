//! The `POST /api/config` semantics: what each scope writes, what it clears,
//! and every request the route refuses.

use crate::fs::work_dir::read_config;
use crate::models::constants::DEFAULT_CONTEXT_CEILING_TOKENS;
use crate::user_config::keys::spec;
use crate::user_config::UserConfig;

use super::super::update;
use super::super::wire::{ConfigError, ConfigUpdated, Source};
use super::{entry, parse, scratch, scratch_without_workspace, Scratch};

/// Post `body` and return the status with the parsed success payload.
fn post_ok(scratch: &Scratch, body: &str) -> ConfigUpdated {
    let (status, response) = update(&scratch.base, body.as_bytes());
    assert_eq!(status, 200, "{response}");
    serde_json::from_str(&response).expect("response is a ConfigUpdated")
}

/// Post `body` expecting a refusal, and return the status with its message.
fn post_err(scratch: &Scratch, body: &str) -> (u16, String) {
    let (status, response) = update(&scratch.base, body.as_bytes());
    assert_ne!(status, 200, "{response}");
    let error: ConfigError = serde_json::from_str(&response).expect("response is a ConfigError");
    (status, error.error)
}

/// The workspace config as text, for asserting on what a write left behind.
fn project_text(scratch: &Scratch) -> String {
    read_config(&scratch.work())
        .expect("read the workspace config")
        .to_string()
}

#[test]
fn a_user_scope_write_reports_the_pair_and_refreshes_the_entry() {
    let scratch = scratch();
    let updated = post_ok(
        &scratch,
        r#"{"scope":"user","name":"context.ceiling_tokens","value":"640000"}"#,
    );
    assert_eq!(updated.old, DEFAULT_CONTEXT_CEILING_TOKENS.to_string());
    assert_eq!(updated.new, "640000");
    assert!(updated.entry.user.set);
    assert_eq!(updated.entry.effective.source, Source::User);
    assert_eq!(UserConfig::load().context_ceiling_tokens(), 640_000);
}

#[test]
fn a_user_scope_unset_reverts_to_the_built_in() {
    let scratch = scratch();
    post_ok(
        &scratch,
        r#"{"scope":"user","name":"update.check","value":"false"}"#,
    );
    let updated = post_ok(
        &scratch,
        r#"{"scope":"user","name":"update.check","value":null}"#,
    );
    assert_eq!(updated.old, "false");
    assert_eq!(updated.new, "true");
    assert!(!updated.entry.user.set);
    assert_eq!(updated.entry.effective.source, Source::Default);
    assert!(UserConfig::load().update_check());
}

#[test]
fn a_project_scope_write_sets_only_the_key_it_was_given() {
    let scratch = scratch();
    let updated = post_ok(
        &scratch,
        r#"{"scope":"project","name":"context.ceiling_tokens","value":"900000"}"#,
    );
    assert_eq!(updated.new, "900000");
    assert_eq!(updated.entry.effective.source, Source::Project);
    let text = project_text(&scratch);
    assert!(text.contains("ceiling_tokens = 900000"), "{text}");
    // The derived siblings must keep deriving rather than being frozen in.
    assert!(!text.contains("subagent_ceiling_tokens"), "{text}");
    assert!(!text.contains("model_window_tokens"), "{text}");
}

#[test]
fn a_project_scope_unset_removes_the_section_it_empties() {
    let scratch = scratch();
    crate::user_config::set(
        spec("context.ceiling_tokens").unwrap(),
        toml_edit::Value::from(640_000_i64),
    )
    .expect("set the user ceiling");
    post_ok(
        &scratch,
        r#"{"scope":"project","name":"context.ceiling_tokens","value":"900000"}"#,
    );

    let updated = post_ok(
        &scratch,
        r#"{"scope":"project","name":"context.ceiling_tokens","value":null}"#,
    );
    assert_eq!(updated.old, "900000");
    // Emptied, so the section goes and the user tier is genuinely restored -
    // a keyless [context] would still win whole and resolve to the built-in.
    assert!(!project_text(&scratch).contains("[context]"));
    assert_eq!(updated.entry.effective.source, Source::User);
    assert_eq!(updated.entry.effective.value, "640000");
    assert_eq!(
        crate::fs::work_dir::resolve_context_ceiling_tokens(&scratch.work(), None),
        640_000
    );
}

#[test]
fn a_project_scope_unset_keeps_a_section_another_owner_still_uses() {
    let scratch = scratch();
    scratch.write_project(
        "context",
        "prompt_cache_split",
        toml_edit::Value::from(true),
    );
    post_ok(
        &scratch,
        r#"{"scope":"project","name":"context.ceiling_tokens","value":"900000"}"#,
    );

    let updated = post_ok(
        &scratch,
        r#"{"scope":"project","name":"context.ceiling_tokens","value":null}"#,
    );
    let text = project_text(&scratch);
    assert!(text.contains("prompt_cache_split = true"), "{text}");
    assert!(!text.contains("ceiling_tokens"), "{text}");
    // The section survives, so it still wins whole and resolves to the built-in.
    assert_eq!(updated.entry.effective.source, Source::Project);
    assert_eq!(updated.new, DEFAULT_CONTEXT_CEILING_TOKENS.to_string());
}

/// The shape `loom init` actually writes: a plan that sets either ceiling gets
/// BOTH written (`commands::init::plan_setup`), so clearing `ceiling_tokens`
/// leaves `subagent_ceiling_tokens` holding the section open. This is the
/// common workspace, not an edge case, and the response has to say so rather
/// than presenting the unset as a return to the user tier.
#[test]
fn a_project_scope_unset_reports_a_section_its_siblings_keep_alive() {
    let scratch = scratch();
    crate::user_config::set(
        spec("context.ceiling_tokens").unwrap(),
        toml_edit::Value::from(640_000_i64),
    )
    .expect("set the user ceiling");
    scratch.write_project(
        "context",
        "subagent_ceiling_tokens",
        toml_edit::Value::from(500_000_i64),
    );
    post_ok(
        &scratch,
        r#"{"scope":"project","name":"context.ceiling_tokens","value":"300000"}"#,
    );

    let updated = post_ok(
        &scratch,
        r#"{"scope":"project","name":"context.ceiling_tokens","value":null}"#,
    );

    assert_eq!(updated.old, "300000");
    assert!(project_text(&scratch).contains("subagent_ceiling_tokens = 500000"));
    assert_eq!(updated.entry.effective.source, Source::Project);
    assert_eq!(updated.new, DEFAULT_CONTEXT_CEILING_TOKENS.to_string());
    assert_ne!(updated.new, "640000", "the user tier stays shadowed");
    assert_eq!(
        crate::fs::work_dir::resolve_context_ceiling_tokens(&scratch.work(), None),
        DEFAULT_CONTEXT_CEILING_TOKENS
    );
}

#[test]
fn a_project_scope_write_reaches_the_terminal_section_too() {
    let scratch = scratch();
    let updated = post_ok(
        &scratch,
        r#"{"scope":"project","name":"terminal.backend","value":"tmux"}"#,
    );
    assert_eq!(updated.new, "tmux");
    assert_eq!(
        crate::fs::work_dir::read_terminal_config(&scratch.work())
            .expect("resolve the terminal config")
            .backend
            .to_string(),
        "tmux"
    );
    let updated = post_ok(
        &scratch,
        r#"{"scope":"project","name":"terminal.backend","value":null}"#,
    );
    assert_eq!(updated.entry.effective.source, Source::Default);
    assert!(!project_text(&scratch).contains("[terminal]"));
}

#[test]
fn an_invalid_value_returns_the_registrys_own_message() {
    let scratch = scratch();
    let (status, message) = post_err(
        &scratch,
        r#"{"scope":"user","name":"context.ceiling_tokens","value":"abc"}"#,
    );
    assert_eq!(status, 400);
    assert_eq!(
        message,
        r#"context.ceiling_tokens: "abc" is not a u32 (expected a non-negative integer)"#
    );

    let (status, message) = post_err(
        &scratch,
        r#"{"scope":"user","name":"terminal.backend","value":"kitty"}"#,
    );
    assert_eq!(status, 400);
    assert_eq!(
        message,
        r#"terminal.backend: "kitty" is not one of the expected values: native, tmux"#
    );
}

#[test]
fn an_unknown_key_names_the_valid_ones() {
    let scratch = scratch();
    let (status, message) = post_err(
        &scratch,
        r#"{"scope":"user","name":"context.nonsense","value":"1"}"#,
    );
    assert_eq!(status, 400);
    assert!(
        message.starts_with(r#"unknown user config key "context.nonsense""#),
        "{message}"
    );
    assert!(message.contains("update.check"), "{message}");
}

#[test]
fn an_unknown_scope_is_rejected() {
    let scratch = scratch();
    let (status, message) = post_err(
        &scratch,
        r#"{"scope":"global","name":"update.check","value":"false"}"#,
    );
    assert_eq!(status, 400);
    assert_eq!(
        message,
        r#"unknown scope "global"; expected "user" or "project""#
    );
}

#[test]
fn a_user_only_key_is_refused_at_project_scope() {
    let scratch = scratch();
    let (status, message) = post_err(
        &scratch,
        r#"{"scope":"project","name":"update.check","value":"false"}"#,
    );
    assert_eq!(status, 400);
    assert!(
        message.starts_with("update.check: has no project scope"),
        "{message}"
    );
    // Refused before anything reached the file.
    assert!(!project_text(&scratch).contains("update"));
}

#[test]
fn a_project_write_without_a_workspace_is_a_conflict() {
    let scratch = scratch_without_workspace();
    let (status, message) = post_err(
        &scratch,
        r#"{"scope":"project","name":"terminal.backend","value":"tmux"}"#,
    );
    assert_eq!(status, 409);
    assert!(message.contains("loom init"), "{message}");
    // A user-scope write against the same tree still works: the missing
    // workspace bounds the project scope only.
    assert_eq!(
        post_ok(
            &scratch,
            r#"{"scope":"user","name":"terminal.backend","value":"tmux"}"#,
        )
        .new,
        "tmux"
    );
}

#[test]
fn a_malformed_body_is_rejected_without_naming_a_path() {
    let scratch = scratch();
    for body in [
        "",
        "not json",
        r#"{"scope":"user"}"#,
        r#"{"scope":"user","name":"update.check","value":true}"#,
    ] {
        let (status, message) = post_err(&scratch, body);
        assert_eq!(status, 400, "{body}");
        assert!(!message.contains('/'), "{message}");
    }
}

/// A project-scope write on a key-level key (`[models]`) creates the section
/// from nothing and reports the pair the same way the section-level keys do —
/// the write path is shared, only [`super::super::workspace`]'s shadowing
/// rule differs.
#[test]
fn a_project_scope_write_creates_a_key_level_section() {
    let scratch = scratch();
    let updated = post_ok(
        &scratch,
        r#"{"scope":"project","name":"models.standard_effort","value":"low"}"#,
    );
    assert_eq!(updated.old, "high");
    assert_eq!(updated.new, "low");
    assert_eq!(updated.entry.effective.source, Source::Project);
    let text = project_text(&scratch);
    assert!(text.contains("standard_effort = \"low\""), "{text}");
}

/// Unsetting that same key drops it and, since it was the section's only key,
/// the now-empty `[models]` too — the same `remove_key` behavior
/// `context.ceiling_tokens` already exercises — leaving the effective value
/// on the user tier rather than a keyless section winning whole.
#[test]
fn a_project_scope_unset_on_a_key_level_key_restores_the_user_tier() {
    let scratch = scratch();
    crate::user_config::set(
        spec("models.standard_effort").unwrap(),
        toml_edit::Value::from("low"),
    )
    .expect("set the user standard effort");
    post_ok(
        &scratch,
        r#"{"scope":"project","name":"models.standard_effort","value":"xhigh"}"#,
    );

    let updated = post_ok(
        &scratch,
        r#"{"scope":"project","name":"models.standard_effort","value":null}"#,
    );
    assert_eq!(updated.old, "xhigh");
    assert_eq!(updated.new, "low");
    assert_eq!(updated.entry.effective.source, Source::User);
    assert!(!project_text(&scratch).contains("[models]"));
}

#[test]
fn a_write_leaves_the_other_scope_alone() {
    let scratch = scratch();
    post_ok(
        &scratch,
        r#"{"scope":"user","name":"context.ceiling_tokens","value":"640000"}"#,
    );
    post_ok(
        &scratch,
        r#"{"scope":"project","name":"context.ceiling_tokens","value":"900000"}"#,
    );
    let payload = parse(&scratch.base);
    let ceiling = entry(&payload, "context.ceiling_tokens");
    assert_eq!(ceiling.user.value, "640000");
    assert!(ceiling.user.set);
    assert_eq!(ceiling.project.as_ref().unwrap().value, "900000");
    assert_eq!(ceiling.effective.source, Source::Project);
}
