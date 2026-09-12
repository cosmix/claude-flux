//! The workspace tier of a config key, read once per request.
//!
//! # Two shadowing rules, because `.loom/work/config.toml` uses both
//!
//! `[terminal]` and `[context]` override `~/.loom/config.toml` a whole SECTION
//! at a time (see [`crate::fs::work_dir::read_context_config`]): a present
//! section wins outright, and any key it omits derives from the built-ins
//! rather than falling through to the user tier. Both structs bake that
//! derivation in — `TerminalConfig`/`ContextConfig` implement `Default`, so
//! there is no way to tell "this section set nothing" from "this section set
//! everything but the one key I asked about" once it is deserialized.
//!
//! `[pressure]` and `[models]` shadow a KEY at a time instead: every field in
//! them is optional, and a present section that omits a key falls through to
//! `~/.loom/config.toml` for that key rather than shadowing it with a built-in
//! the operator never asked for — a plan's stage can leave `standard_effort`
//! unset while pinning `standard_model`, and the effort must still come from
//! whatever the user tier (or the built-in) says. [`ProjectTier`] names which
//! rule a key lives under, [`resolve`] is the one `match` that assigns it, and
//! [`Workspace::shadows`] is what a caller asks instead of re-deriving the
//! rule from the key's section.
//!
//! [`Workspace::value_of`] answers "what does the project tier's own value for
//! this key look like" regardless of whether it shadows — for an omitted
//! section-level key that is the built-in the section would derive; for an
//! omitted key-level key it is the user tier's value, the one the fallback
//! leaves in force, while [`Workspace::shadows`] is what keeps the project
//! tier out of `effective` in that case.
//!
//! `config_api::tests` pins all of this against the runtime readers —
//! [`crate::fs::work_dir::read_terminal_config`] and
//! [`crate::fs::work_dir::resolve_context_ceiling_tokens`] for the two
//! section-level keys, [`crate::fs::work_dir::read_pressure_config`] and
//! [`crate::fs::work_dir::resolve_stage_model_effort`] for the fourteen
//! key-level ones — because a settings page that disagrees with the daemon
//! about the value in force is worse than no settings page.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use toml_edit::DocumentMut;

use crate::fs::work_dir::{read_config, ContextConfig, WorkDir};
use crate::models::session::TerminalConfig;
use crate::user_config::keys::KeySpec;
use crate::user_config::UserConfig;

/// How the workspace tier shadows the user tier for one key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProjectTier {
    /// A present section wins WHOLE: a key it omits resolves to the built-in
    /// rather than falling through (`[terminal]`, `[context]`).
    Section,
    /// Only a present KEY wins; a section that omits the key falls through to
    /// the user tier (`[pressure]`, `[models]`).
    Key,
}

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
    /// For a section-level key with the section absent this is the built-in
    /// the section would derive, which is what an operator gets the moment
    /// the section appears — not the user tier's value, which the section
    /// would shadow. For a key-level key the file does not set, it is the
    /// user tier's value: that is what clearing the key leaves in force.
    pub(super) fn value_of(&self, spec: &KeySpec) -> Result<String> {
        match resolve(spec, self.doc.get(spec.section).cloned()) {
            Some((_, value)) => Ok(value?.unwrap_or_else(|| UserConfig::load().value_of(spec).0)),
            None => bail!("{} has no project scope", spec.name),
        }
    }

    /// Whether the project tier is the one in force for `spec`: a present
    /// section for a section-level key, a present key for a key-level one.
    pub(super) fn shadows(&self, spec: &KeySpec) -> bool {
        match tier_of(spec) {
            Some(ProjectTier::Section) => self.has_section(spec),
            Some(ProjectTier::Key) => self.has_key(spec),
            None => false,
        }
    }
}

/// What the workspace tier resolves `spec` to in `section`, and how it
/// shadows the user tier — `None` when the key has no workspace tier at all.
///
/// The ONE place the workspace-backed keys are named. [`backs`] and
/// [`tier_of`] ask this question rather than keeping a second list of key
/// names beside it: a list and a `match` that must agree drift, and the drift
/// would land as a panic on a live request rather than as a build failure.
///
/// The `_` arm matches on `spec.section` rather than `spec.name` so a key
/// added to `[pressure]`/`[models]` in the registry picks up the key-level
/// rule with no edit here — the two exact-name arms above stay name-matched
/// so a later key added to `[context]` cannot silently inherit
/// `ContextConfig`'s reading of a DIFFERENT field.
///
/// The value is `None` only for a key-level key the section does not name;
/// [`Workspace::value_of`] fills that in from the user tier.
fn resolve(
    spec: &KeySpec,
    section: Option<toml::Value>,
) -> Option<(ProjectTier, Result<Option<String>>)> {
    match spec.name {
        "terminal.backend" => Some((
            ProjectTier::Section,
            typed::<TerminalConfig>(section, spec).map(|config| Some(config.backend.to_string())),
        )),
        "context.ceiling_tokens" => Some((
            ProjectTier::Section,
            typed::<ContextConfig>(section, spec)
                .map(|config| Some(config.ceiling_tokens.to_string())),
        )),
        _ => match spec.section {
            "pressure" | "models" => {
                Some((ProjectTier::Key, Ok(section_key(section.as_ref(), spec))))
            }
            _ => None,
        },
    }
}

/// Whether the workspace tier resolves `spec` at all — see [`resolve`], which
/// answers this by having an arm for the key or not.
pub(super) fn backs(spec: &KeySpec) -> bool {
    resolve(spec, None).is_some()
}

/// How `spec`'s workspace tier shadows the user tier — see [`resolve`].
pub(super) fn tier_of(spec: &KeySpec) -> Option<ProjectTier> {
    resolve(spec, None).map(|(tier, _)| tier)
}

/// The section's own string for `spec.field`, or `None` when the section
/// omits it or is itself absent.
fn section_key(section: Option<&toml::Value>, spec: &KeySpec) -> Option<String> {
    section
        .and_then(|section| section.get(spec.field))
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
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
