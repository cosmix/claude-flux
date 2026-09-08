//! Resolves the three models one `loom pressure` run spawns with.
//!
//! Each slot is independently selectable, precedence CLI flag > operator's
//! `~/.loom/config.toml` > built-in default — the same precedence
//! [`crate::user_config::UserConfig`]'s other resolved getters follow, just
//! spread across three getters instead of one.

use crate::user_config::UserConfig;

/// The three models one pressure run spawns with, each resolved
/// flag > ~/.loom/config.toml > built-in default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PressureModels {
    /// Claude model for the `/pressure` step.
    pub claude: String,
    /// Codex model for the `$pressure` step.
    pub codex: String,
    /// Claude model for the `/address` step.
    pub address: String,
}

impl PressureModels {
    /// Resolve the three slots. `config` is passed in (rather than loaded
    /// here) so tests can exercise precedence against a constructed
    /// [`UserConfig`] without touching the filesystem —
    /// `UserConfig::default()` gives all-defaults.
    pub(super) fn resolve(
        claude_flag: Option<String>,
        codex_flag: Option<String>,
        address_flag: Option<String>,
        config: &UserConfig,
    ) -> PressureModels {
        PressureModels {
            claude: claude_flag.unwrap_or_else(|| config.pressure_claude_model().to_string()),
            codex: codex_flag.unwrap_or_else(|| config.pressure_codex_model().to_string()),
            address: address_flag.unwrap_or_else(|| config.pressure_address_model().to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_defaults_when_nothing_set() {
        let models = PressureModels::resolve(None, None, None, &UserConfig::default());
        assert_eq!(models.claude, "opus");
        assert_eq!(models.codex, "gpt-5.6-sol");
        assert_eq!(models.address, "opus");
    }

    #[test]
    fn config_set_flag_absent_uses_config() {
        let toml = "[pressure]\nclaude_model = \"fable\"\ncodex_model = \"gpt-6-astra\"\naddress_model = \"sonnet\"\n";
        let config = crate::user_config::parse_document(toml).unwrap();
        let models = PressureModels::resolve(None, None, None, &config);
        assert_eq!(models.claude, "fable");
        assert_eq!(models.codex, "gpt-6-astra");
        assert_eq!(models.address, "sonnet");
    }

    #[test]
    fn flag_overrides_config() {
        let toml = "[pressure]\nclaude_model = \"fable\"\ncodex_model = \"gpt-6-astra\"\naddress_model = \"sonnet\"\n";
        let config = crate::user_config::parse_document(toml).unwrap();
        let models = PressureModels::resolve(
            Some("haiku".to_string()),
            Some("gpt-5.6-luna".to_string()),
            Some("opus".to_string()),
            &config,
        );
        assert_eq!(models.claude, "haiku");
        assert_eq!(models.codex, "gpt-5.6-luna");
        assert_eq!(models.address, "opus");
    }

    #[test]
    fn address_slot_is_independent_of_pressure_slot() {
        // A flag on --claude-model alone must not leak into address, and vice
        // versa - the two Claude slots are separate steps in the pipeline.
        let models = PressureModels::resolve(
            Some("fable".to_string()),
            None,
            None,
            &UserConfig::default(),
        );
        assert_eq!(models.claude, "fable");
        assert_eq!(models.address, "opus");

        let models = PressureModels::resolve(
            None,
            None,
            Some("sonnet".to_string()),
            &UserConfig::default(),
        );
        assert_eq!(models.claude, "opus");
        assert_eq!(models.address, "sonnet");
    }
}
