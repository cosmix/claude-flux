//! The `[pressure]` section of `.loom/work/config.toml`: the project tier of
//! `loom pressure`'s three model/effort slots.

use std::path::Path;

use crate::models::stage::ALLOWED_REASONING_EFFORTS;

use super::allowed::allowed_value;

const PRESSURE_SECTION: &str = "pressure";

/// The `[pressure]` section of `.loom/work/config.toml`: the project tier of
/// `loom pressure`'s three model/effort slots. Key-level fallback, same as
/// `[models]`.
#[derive(Debug, Clone, Default, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PressureConfig {
    claude_model: Option<String>,
    claude_effort: Option<String>,
    codex_model: Option<String>,
    codex_effort: Option<String>,
    address_model: Option<String>,
    address_effort: Option<String>,
}

/// Each accessor below shares its name with a private field of the same
/// meaning - distinct namespaces, so that is legal Rust, and intended here:
/// the field is what the file said, the method is what survives validation.
impl PressureConfig {
    /// Parse a `[pressure]` body - the filesystem-free seam
    /// `commands::pressure`'s resolution tests build sections with.
    pub fn from_section_toml(body: &str) -> anyhow::Result<Self> {
        toml::from_str(body)
            .map_err(|e| anyhow::anyhow!("failed to parse [pressure] section body: {e}"))
    }

    /// The value the file set, if it names one of [`crate::claude::CLAUDE_MODELS`].
    pub fn claude_model(&self) -> Option<&str> {
        allowed_value(
            self.claude_model.as_ref(),
            crate::claude::CLAUDE_MODELS,
            "pressure.claude_model",
        )
    }

    /// The value the file set, if it names one of [`ALLOWED_REASONING_EFFORTS`].
    pub fn claude_effort(&self) -> Option<&str> {
        allowed_value(
            self.claude_effort.as_ref(),
            ALLOWED_REASONING_EFFORTS,
            "pressure.claude_effort",
        )
    }

    /// The value the file set, if it names one of [`crate::codex::CODEX_MODELS`].
    pub fn codex_model(&self) -> Option<&str> {
        allowed_value(
            self.codex_model.as_ref(),
            crate::codex::CODEX_MODELS,
            "pressure.codex_model",
        )
    }

    /// The value the file set, if it names one of [`crate::codex::CODEX_EFFORTS`].
    pub fn codex_effort(&self) -> Option<&str> {
        allowed_value(
            self.codex_effort.as_ref(),
            crate::codex::CODEX_EFFORTS,
            "pressure.codex_effort",
        )
    }

    /// The value the file set, if it names one of [`crate::claude::CLAUDE_MODELS`].
    pub fn address_model(&self) -> Option<&str> {
        allowed_value(
            self.address_model.as_ref(),
            crate::claude::CLAUDE_MODELS,
            "pressure.address_model",
        )
    }

    /// The value the file set, if it names one of [`ALLOWED_REASONING_EFFORTS`].
    pub fn address_effort(&self) -> Option<&str> {
        allowed_value(
            self.address_effort.as_ref(),
            ALLOWED_REASONING_EFFORTS,
            "pressure.address_effort",
        )
    }
}

/// Read `[pressure]`; a missing, unreadable or malformed section yields an
/// all-`None` config, so a broken workspace file falls through to
/// `~/.loom/config.toml` instead of failing the run.
pub fn read_pressure_config(work_dir: &Path) -> PressureConfig {
    super::read_section::<PressureConfig>(work_dir, PRESSURE_SECTION)
        .ok()
        .flatten()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_accessor_returns_a_set_value() {
        let config = PressureConfig::from_section_toml(
            "claude_model = \"sonnet\"\n\
             claude_effort = \"high\"\n\
             codex_model = \"gpt-5.6-terra\"\n\
             codex_effort = \"xhigh\"\n\
             address_model = \"opus\"\n\
             address_effort = \"medium\"\n",
        )
        .unwrap();
        assert_eq!(config.claude_model(), Some("sonnet"));
        assert_eq!(config.claude_effort(), Some("high"));
        assert_eq!(config.codex_model(), Some("gpt-5.6-terra"));
        assert_eq!(config.codex_effort(), Some("xhigh"));
        assert_eq!(config.address_model(), Some("opus"));
        assert_eq!(config.address_effort(), Some("medium"));
    }

    #[test]
    fn an_out_of_set_value_is_dropped() {
        // "max" is a valid Claude effort but not a Codex one - the interesting
        // case where a value is real, just wrong for this field.
        let config = PressureConfig::from_section_toml("codex_effort = \"max\"\n").unwrap();
        assert_eq!(config.codex_effort(), None);
    }

    #[test]
    fn a_section_with_an_unknown_key_fails_to_deserialize_and_reads_as_all_none() {
        let temp = tempfile::TempDir::new().unwrap();
        let work_dir = temp.path().to_path_buf();
        std::fs::write(
            work_dir.join("config.toml"),
            "[pressure]\nclaude_model = \"sonnet\"\nbogus_key = \"nope\"\n",
        )
        .unwrap();

        let config = read_pressure_config(&work_dir);
        assert_eq!(config, PressureConfig::default());
    }

    #[test]
    fn a_missing_config_toml_yields_all_none() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = read_pressure_config(temp.path());
        assert_eq!(config, PressureConfig::default());
    }
}
