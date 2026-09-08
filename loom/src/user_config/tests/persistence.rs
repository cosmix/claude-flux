//! Coverage for the file-backed seams: `set_in`, `unset_in`, and the two
//! `UserConfig::load*` entry points. Every test here builds against a temp
//! file path (`set_in`/`unset_in`) or installs a `redirect_user_config`
//! guard (`load`/`load_strict`) — never the real `$HOME` — so the suite
//! stays deterministic under parallel execution.

use super::super::*;
use keys::spec;

#[test]
fn set_in_preserves_comments_and_unknown_keys() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    std::fs::write(
        &path,
        "# a comment worth keeping\n[terminal]\nbackend = \"native\"\nsome_future_key = \"kept\"\n",
    )
    .unwrap();

    set_in(
        &path,
        spec("terminal.backend").unwrap(),
        toml_edit::Value::from("tmux"),
    )
    .unwrap();

    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.contains("# a comment worth keeping"), "{after}");
    assert!(after.contains("some_future_key = \"kept\""), "{after}");
    assert!(after.contains("backend = \"tmux\""), "{after}");
}

#[test]
fn load_over_a_malformed_file_yields_all_defaults_without_panicking() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    std::fs::write(&path, "[update]\ncheck = \"nope\"\n").unwrap();
    let _guard = redirect_user_config(path);

    // Must not panic, and must resolve every key to its built-in default —
    // a broken user config must never take down `loom run`.
    assert_eq!(UserConfig::load(), UserConfig::default());
}

#[test]
fn load_strict_over_a_type_mismatched_file_names_the_config_path() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    std::fs::write(&path, "[update]\ncheck = \"nope\"\n").unwrap();
    let _guard = redirect_user_config(path.clone());

    let err = UserConfig::load_strict().unwrap_err();
    let rendered = format!("{err:?}");
    assert!(
        rendered.contains(&path.display().to_string()),
        "error should name the config file path: {rendered}"
    );
}

#[test]
fn load_strict_over_syntactically_invalid_toml_retains_the_parse_position() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    // A missing closing bracket is a genuine TOML syntax error, not merely a
    // type mismatch, so toml_edit's own parser reports a line/column.
    std::fs::write(&path, "[update\ncheck = true\n").unwrap();
    let _guard = redirect_user_config(path.clone());

    let err = UserConfig::load_strict().unwrap_err();
    let rendered = format!("{err:?}");
    assert!(
        rendered.contains(&path.display().to_string()),
        "error should name the config file path: {rendered}"
    );
    assert!(
        rendered.to_lowercase().contains("line"),
        "error should retain toml_edit's parse position: {rendered}"
    );
}

#[test]
fn set_in_creates_an_absent_section() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    // File does not exist yet - `set_in` creates it and the section.

    set_in(
        &path,
        spec("context.ceiling_tokens").unwrap(),
        toml_edit::Value::from(70000_i64),
    )
    .unwrap();

    let config = parse_document(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(config.context_ceiling_tokens(), 70000);
}

#[test]
fn unset_in_removes_the_key_and_reverts_to_the_built_in() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    set_in(
        &path,
        spec("context.ceiling_tokens").unwrap(),
        toml_edit::Value::from(70000_i64),
    )
    .unwrap();

    let (old, new) = unset_in(&path, spec("context.ceiling_tokens").unwrap()).unwrap();

    assert_eq!(old, "70000");
    assert_eq!(new, DEFAULT_CONTEXT_CEILING_TOKENS.to_string());
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(!after.contains("ceiling_tokens"), "{after}");
    let config = parse_document(&after).unwrap();
    assert_eq!(config.context_ceiling_tokens_set(), None);
}

#[test]
fn unset_in_leaves_comments_and_unrelated_keys() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    std::fs::write(
        &path,
        "# a comment worth keeping\n[terminal]\nbackend = \"tmux\"\nsome_future_key = \"kept\"\n\n[update]\ncheck = false\n",
    )
    .unwrap();

    unset_in(&path, spec("terminal.backend").unwrap()).unwrap();

    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.contains("# a comment worth keeping"), "{after}");
    assert!(after.contains("some_future_key = \"kept\""), "{after}");
    assert!(after.contains("check = false"), "{after}");
    assert!(!after.contains("backend"), "{after}");
}

/// Unlike the workspace config, this file resolves key by key, so an emptied
/// section changes nothing an operator can observe - and removing it would
/// discard the comments attached to its header.
#[test]
fn unset_in_keeps_an_emptied_section() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    std::fs::write(
        &path,
        "# why this section exists\n[update]\ncheck = false\n",
    )
    .unwrap();

    let (old, new) = unset_in(&path, spec("update.check").unwrap()).unwrap();

    assert_eq!(old, "false");
    assert_eq!(new, "true");
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.contains("[update]"), "{after}");
    assert!(after.contains("# why this section exists"), "{after}");
    assert!(!parse_document(&after).unwrap().update_check.is_some());
}

#[test]
fn unset_in_is_a_no_op_for_a_key_that_was_never_set() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    // Neither the file nor the section exists.

    let (old, new) = unset_in(&path, spec("pressure.claude_model").unwrap()).unwrap();

    assert_eq!(old, crate::claude::DEFAULT_PRESSURE_CLAUDE_MODEL);
    assert_eq!(new, crate::claude::DEFAULT_PRESSURE_CLAUDE_MODEL);
    assert!(!std::fs::read_to_string(&path).unwrap().contains("pressure"));
}

#[test]
fn unset_refuses_an_unparseable_file_rather_than_rewriting_it() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.toml");
    std::fs::write(&path, "[update\ncheck = false\n").unwrap();

    let error = unset_in(&path, spec("update.check").unwrap()).unwrap_err();

    assert!(
        format!("{error:?}").contains("refusing to rewrite"),
        "{error:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[update\ncheck = false\n"
    );
}
