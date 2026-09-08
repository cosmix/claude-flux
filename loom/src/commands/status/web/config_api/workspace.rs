//! The workspace tier of a config key, read once per request.
//!
//! # Why this mirrors the section-level fallback rather than reading a key
//!
//! `.loom/work/config.toml` overrides `~/.loom/config.toml` a whole SECTION at
//! a time, not a key at a time (see
//! [`crate::fs::work_dir::read_context_config`]): a present `[context]` wins
//! outright, and any key it omits derives from the built-ins rather than
//! falling through to the user tier. So "what does the project scope say about
//! `context.ceiling_tokens`" is not "what does that key hold" — it is "what
//! does the section this key lives in resolve that key to", which for a
//! present-but-keyless section is the built-in.
//!
//! [`Workspace::value_of`] answers that question, and
//! `config_api::tests` pins it against
//! [`crate::fs::work_dir::read_terminal_config`] and
//! [`crate::fs::work_dir::resolve_context_ceiling_tokens`] for all three
//! sources, because a settings page that disagrees with the daemon about the
//! value in force is worse than no settings page.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use toml_edit::DocumentMut;

use crate::fs::work_dir::{read_config, ContextConfig, WorkDir};
use crate::models::session::TerminalConfig;
use crate::user_config::keys::KeySpec;

/// A served tree's `.loom/work` and its parsed `config.toml`.
pub(super) struct Workspace {
    /// The `.loom/work` directory itself — what the write path locks.
    root: PathBuf,
    /// The parsed config, an empty table when the file does not exist yet.
    doc: toml::Value,
}

impl Workspace {
    /// The workspace under `base`, or `None` when the served tree has none.
    ///
    /// Absence is reported rather than created: a dashboard write must not
    /// materialize a workspace the operator never ran `loom init` for, so a
    /// project-scope write against `None` is a 409.
    pub(super) fn open(base: &Path) -> Result<Option<Self>> {
        let root = WorkDir::new(base)
            .context("failed to resolve the work directory")?
            .root()
            .to_path_buf();
        if !root.exists() {
            return Ok(None);
        }
        let text = read_config(&root)?.to_string();
        Ok(Some(Self {
            doc: parse(&text)?,
            root,
        }))
    }

    /// The `.loom/work` directory this workspace writes to.
    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    /// The same view over an in-flight document, so the write path can report
    /// the values either side of its own edit using this exact resolution.
    pub(super) fn from_document(root: &Path, doc: &DocumentMut) -> Result<Self> {
        Ok(Self {
            root: root.to_path_buf(),
            doc: parse(&doc.to_string())?,
        })
    }

    /// Whether `spec`'s section is present at all — the question the
    /// section-level fallback actually turns on, and so the one that decides
    /// whether the project tier is the effective source.
    pub(super) fn has_section(&self, spec: &KeySpec) -> bool {
        self.doc.get(spec.section).is_some()
    }

    /// Whether the file sets `spec`'s key itself.
    pub(super) fn has_key(&self, spec: &KeySpec) -> bool {
        self.doc
            .get(spec.section)
            .and_then(|section| section.get(spec.field))
            .is_some()
    }

    /// What the workspace tier resolves `spec` to.
    ///
    /// With the section absent this is the built-in the section would derive,
    /// which is what an operator gets the moment the section appears — not the
    /// user tier's value, which the section would shadow.
    pub(super) fn value_of(&self, spec: &KeySpec) -> Result<String> {
        match resolve(spec, self.doc.get(spec.section).cloned()) {
            Some(value) => value,
            None => bail!("{} has no project scope", spec.name),
        }
    }
}

/// What the workspace tier resolves `spec` to in `section`, or `None` when the
/// key has no workspace tier at all.
///
/// The ONE place the workspace-backed keys are named. [`backs`] asks this
/// question rather than keeping a second list of key names beside it: a list
/// and a `match` that must agree drift, and the drift would land as a panic on
/// a live request rather than as a build failure.
fn resolve(spec: &KeySpec, section: Option<toml::Value>) -> Option<Result<String>> {
    match spec.name {
        "terminal.backend" => {
            Some(typed::<TerminalConfig>(section, spec).map(|config| config.backend.to_string()))
        }
        "context.ceiling_tokens" => Some(
            typed::<ContextConfig>(section, spec).map(|config| config.ceiling_tokens.to_string()),
        ),
        _ => None,
    }
}

/// Whether the workspace tier resolves `spec` at all — see [`resolve`], which
/// answers this by having an arm for the key or not.
pub(super) fn backs(spec: &KeySpec) -> bool {
    resolve(spec, None).is_some()
}

fn parse(text: &str) -> Result<toml::Value> {
    if text.trim().is_empty() {
        return Ok(toml::Value::Table(toml::map::Map::new()));
    }
    toml::from_str(text).context("failed to parse the workspace config as TOML")
}

/// Deserialize `section` into the struct that owns it, or take that struct's
/// built-in defaults when the section is absent.
fn typed<T: serde::de::DeserializeOwned + Default>(
    section: Option<toml::Value>,
    spec: &KeySpec,
) -> Result<T> {
    match section {
        Some(value) => value.try_into().with_context(|| {
            format!(
                "failed to read [{}] from the workspace config",
                spec.section
            )
        }),
        None => Ok(T::default()),
    }
}
