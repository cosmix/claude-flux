//! The dashboard's config surface: `GET /api/config` and `POST /api/config`.
//!
//! This is the server's only write route. Everything else it serves is a
//! snapshot; this route edits `~/.loom/config.toml` and
//! `.loom/work/config.toml` on the operator's behalf, so it is gated three
//! deep — `Host`, a *strict* `Origin`, and a double-submit CSRF token — and its
//! body is capped. See the access-control notes on
//! [`super`][`crate::commands::status::web`] for the posture those gates defend,
//! and `csrf` for why the token works.
//!
//! # What it exposes
//!
//! Exactly [`crate::user_config::keys::KEYS`], projected by `entries`. Two of
//! those keys — `terminal.backend` and `context.ceiling_tokens` — also have a
//! workspace tier, and for them the payload reports both tiers plus the one in
//! force. Nothing here keeps a second list of keys: adding one to the registry
//! adds it to the dashboard.

mod apply;
mod csrf;
mod entries;
mod request;
mod wire;
mod workspace;

#[cfg(test)]
mod tests;

use std::path::Path;

use anyhow::{bail, Result};

use crate::user_config::keys::{self, KeySpec};
use crate::user_config::UserConfig;

pub(super) use request::{handle_post, serve_get};

use wire::{error_body, ConfigPayload, ConfigUpdate, ConfigUpdated, ProjectScope};
use workspace::Workspace;

/// Where the project scope lives, as the payload reports it: a fixed relative
/// path, never an absolute one. Any local process can read this dashboard, so
/// nothing it serves names a directory on the operator's disk.
const PROJECT_CONFIG_PATH: &str = ".loom/work/config.toml";

/// Which config file a write targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Scope {
    /// `~/.loom/config.toml`.
    User,
    /// `<repo>/.loom/work/config.toml`.
    Project,
}

impl Scope {
    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "user" => Ok(Self::User),
            "project" => Ok(Self::Project),
            other => bail!("unknown scope {other:?}; expected \"user\" or \"project\""),
        }
    }
}

/// The `GET /api/config` body for the tree served from `base`.
pub(super) fn payload(base: &Path) -> Result<String> {
    let workspace = Workspace::open(base)?;
    let entries = entries::all(&UserConfig::load(), workspace.as_ref())?;
    Ok(serde_json::to_string(&ConfigPayload {
        csrf_token: csrf::token().to_owned(),
        project: ProjectScope {
            available: workspace.is_some(),
            path: PROJECT_CONFIG_PATH.to_owned(),
        },
        entries,
    })?)
}

/// Apply a `POST /api/config` body, as an HTTP status and a JSON body.
pub(super) fn update(base: &Path, body: &[u8]) -> (u16, String) {
    match apply_update(base, body) {
        Ok(response) => (200, response),
        Err(UpdateError::Invalid(message)) => (400, error_body(&message)),
        Err(UpdateError::NoWorkspace) => (
            409,
            error_body("no project workspace; run loom init in this repository first"),
        ),
        Err(UpdateError::Failed(error)) => {
            // The underlying error names absolute config paths, so it is logged
            // rather than served — the same rule the status route follows.
            tracing::warn!("dashboard could not apply a config update: {error}");
            (500, error_body("config update failed"))
        }
    }
}

/// Why an update did not happen, split by what may be told to the client.
enum UpdateError {
    /// The client's own request was wrong. The message is safe to serve: it is
    /// the registry's own validator wording, naming only the key and the value
    /// the client itself sent.
    Invalid(String),
    /// A project-scope write with no workspace to write to.
    NoWorkspace,
    /// Anything else — a locked file, an unparseable config, a failed rename.
    Failed(anyhow::Error),
}

fn apply_update(base: &Path, body: &[u8]) -> Result<String, UpdateError> {
    let request: ConfigUpdate = serde_json::from_slice(body)
        .map_err(|error| UpdateError::Invalid(format!("malformed request body: {error}")))?;
    let scope = Scope::parse(&request.scope).map_err(invalid)?;
    let spec = keys::spec(&request.name).map_err(invalid)?;
    let value = parse_value(spec, scope, request.value.as_deref())?;

    let workspace = Workspace::open(base).map_err(UpdateError::Failed)?;
    if scope == Scope::Project && workspace.is_none() {
        return Err(UpdateError::NoWorkspace);
    }
    let (old, new) =
        apply::apply(scope, spec, value, workspace.as_ref()).map_err(UpdateError::Failed)?;

    // Re-opened rather than reused: the write just changed the file this was
    // read from, and the refreshed entry must describe the state on disk now.
    let workspace = Workspace::open(base).map_err(UpdateError::Failed)?;
    let entry = entries::entry(spec, &UserConfig::load(), workspace.as_ref())
        .map_err(UpdateError::Failed)?;
    serde_json::to_string(&ConfigUpdated { entry, old, new })
        .map_err(|error| UpdateError::Failed(error.into()))
}

/// Validate the requested value against the key's own registry entry, and the
/// key against the requested scope. `None` is an unset and needs no parse.
fn parse_value(
    spec: &KeySpec,
    scope: Scope,
    value: Option<&str>,
) -> Result<Option<toml_edit::Value>, UpdateError> {
    if scope == Scope::Project && !entries::project_scoped(spec) {
        return Err(UpdateError::Invalid(format!(
            "{}: has no project scope; set it at user scope instead",
            spec.name
        )));
    }
    match value {
        Some(raw) => spec.parse(raw).map(Some).map_err(invalid),
        None => Ok(None),
    }
}

/// The token a test server will accept. In-process, so the loopback tests in
/// `web::tests::config_api` read the same `OnceLock` the server does.
#[cfg(test)]
pub(super) fn test_token() -> &'static str {
    csrf::token()
}

/// Carry an `anyhow` message through as a client-safe validation failure.
/// Used only for the registry's own validators, whose messages quote the key
/// name and the client's own value and nothing else.
fn invalid(error: anyhow::Error) -> UpdateError {
    UpdateError::Invalid(error.to_string())
}
