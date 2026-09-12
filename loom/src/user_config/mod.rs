//! User-level configuration at `~/.loom/config.toml`.
//!
//! Distinct from, and a fallback tier under, a project's workspace config at
//! `<repo>/.loom/work/config.toml` (see [`crate::fs::work_dir::Config`], the
//! `.loom/work/` type). This file holds settings that make sense per operator
//! rather than per plan: whether loom checks for updates, which terminal
//! backend `loom run` defaults to, the default context ceiling, the
//! Claude/Codex models and reasoning efforts `loom pressure` spawns absent a
//! CLI flag, and the model and reasoning effort each stage type's main-agent
//! session launches with absent a plan field. `loom config`
//! (`crate::commands::config`) is the only writer; every other consumer
//! reads through [`UserConfig::load`], which never fails.
//!
//! # Reads must not create `~/.loom/`
//!
//! [`UserConfig::load`] and [`UserConfig::load_strict`] use a plain
//! `std::fs::read_to_string`, treating `NotFound` as "absent". They
//! deliberately avoid [`crate::fs::locking::locked_read`] and friends: those
//! helpers lock the file's *parent directory*, and acquiring that lock
//! creates the parent if it does not exist (see
//! `crate::fs::locking::lock_parent_dir`) — which would make a pure read like
//! `loom config --list` materialize `~/.loom/` on disk.
//!
//! # Writes are locked and atomic
//!
//! [`set`] and [`unset`] (both in `write.rs`) go through
//! [`crate::fs::locking::locked_update`], which holds the exclusive
//! parent-directory lock across the whole read-modify-write, presents a missing
//! file as an empty string, creates `~/.loom/` as a side effect of locking, and
//! finishes with a crash-atomic temp+rename write. Those are the ONLY paths
//! that may create `~/.loom/`. loom is invoked concurrently from shell hooks,
//! so this race is real.
//!
//! # Hermeticity: two different seams, two different scopes
//!
//! [`UserConfig::load`]/[`UserConfig::load_strict`] resolve their path
//! through `read_config_path`, and [`set`]/[`unset`] through
//! `write_config_path`.
//! Those two are private, so these are plain code spans: a bracketed link
//! would resolve only under `--document-private-items`, which the docs gate
//! does not pass.
//! Both are `config_path()` in production but, in `#[cfg(test)]`, the same
//! per-thread redirect (`redirect_user_config`, itself test-only) that defaults
//! to "no user config" when unset. Sharing one redirect between the read and
//! write seams lets a test do a real set/read round trip against a temp path.
//! Any lib **unit** test that reaches a loader without installing a redirect
//! therefore sees an all-default [`UserConfig`], never `~/.loom/config.toml`.
//! This matters because `loom config` (`crate::commands::config`) is the tool
//! that CREATES that file: without this seam, the whole lib unit-test binary
//! would depend on the developer running these tests never having run `loom
//! config`, and would start failing the moment they had.
//!
//! That `#[cfg(test)]` redirect covers the lib's own unit-test binary ONLY.
//! `loom/tests/**` links this lib WITHOUT `cfg(test)`, so `read_config_path`
//! resolves through plain `config_path()` there — a test under `loom/tests/`
//! that wants isolation from the operator's real `~/.loom/config.toml` sets
//! `LOOM_HOME` instead (see `config_path`'s doc comment and
//! `loom/tests/e2e/daemon_config/mod.rs::isolate_user_config`).

use std::path::PathBuf;

use anyhow::{Context, Result};
use toml_edit::DocumentMut;

use crate::models::constants::DEFAULT_CONTEXT_CEILING_TOKENS;
use crate::models::session::SessionBackendKind;

pub mod keys;

use keys::KeySpec;

mod models;
mod parse;
mod pressure;
mod render;
mod write;

pub use write::{set, unset};
/// The path-explicit write seams, for tests that must not touch a real
/// `~/.loom/config.toml`. `set`/`unset` are the production entry points.
#[cfg(test)]
pub(crate) use write::{set_in, unset_in};

