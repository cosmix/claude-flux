//! Tests for `loom init`'s terminal-backend prompt (see
//! `backend::{resolve_backend_choice, prompt_backend_choice}`). The prompt
//! reads real stdin, so these exercise the pure logic factored out of it
//! rather than driving a TTY: mapping one line of input to a choice, building
//! the prompt's default clause, and computing the backend that actually
//! applies.

use tempfile::tempdir;

use super::backend::{backend_default_label, effective_backend, parse_backend_response};
use crate::models::session::SessionBackendKind;
use crate::user_config::redirect_user_config;

#[test]
fn init_backend_prompt_empty_input_inherits() {
    assert_eq!(parse_backend_response(""), Some(None));
    assert_eq!(parse_backend_response("   \n"), Some(None));
}

#[test]
fn init_backend_prompt_native_and_tmux_pin() {
    assert_eq!(
        parse_backend_response("native"),
        Some(Some(SessionBackendKind::Native))
    );
    assert_eq!(
        parse_backend_response("tmux"),
        Some(Some(SessionBackendKind::Tmux))
    );
}

#[test]
fn init_backend_prompt_accepts_mixed_case_and_surrounding_whitespace() {
    assert_eq!(
        parse_backend_response("  NaTive\n"),
        Some(Some(SessionBackendKind::Native))
    );
    assert_eq!(
        parse_backend_response(" TMUX \n"),
        Some(Some(SessionBackendKind::Tmux))
    );
}

#[test]
fn init_backend_prompt_rejects_invalid_input() {
    assert_eq!(parse_backend_response("ssh\n"), None);
}

#[test]
fn init_backend_default_label_names_the_set_value_and_its_source() {
    assert_eq!(
        backend_default_label(Some(SessionBackendKind::Tmux)),
        "tmux, from ~/.loom/config.toml"
    );
    assert_eq!(
        backend_default_label(Some(SessionBackendKind::Native)),
        "native, from ~/.loom/config.toml"
    );
}

#[test]
fn init_backend_default_label_says_built_in_default_when_unset() {
    assert_eq!(backend_default_label(None), "native, built-in default");
}

#[test]
fn init_backend_effective_backend_prefers_an_explicit_choice_over_user_config() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("config.toml");
    std::fs::write(&path, "[terminal]\nbackend = \"tmux\"\n").unwrap();
    let _guard = redirect_user_config(path);

    assert_eq!(
        effective_backend(Some(SessionBackendKind::Native)),
        SessionBackendKind::Native
    );
}

#[test]
fn init_backend_effective_backend_inherits_user_config_when_none_chosen() {
    let temp = tempdir().unwrap();
    let path = temp.path().join("config.toml");
    std::fs::write(&path, "[terminal]\nbackend = \"tmux\"\n").unwrap();
    let _guard = redirect_user_config(path);

    assert_eq!(effective_backend(None), SessionBackendKind::Tmux);
}

#[test]
fn init_backend_effective_backend_defaults_to_native_without_user_config() {
    let temp = tempdir().unwrap();
    let _guard = redirect_user_config(temp.path().join("config.toml"));

    assert_eq!(effective_backend(None), SessionBackendKind::Native);
}
