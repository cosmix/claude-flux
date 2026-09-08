//! Resolving the terminal backend for `loom init`.
//!
//! Precedence: an explicit `--backend` flag pins the choice. On an
//! interactive terminal, a typed answer also pins it, while a plain Enter
//! inherits instead - the workspace `[terminal]` section is left absent so
//! `~/.loom/config.toml` decides at read time. A non-interactive run (no
//! TTY) inherits the same way, with no prompt shown at all.

use crate::models::session::SessionBackendKind;
use anyhow::{bail, Result};
use colored::Colorize;
use std::io::{IsTerminal, Write};

/// Resolve the terminal backend choice for `loom init`.
///
/// Precedence: an explicit `--backend` flag always wins (clap's
/// `value_parser` already constrains it to "native"/"tmux"), yielding
/// `Some(kind)`. Otherwise, on an interactive terminal, prompt the operator:
/// a typed answer yields `Some(kind)`, while a plain Enter (or EOF) yields
/// `None`, the same as what a non-interactive run yields with no prompt at
/// all. `None` means nobody chose a backend, so `plan_setup` leaves the
/// workspace `[terminal]` section out of `config.toml` entirely, letting
/// `~/.loom/config.toml`'s `terminal.backend` — then the built-in default —
/// decide at read time (see `fs::work_dir::read_terminal_config`).
pub(super) fn resolve_backend_choice(flag: Option<String>) -> Result<Option<SessionBackendKind>> {
    let kind = if let Some(value) = flag {
        Some(match value.as_str() {
            "native" => SessionBackendKind::Native,
            "tmux" => SessionBackendKind::Tmux,
            other => bail!("Invalid terminal backend: {other}"),
        })
    } else if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        prompt_backend_choice()?
    } else {
        None
    };

    let effective = effective_backend(kind);
    if effective == SessionBackendKind::Tmux && which::which("tmux").is_err() {
        eprintln!(
            "  {} tmux backend selected but tmux was not found on PATH - install tmux \
             before running `loom run`, or re-run `loom init` with `--backend native`",
            "!".yellow().bold()
        );
    }

    Ok(kind)
}

/// The backend that will actually apply: an explicit choice if any, else
/// `~/.loom/config.toml`'s value. The warning in `resolve_backend_choice`
/// must key off this, not just an explicit choice, since an inherited tmux
/// setting is just as unusable without tmux on PATH as a typed one.
pub(super) fn effective_backend(kind: Option<SessionBackendKind>) -> SessionBackendKind {
    kind.unwrap_or_else(|| crate::user_config::UserConfig::load().terminal_backend())
}

/// Prompt for the backend, re-prompting on invalid input.
///
/// `Ok(None)` (a plain Enter, or EOF) means inherit rather than pin;
/// `Ok(Some(kind))` means the operator typed a choice.
fn prompt_backend_choice() -> Result<Option<SessionBackendKind>> {
    let user_set = crate::user_config::UserConfig::load().terminal_backend_set();
    let prompt = format!(
        "Terminal backend for sessions [native/tmux] ({}): ",
        backend_default_label(user_set)
    );
    loop {
        print!("{prompt}");
        std::io::stdout().flush().ok();
        let mut response = String::new();
        if std::io::stdin().read_line(&mut response)? == 0 {
            return Ok(None); // EOF - no answer given, same as a plain Enter.
        }
        match parse_backend_response(&response) {
            Some(choice) => return Ok(choice),
            None => println!("  Please enter 'native' or 'tmux'."),
        }
    }
}

/// Map trimmed prompt input to a choice.
///
/// The outer `None` is invalid input (the caller re-prompts); the inner
/// `None` is a plain Enter, i.e. inherit rather than pin.
pub(super) fn parse_backend_response(input: &str) -> Option<Option<SessionBackendKind>> {
    match input.trim().to_ascii_lowercase().as_str() {
        "" => Some(None),
        "native" => Some(Some(SessionBackendKind::Native)),
        "tmux" => Some(Some(SessionBackendKind::Tmux)),
        _ => None,
    }
}

/// The prompt's default clause, from what the user config actually set (see
/// `UserConfig::terminal_backend_set`, not the resolved getter, which cannot
/// tell "set" from "defaulted").
pub(super) fn backend_default_label(user_set: Option<SessionBackendKind>) -> String {
    match user_set {
        Some(kind) => format!("{kind}, from ~/.loom/config.toml"),
        None => "native, built-in default".to_string(),
    }
}