#[cfg(test)]
mod redirect;

#[cfg(test)]
pub(crate) use redirect::{redirect_user_config, UserConfigRedirect};

#[cfg(test)]
mod tests;

/// Where a resolved key's value came from.
///
/// `loom config` reads only the user config, never a workspace config, so
/// these two are the whole space — a workspace `[terminal]`/`[context]`/
/// `[pressure]`/`[models]` section is a separate resolution tier that
/// [`crate::fs::work_dir::read_terminal_config`]/
/// [`crate::fs::work_dir::read_context_config`] merge in themselves, above
/// this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Explicitly set in `~/.loom/config.toml`.
    Set,
    /// Not set; the rendered value is the built-in default.
    Default,
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Origin::Set => write!(f, "set"),
            Origin::Default => write!(f, "default"),
        }
    }
}

/// The user-level config resolved from `~/.loom/config.toml`.
///
/// Every field is `Option` so "explicitly set in the file" stays
/// distinguishable from "fell through to the built-in default": the resolved
/// getters below collapse that distinction for an ordinary caller, while
/// [`UserConfig::value_of`] and the `terminal_backend_set`/
/// `context_ceiling_tokens_set` accessors keep it for `loom config --list` and
/// for the workspace fallback tier in `crate::fs::work_dir`, both of which
/// need to know whether the user config actually set a value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserConfig {
    update_check: Option<bool>,
    update_check_interval_hours: Option<u32>,
    terminal_backend: Option<SessionBackendKind>,
    context_ceiling_tokens: Option<u32>,
    pressure: pressure::PressureSection,
    models: models::ModelsSection,
}

/// The absolute path to `~/.loom/config.toml`.
///
/// Honors `LOOM_HOME` when set to a non-empty value: the returned path is
/// then `$LOOM_HOME/config.toml` instead of `~/.loom/config.toml`. `LOOM_HOME`
/// names the loom user directory itself (the `.loom` directory, not the home
/// directory above it) — the same shape as `LOOM_HOOKS_DIR`
/// (`hooks/generator.rs`). This is the seam a caller outside `#[cfg(test)]`
/// uses to keep a test or a scratch run off the operator's real config; see
/// `loom/tests/e2e/daemon_config/mod.rs::isolate_user_config`.
///
/// Errors only when neither `LOOM_HOME` is set nor the home directory can be
/// resolved. Never creates anything — this is a pure path computation.
pub fn config_path() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("LOOM_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(dir).join("config.toml"));
    }
    dirs::home_dir()
        .map(|home| home.join(".loom").join("config.toml"))
        .ok_or_else(|| {
            anyhow::anyhow!("could not resolve the home directory for ~/.loom/config.toml")
        })
}

/// The file the read paths (`UserConfig::load`/`UserConfig::load_strict`)
/// consult, or `None` when no user config file is in play. Fallible because
/// an unresolvable home directory is a real failure that `load_strict` must
/// surface; `load` discards it.
///
/// In production this is `config_path()`. In the lib test binary it is the
/// per-thread redirect from [`redirect::redirect_user_config`] and NOTHING
/// else: an unset redirect means "no user config", so a test can never
/// observe — or be broken by — the operator's own `~/.loom/config.toml`.
/// `loom config` itself creates that file, so the suite must not depend on
/// nobody having one.
#[cfg(not(test))]
fn read_config_path() -> Result<Option<PathBuf>> {
    config_path().map(Some)
}

#[cfg(test)]
fn read_config_path() -> Result<Option<PathBuf>> {
    Ok(redirect::test_redirect())
}

/// The file [`set`] and [`unset`] write to. Unlike the read side there is no
/// "absent" case: a write with nowhere to go is an error. Shares the same
/// thread-local redirect as [`read_config_path`], so a test installing one
/// redirect can do a real set/read round trip against a single temp path.
#[cfg(not(test))]
fn write_config_path() -> Result<PathBuf> {
    config_path()
}

