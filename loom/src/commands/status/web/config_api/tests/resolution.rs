//! How `effective.value` resolves across the three config sources (default,
//! user, project) — the property that keeps the settings page from
//! disagreeing with the daemon about the value actually in force.

use crate::fs::work_dir::{read_terminal_config, resolve_context_ceiling_tokens};
use crate::models::constants::DEFAULT_CONTEXT_CEILING_TOKENS;
use crate::user_config::keys::spec;

use super::super::wire::Source;
use super::{entry, parse, scratch, Scratch};

/// The projection must agree with the functions loom itself resolves through.
/// A settings page that disagrees with the daemon about the value in force is
/// worse than none. Shared by the three tests below, one per source, so a
/// failure names which tier disagreed.
fn assert_effective_agrees_with_resolution(scratch: &Scratch) {
    let payload = parse(&scratch.base);
    let work = scratch.work();
    assert_eq!(
        entry(&payload, "terminal.backend").effective.value,
        read_terminal_config(&work)
            .expect("resolve the terminal config")
            .backend
            .to_string()
    );
    assert_eq!(
        entry(&payload, "context.ceiling_tokens").effective.value,
        resolve_context_ceiling_tokens(&work, None).to_string()
    );
}

/// Source: default. Neither workspace-backed key is set anywhere.
#[test]
fn effective_value_for_default_source_agrees_with_resolution() {
    let scratch = scratch();
    assert_effective_agrees_with_resolution(&scratch);
}

/// Source: user. Both workspace-backed keys set in the user config only.
#[test]
fn effective_value_for_user_source_agrees_with_resolution() {
    let scratch = scratch();
    crate::user_config::set(
        spec("context.ceiling_tokens").unwrap(),
        toml_edit::Value::from(640_000_i64),
    )
    .expect("set the user ceiling");
    crate::user_config::set(
        spec("terminal.backend").unwrap(),
        toml_edit::Value::from("tmux"),
    )
    .expect("set the user backend");

    let payload = parse(&scratch.base);
    assert_eq!(
        entry(&payload, "terminal.backend").effective.source,
        Source::User
    );
    assert_eq!(
        entry(&payload, "context.ceiling_tokens").user.value,
        "640000"
    );
    assert_effective_agrees_with_resolution(&scratch);
}

/// Source: project, shadowing user values set on the same keys.
#[test]
fn effective_value_for_project_source_agrees_with_resolution() {
    let scratch = scratch();
    crate::user_config::set(
        spec("context.ceiling_tokens").unwrap(),
        toml_edit::Value::from(640_000_i64),
    )
    .expect("set the user ceiling");
    crate::user_config::set(
        spec("terminal.backend").unwrap(),
        toml_edit::Value::from("tmux"),
    )
    .expect("set the user backend");

    scratch.write_project(
        "context",
        "ceiling_tokens",
        toml_edit::Value::from(900_000_i64),
    );
    scratch.write_project("terminal", "backend", toml_edit::Value::from("native"));

    let payload = parse(&scratch.base);
    for name in ["terminal.backend", "context.ceiling_tokens"] {
        assert_eq!(
            entry(&payload, name).effective.source,
            Source::Project,
            "{name}"
        );
        assert!(
            entry(&payload, name).project.as_ref().unwrap().set,
            "{name}"
        );
    }
    assert_effective_agrees_with_resolution(&scratch);
}

/// A `[context]` holding only `prompt_cache_split` sets no registry key, yet
/// still wins the whole section — the fallback loom actually implements. The
/// projection has to report that, not the user tier it shadows.
#[test]
fn a_present_but_keyless_section_still_wins_whole() {
    let scratch = scratch();
    crate::user_config::set(
        spec("context.ceiling_tokens").unwrap(),
        toml_edit::Value::from(640_000_i64),
    )
    .expect("set the user ceiling");
    scratch.write_project(
        "context",
        "prompt_cache_split",
        toml_edit::Value::from(true),
    );

    let payload = parse(&scratch.base);
    let ceiling = entry(&payload, "context.ceiling_tokens");
    assert_eq!(ceiling.effective.source, Source::Project);
    assert_eq!(
        ceiling.effective.value,
        DEFAULT_CONTEXT_CEILING_TOKENS.to_string()
    );
    let project = ceiling.project.as_ref().expect("project scope");
    assert!(!project.set, "the section sets no ceiling of its own");
    assert_eq!(project.value, DEFAULT_CONTEXT_CEILING_TOKENS.to_string());
    assert_eq!(
        ceiling.user.value, "640000",
        "the user tier is shadowed, not gone"
    );
    assert_eq!(
        ceiling.effective.value,
        resolve_context_ceiling_tokens(&scratch.work(), None).to_string()
    );
}
