//! The `[pressure]` section of `~/.loom/config.toml`: the model and
//! reasoning effort `loom pressure` uses for its three foreground steps
//! absent a CLI flag or a project-tier override. Split out of `mod.rs` to
//! keep that file under Rule 17's 400-line limit — the `[pressure]` surface
//! grew from three model keys to six model+effort keys.

use anyhow::Result;
use toml_edit::DocumentMut;

use super::{parse, KeySpec, Origin, UserConfig};

/// The `[pressure]` section as the file set it: every field `Option` so
/// "explicitly set" stays distinguishable from "fell through to the built-in",
/// which is what the project tier in `crate::fs::work_dir` and
/// `loom config --list` both need.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct PressureSection {
    claude_model: Option<String>,
    claude_effort: Option<String>,
    codex_model: Option<String>,
    codex_effort: Option<String>,
    address_model: Option<String>,
    address_effort: Option<String>,
}

impl PressureSection {
    /// Parse every `[pressure]` key out of `doc`, validating each against its
    /// value set.
    pub(super) fn parse(doc: &DocumentMut) -> Result<Self> {
        let claude_efforts = crate::models::stage::ALLOWED_REASONING_EFFORTS;
        Ok(Self {
            claude_model: parse::get_enum(
                doc,
                "pressure",
                "claude_model",
                crate::claude::CLAUDE_MODELS,
            )?,
            claude_effort: parse::get_enum(doc, "pressure", "claude_effort", claude_efforts)?,
            codex_model: parse::get_enum(
                doc,
                "pressure",
                "codex_model",
                crate::codex::CODEX_MODELS,
            )?,
            codex_effort: parse::get_enum(
                doc,
                "pressure",
                "codex_effort",
                crate::codex::CODEX_EFFORTS,
            )?,
            address_model: parse::get_enum(
                doc,
                "pressure",
                "address_model",
                crate::claude::CLAUDE_MODELS,
            )?,
            address_effort: parse::get_enum(doc, "pressure", "address_effort", claude_efforts)?,
        })
    }
}

impl UserConfig {
    /// Claude model `loom pressure` uses for the `/pressure` step, absent a
    /// `--claude-model` flag. Default:
    /// [`crate::claude::DEFAULT_PRESSURE_CLAUDE_MODEL`].
    pub fn pressure_claude_model(&self) -> &str {
        self.pressure
            .claude_model
            .as_deref()
            .unwrap_or(crate::claude::DEFAULT_PRESSURE_CLAUDE_MODEL)
    }

    /// Claude reasoning effort `loom pressure` uses for the `/pressure` step,
    /// absent a `--claude-effort` flag. Default:
    /// [`crate::claude::DEFAULT_PRESSURE_CLAUDE_EFFORT`].
    pub fn pressure_claude_effort(&self) -> &str {
        self.pressure
            .claude_effort
            .as_deref()
            .unwrap_or(crate::claude::DEFAULT_PRESSURE_CLAUDE_EFFORT)
    }

    /// Codex model `loom pressure` uses for the `$pressure` step, absent a
    /// `--codex-model` flag. Default:
    /// [`crate::codex::DEFAULT_PRESSURE_CODEX_MODEL`].
    pub fn pressure_codex_model(&self) -> &str {
        self.pressure
            .codex_model
            .as_deref()
            .unwrap_or(crate::codex::DEFAULT_PRESSURE_CODEX_MODEL)
    }

    /// Codex reasoning effort `loom pressure` uses for the `$pressure` step,
    /// absent a `--codex-effort` flag. Default:
    /// [`crate::codex::DEFAULT_PRESSURE_CODEX_EFFORT`].
    pub fn pressure_codex_effort(&self) -> &str {
        self.pressure
            .codex_effort
            .as_deref()
            .unwrap_or(crate::codex::DEFAULT_PRESSURE_CODEX_EFFORT)
    }

    /// Claude model `loom pressure` uses for the `/address` reconciliation
    /// step, absent an `--address-model` flag. Default:
    /// [`crate::claude::DEFAULT_PRESSURE_CLAUDE_MODEL`].
    pub fn pressure_address_model(&self) -> &str {
        self.pressure
            .address_model
            .as_deref()
            .unwrap_or(crate::claude::DEFAULT_PRESSURE_CLAUDE_MODEL)
    }

    /// Claude reasoning effort `loom pressure` uses for the `/address`
    /// reconciliation step, absent an `--address-effort` flag. Default:
    /// [`crate::claude::DEFAULT_PRESSURE_ADDRESS_EFFORT`].
    pub fn pressure_address_effort(&self) -> &str {
        self.pressure
            .address_effort
            .as_deref()
            .unwrap_or(crate::claude::DEFAULT_PRESSURE_ADDRESS_EFFORT)
    }

    /// The rendered value and origin for a `pressure.*` key, or `None` when
    /// `spec` is not one — the arm `UserConfig::value_of` delegates to.
    pub(super) fn pressure_value_of(&self, spec: &KeySpec) -> Option<(String, Origin)> {
        Some(match spec.name {
            "pressure.claude_model" => (
                self.pressure_claude_model().to_string(),
                self.origin_of(self.pressure.claude_model.as_ref()),
            ),
            "pressure.claude_effort" => (
                self.pressure_claude_effort().to_string(),
                self.origin_of(self.pressure.claude_effort.as_ref()),
            ),
            "pressure.codex_model" => (
                self.pressure_codex_model().to_string(),
                self.origin_of(self.pressure.codex_model.as_ref()),
            ),
            "pressure.codex_effort" => (
                self.pressure_codex_effort().to_string(),
                self.origin_of(self.pressure.codex_effort.as_ref()),
            ),
            "pressure.address_model" => (
                self.pressure_address_model().to_string(),
                self.origin_of(self.pressure.address_model.as_ref()),
            ),
            "pressure.address_effort" => (
                self.pressure_address_effort().to_string(),
                self.origin_of(self.pressure.address_effort.as_ref()),
            ),
            _ => return None,
        })
    }

    /// The `[pressure]` block of `loom config --print`, every key resolved.
    /// Ends with a single newline — [`UserConfig::to_toml_string`] supplies
    /// the blank line separating sections.
    pub(super) fn pressure_toml(&self) -> String {
        format!(
            "[pressure]\nclaude_model = \"{}\"\nclaude_effort = \"{}\"\ncodex_model = \"{}\"\ncodex_effort = \"{}\"\naddress_model = \"{}\"\naddress_effort = \"{}\"\n",
            self.pressure_claude_model(),
            self.pressure_claude_effort(),
            self.pressure_codex_model(),
            self.pressure_codex_effort(),
            self.pressure_address_model(),
            self.pressure_address_effort(),
        )
    }
}
