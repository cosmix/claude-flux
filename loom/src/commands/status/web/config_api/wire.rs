//! The `/api/config` wire shape.
//!
//! Pinned by `web/src/api/fixtures/config.json` and the test that reads it back
//! (`config_api::tests`), the same discipline `model.rs` keeps for
//! `/api/status`: the page parses the fixture with zod, so a field renamed here
//! and not there is a runtime failure in the browser rather than a build
//! failure here.

use serde::{Deserialize, Serialize};

use crate::user_config::keys::ValueKind;

/// The whole `GET /api/config` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigPayload {
    /// The token every `POST /api/config` must echo in `X-Loom-Csrf`.
    pub csrf_token: String,
    /// Whether a workspace config exists to write, and where it lives.
    pub project: ProjectScope,
    /// Every registry key, in `crate::user_config::keys::KEYS` order.
    pub entries: Vec<ConfigEntry>,
}

/// The project scope's availability, as the dialog needs to know it before
/// offering a project-scope control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectScope {
    /// Whether the served tree has a `.loom/work` to write to. When false,
    /// every entry's `project` is null and a project-scope write is a 409.
    pub available: bool,
    /// Where the project config lives, relative to the repository root. A
    /// fixed relative path: the dashboard is readable by any local process, so
    /// it names no absolute path anywhere in its output.
    pub path: String,
}

/// One registry key as the settings dialog sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigEntry {
    /// Dotted key name, e.g. `context.ceiling_tokens`.
    pub name: String,
    /// The registry's one-line description.
    pub help: String,
    /// The value shape, so the dialog can pick a control.
    pub kind: ConfigKind,
    /// Which scopes accept a write for this key.
    pub scopes: Vec<String>,
    /// The built-in this key resolves to when NEITHER scope sets it.
    ///
    /// Not recoverable from the rest of the entry: `user.value` is the
    /// RESOLVED user-scope value, so it stops being the built-in the moment
    /// `user.set` turns true. A dialog offering "reset to the built-in" needs
    /// this even then.
    pub default: String,
    /// What the user tier resolves to, and whether the file set it.
    pub user: ScopeValue,
    /// The same for the project tier; null for a user-only key, and for every
    /// key when no workspace is available.
    pub project: Option<ScopeValue>,
    /// The value loom will actually use, and which tier it came from.
    pub effective: EffectiveValue,
}

/// The value shape a key accepts, mirroring [`ValueKind`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ConfigKind {
    /// `true` / `false`.
    Bool,
    /// A non-negative integer.
    U32,
    /// One of a fixed set of string variants, listed verbatim.
    Enum { variants: Vec<String> },
}

/// One tier's view of a key: what it resolves to, and whether that tier's file
/// actually says so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeValue {
    /// The rendered value this tier resolves to. With `set` false this is the
    /// built-in the tier falls back to, not a value read from a file.
    pub value: String,
    /// Whether this tier's file sets the key.
    pub set: bool,
}

/// The value loom resolves for a key, and the tier it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveValue {
    /// The rendered value.
    pub value: String,
    /// The tier that supplied it.
    pub source: Source,
}

/// Which tier an effective value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// The workspace `.loom/work/config.toml`.
    Project,
    /// `~/.loom/config.toml`.
    User,
    /// Neither file set it; this is loom's built-in.
    Default,
}

/// A `POST /api/config` request body.
#[derive(Debug, Clone, Deserialize)]
pub struct ConfigUpdate {
    /// `"user"` or `"project"`.
    pub scope: String,
    /// The dotted key name.
    pub name: String,
    /// The new value as a string, parsed against the key's [`ConfigKind`], or
    /// null to unset the key at `scope`.
    #[serde(default)]
    pub value: Option<String>,
}

/// The `200` body of a successful `POST /api/config`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigUpdated {
    /// The key re-projected after the write, so the dialog can refresh one row
    /// without re-fetching the whole payload.
    pub entry: ConfigEntry,
    /// What the written scope resolved to before the write.
    pub old: String,
    /// What it resolves to now.
    pub new: String,
}

/// Any non-2xx `/api/config` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigError {
    /// An operator-facing message. Validation failures carry the registry's own
    /// wording; anything that could name a filesystem path is logged instead
    /// and reported generically.
    pub error: String,
}

impl From<&ValueKind> for ConfigKind {
    fn from(kind: &ValueKind) -> Self {
        match kind {
            ValueKind::Bool => Self::Bool,
            ValueKind::U32 => Self::U32,
            ValueKind::Enum(variants) => Self::Enum {
                variants: variants.iter().map(|value| (*value).to_owned()).collect(),
            },
        }
    }
}

/// Serialize an error body. Infallible in practice — a single owned `String`
/// field cannot fail to serialize — so this returns the body directly rather
/// than a `Result` no caller could act on. The fallback is a fixed string
/// rather than a hand-built one holding `message`: interpolating unescaped
/// text into JSON is how an error path starts emitting malformed responses.
pub(super) fn error_body(message: &str) -> String {
    serde_json::to_string(&ConfigError {
        error: message.to_owned(),
    })
    .unwrap_or_else(|_| "{\"error\":\"config request failed\"}".to_owned())
}
