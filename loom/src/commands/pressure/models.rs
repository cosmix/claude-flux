//! Resolves the six per-slot model/effort values one `loom pressure` run
//! spawns with.
//!
//! Each slot is independently selectable, precedence CLI flag > project
//! `.loom/work/config.toml` `[pressure]` > operator's `~/.loom/config.toml`
//! `[pressure]` > built-in default — the same precedence
//! [`crate::user_config::UserConfig`]'s other resolved getters follow, just
//! spread across six getters and a fourth (project) tier instead of one.

use crate::cli::types_pressure::PressureModelFlags;
use crate::fs::work_dir::PressureConfig;
use crate::user_config::UserConfig;

/// The six model/effort slots one pressure run spawns with, each resolved
/// flag > project `[pressure]` > user `[pressure]` > built-in default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PressureModels {
    /// Claude model for the `/pressure` step.
    pub claude: String,
    /// Claude reasoning effort for the `/pressure` step.
    pub claude_effort: String,
    /// Codex model for the `$pressure` step.
    pub codex: String,
    /// Codex reasoning effort for the `$pressure` step.
    pub codex_effort: String,
    /// Claude model for the `/address` step.
    pub address: String,
    /// Claude reasoning effort for the `/address` step.
    pub address_effort: String,
}

/// Resolve one slot: the flag, else the project value, else the user value —
/// which itself already collapses "user set it" and "built-in default".
fn slot(flag: Option<String>, project: Option<&str>, user: &str) -> String {
    flag.or_else(|| project.map(str::to_string))
        .unwrap_or_else(|| user.to_string())
}

impl PressureModels {
    /// Resolve all six slots. `project` and `user` are passed in (rather than
    /// read here) so tests can exercise precedence against constructed
    /// configs without touching the filesystem — `PressureConfig::default()`
    /// and `UserConfig::default()` give all-defaults.
    pub(super) fn resolve(
        flags: PressureModelFlags,
        project: &PressureConfig,
        user: &UserConfig,
    ) -> PressureModels {
        PressureModels {
            claude: slot(
                flags.claude_model,
                project.claude_model(),
                user.pressure_claude_model(),
            ),
            claude_effort: slot(
                flags.claude_effort,
                project.claude_effort(),
                user.pressure_claude_effort(),
            ),
            codex: slot(
                flags.codex_model,
                project.codex_model(),
                user.pressure_codex_model(),
            ),
            codex_effort: slot(
                flags.codex_effort,
                project.codex_effort(),
                user.pressure_codex_effort(),
            ),
            address: slot(
                flags.address_model,
                project.address_model(),
                user.pressure_address_model(),
            ),
            address_effort: slot(
                flags.address_effort,
                project.address_effort(),
                user.pressure_address_effort(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags() -> PressureModelFlags {
        PressureModelFlags::default()
    }

    #[test]
    fn all_defaults_when_nothing_set() {
        let models =
            PressureModels::resolve(flags(), &PressureConfig::default(), &UserConfig::default());
        assert_eq!(models.claude, "opus");
        assert_eq!(models.claude_effort, "xhigh");
        assert_eq!(models.codex, "gpt-5.6-sol");
        assert_eq!(models.codex_effort, "xhigh");
        assert_eq!(models.address, "opus");
        assert_eq!(models.address_effort, "high");
    }

    #[test]
    fn user_config_set_flag_absent_uses_user_config() {
        let toml = "[pressure]\nclaude_model = \"fable\"\ncodex_model = \"gpt-6-astra\"\naddress_model = \"sonnet\"\n";
        let user = crate::user_config::parse_document(toml).unwrap();
        let models = PressureModels::resolve(flags(), &PressureConfig::default(), &user);
        assert_eq!(models.claude, "fable");
        assert_eq!(models.codex, "gpt-6-astra");
        assert_eq!(models.address, "sonnet");
    }

    #[test]
    fn flag_overrides_both_config_tiers() {
        let toml = "[pressure]\nclaude_model = \"fable\"\ncodex_model = \"gpt-6-astra\"\naddress_model = \"sonnet\"\n";
        let user = crate::user_config::parse_document(toml).unwrap();
        let project = PressureConfig::from_section_toml("claude_model = \"haiku\"\n").unwrap();
        let mut f = flags();
        f.claude_model = Some("opus".to_string());
        f.codex_model = Some("gpt-5.6-luna".to_string());
        f.address_model = Some("opus".to_string());
        let models = PressureModels::resolve(f, &project, &user);
        assert_eq!(models.claude, "opus");
        assert_eq!(models.codex, "gpt-5.6-luna");
        assert_eq!(models.address, "opus");
    }

    #[test]
    fn address_slot_is_independent_of_pressure_slot() {
        // A flag on --claude-model alone must not leak into address, and vice
        // versa - the two Claude slots are separate steps in the pipeline.
        let mut f = flags();
        f.claude_model = Some("fable".to_string());
        let models = PressureModels::resolve(f, &PressureConfig::default(), &UserConfig::default());
        assert_eq!(models.claude, "fable");
        assert_eq!(models.address, "opus");

        let mut f = flags();
        f.address_model = Some("sonnet".to_string());
        let models = PressureModels::resolve(f, &PressureConfig::default(), &UserConfig::default());
        assert_eq!(models.claude, "opus");
        assert_eq!(models.address, "sonnet");
    }

    #[test]
    fn project_value_beats_user_file() {
        let user_toml = "[pressure]\nclaude_model = \"fable\"\n";
        let user = crate::user_config::parse_document(user_toml).unwrap();
        let project = PressureConfig::from_section_toml("claude_model = \"sonnet\"\n").unwrap();
        let models = PressureModels::resolve(flags(), &project, &user);
        assert_eq!(models.claude, "sonnet");
    }

    #[test]
    fn flag_beats_project_value() {
        let project = PressureConfig::from_section_toml("claude_model = \"sonnet\"\n").unwrap();
        let mut f = flags();
        f.claude_model = Some("haiku".to_string());
        let models = PressureModels::resolve(f, &project, &UserConfig::default());
        assert_eq!(models.claude, "haiku");
    }

    #[test]
    fn key_omitted_by_project_falls_through_to_user_file_not_builtin() {
        // The project section sets only claude_model - codex_model must fall
        // through to the user file's value, not skip straight to the built-in.
        let user_toml = "[pressure]\ncodex_model = \"gpt-6-astra\"\n";
        let user = crate::user_config::parse_document(user_toml).unwrap();
        let project = PressureConfig::from_section_toml("claude_model = \"sonnet\"\n").unwrap();
        let models = PressureModels::resolve(flags(), &project, &user);
        assert_eq!(models.claude, "sonnet");
        assert_eq!(models.codex, "gpt-6-astra");
    }

    #[test]
    fn every_effort_slot_is_independent_of_every_model_slot() {
        let project = PressureConfig::from_section_toml(
            "claude_effort = \"low\"\ncodex_effort = \"medium\"\naddress_effort = \"low\"\n",
        )
        .unwrap();
        let models = PressureModels::resolve(flags(), &project, &UserConfig::default());
        // Efforts come from the project tier...
        assert_eq!(models.claude_effort, "low");
        assert_eq!(models.codex_effort, "medium");
        assert_eq!(models.address_effort, "low");
        // ...while every model slot is untouched, still on the built-in.
        assert_eq!(models.claude, "opus");
        assert_eq!(models.codex, "gpt-5.6-sol");
        assert_eq!(models.address, "opus");
    }
}