#[cfg(test)]
fn write_config_path() -> Result<PathBuf> {
    redirect::test_redirect()
        .ok_or_else(|| anyhow::anyhow!("no user config redirect installed for this test"))
}

impl UserConfig {
    /// Load `~/.loom/config.toml` for production consumers. Never fails: an
    /// absent, unreadable, or unparseable file yields an all-`None`
    /// [`UserConfig`] (every getter then returns the built-in default). A
    /// broken user config must never take down `loom run`. Does not create
    /// `~/.loom`.
    pub fn load() -> UserConfig {
        let Ok(Some(path)) = read_config_path() else {
            return UserConfig::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => parse_document(&text).unwrap_or_default(),
            Err(_) => UserConfig::default(),
        }
    }

    /// Load `~/.loom/config.toml`, surfacing a read or parse failure. Used
    /// only by `loom config` (`crate::commands::config`), which wants to
    /// report a broken user config rather than silently fall back to
    /// defaults. Does not create `~/.loom`.
    pub fn load_strict() -> Result<UserConfig> {
        let Some(path) = read_config_path()? else {
            return Ok(UserConfig::default());
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(UserConfig::default()),
            Err(e) => {
                return Err(e).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        parse_document(&text).with_context(|| format!("failed to parse {}", path.display()))
    }

    /// Whether loom checks for updates on startup. Default: `true`.
    pub fn update_check(&self) -> bool {
        self.update_check.unwrap_or(true)
    }

    /// Hours between update checks. Default: `24`.
    pub fn update_check_interval_hours(&self) -> u32 {
        self.update_check_interval_hours.unwrap_or(24)
    }

    /// Terminal backend `loom run` defaults to. Default: native.
    pub fn terminal_backend(&self) -> SessionBackendKind {
        self.terminal_backend.unwrap_or_default()
    }

    /// `terminal.backend`, only when the user config explicitly set it — the
    /// accessor `crate::fs::work_dir::read_terminal_config`'s fallback tier
    /// needs, since it must not treat "fell through to the built-in" as a
    /// value worth applying over an absent workspace section any differently
    /// than the built-in itself.
    pub fn terminal_backend_set(&self) -> Option<SessionBackendKind> {
        self.terminal_backend
    }

    /// Default context ceiling, in resident tokens, for a stage's agent
    /// session. Default: [`DEFAULT_CONTEXT_CEILING_TOKENS`].
    pub fn context_ceiling_tokens(&self) -> u32 {
        self.context_ceiling_tokens
            .unwrap_or(DEFAULT_CONTEXT_CEILING_TOKENS)
    }

    /// `context.ceiling_tokens`, only when the user config explicitly set it
    /// — see [`UserConfig::terminal_backend_set`] for why the workspace
    /// fallback tier needs this rather than the resolved getter.
    pub fn context_ceiling_tokens_set(&self) -> Option<u32> {
        self.context_ceiling_tokens
    }
}

/// Parse `text` as `~/.loom/config.toml`, validating every present key
/// against its type. `pub(crate)` so `UserConfig::load`/`load_strict` share
/// one parser, and so tests can exercise parsing/rendering against a TOML
/// string without touching the real `$HOME`.
pub(crate) fn parse_document(text: &str) -> Result<UserConfig> {
    let doc: DocumentMut = text.parse().context("invalid TOML")?;
    Ok(UserConfig {
        update_check: parse::get_bool(&doc, "update", "check")?,
        update_check_interval_hours: parse::get_u32(&doc, "update", "check_interval_hours")?,
        terminal_backend: parse::get_backend(&doc)?,
        context_ceiling_tokens: parse::get_u32(&doc, "context", "ceiling_tokens")?,
        pressure: pressure::PressureSection::parse(&doc)?,
        models: models::ModelsSection::parse(&doc)?,
    })
}
