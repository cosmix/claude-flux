//! The `[models]` section of `.loom/work/config.toml`: the project tier of a
//! stage type's model/reasoning-effort policy. See
//! [`resolve_stage_model_effort`] for the full four-tier chain this section
//! sits in.

use std::path::Path;

use crate::models::stage::{StageType, ALLOWED_REASONING_EFFORTS};
use crate::user_config::UserConfig;

use super::allowed::allowed_value;

/// Section name of the project-level stage model policy.
const MODELS_SECTION: &str = "models";

/// The `[models]` section of `.loom/work/config.toml`.
///
/// Every key is optional and the fallback is KEY-level, not section-level: a
/// present section that omits a key falls through to `~/.loom/config.toml`
/// rather than shadowing it whole. That differs deliberately from
/// `[terminal]`/`[context]` (see `read_terminal_config`), whose fallback is
/// section-level because their structs bake in derived defaults before a
/// caller can tell "set" from "derived" apart.
#[derive(Debug, Clone, Default, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ModelsConfig {
    standard_model: Option<String>,
    standard_effort: Option<String>,
    knowledge_model: Option<String>,
    knowledge_effort: Option<String>,
    knowledge_distill_model: Option<String>,
    knowledge_distill_effort: Option<String>,
    integration_verify_model: Option<String>,
    integration_verify_effort: Option<String>,
}

impl ModelsConfig {
    /// Parse a `[models]` body, e.g. `standard_model = "sonnet"` - the
    /// filesystem-free seam this module's tests build sections with. Nothing
    /// in a production build reaches it: `read_models_config` deserializes the
    /// section straight out of the file.
    #[cfg(test)]
    fn from_section_toml(body: &str) -> anyhow::Result<Self> {
        toml::from_str(body)
            .map_err(|e| anyhow::anyhow!("failed to parse [models] section body: {e}"))
    }

    /// The validated model this section sets for `stage_type`, if any.
    fn model_for(&self, stage_type: StageType) -> Option<&str> {
        let (value, key) = match stage_type {
            StageType::Standard => (&self.standard_model, "models.standard_model"),
            StageType::Knowledge => (&self.knowledge_model, "models.knowledge_model"),
            StageType::KnowledgeDistill => (
                &self.knowledge_distill_model,
                "models.knowledge_distill_model",
            ),
            StageType::IntegrationVerify => (
                &self.integration_verify_model,
                "models.integration_verify_model",
            ),
        };
        allowed_value(value.as_ref(), crate::claude::CLAUDE_MODELS, key)
    }

    /// The validated reasoning effort this section sets for `stage_type`, if any.
    fn effort_for(&self, stage_type: StageType) -> Option<&str> {
        let (value, key) = match stage_type {
            StageType::Standard => (&self.standard_effort, "models.standard_effort"),
            StageType::Knowledge => (&self.knowledge_effort, "models.knowledge_effort"),
            StageType::KnowledgeDistill => (
                &self.knowledge_distill_effort,
                "models.knowledge_distill_effort",
            ),
            StageType::IntegrationVerify => (
                &self.integration_verify_effort,
                "models.integration_verify_effort",
            ),
        };
        allowed_value(value.as_ref(), ALLOWED_REASONING_EFFORTS, key)
    }
}

/// Read `[models]`; a missing, unreadable or malformed section yields an
/// all-`None` config so a broken workspace file falls through to the user tier
/// instead of taking down a spawn (the stance `resolve_context_ceiling_tokens`
/// already takes).
fn read_models_config(work_dir: &Path) -> ModelsConfig {
    super::read_section::<ModelsConfig>(work_dir, MODELS_SECTION)
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// Pure resolution, filesystem-free: the seam this module's tests exercise
/// every tier of the chain through.
fn resolve_from(
    project: &ModelsConfig,
    user: &UserConfig,
    stage_type: StageType,
    stage_model: Option<&str>,
    stage_effort: Option<&str>,
) -> (String, String) {
    let model = stage_model
        .map(str::to_string)
        .or_else(|| project.model_for(stage_type).map(str::to_string))
        .unwrap_or_else(|| user.stage_model(stage_type).to_string());
    let effort = stage_effort
        .map(str::to_string)
        .or_else(|| project.effort_for(stage_type).map(str::to_string))
        .unwrap_or_else(|| user.stage_reasoning_effort(stage_type).to_string());
    (model, effort)
}

/// The (model, reasoning effort) a stage's agent session launches with.
///
/// ONE resolution order, and every reader of a stage's model or effort must
/// use it: the stage's own plan fields -> `.loom/work/config.toml` `[models]`
/// -> `~/.loom/config.toml` `[models]` -> `StageType`'s built-in. Skipping a
/// tier makes the dashboard and the spawn quote different models for one
/// session.
///
/// Takes the stage's own values rather than the `Stage` itself, so `fs/` keeps
/// no dependency on the stage model - the same shape
/// `resolve_context_ceiling_tokens` uses. Call it as
/// `resolve_stage_model_effort(work_dir, stage.stage_type, stage.model.as_deref(), stage.reasoning_effort.as_deref())`.
pub fn resolve_stage_model_effort(
    work_dir: &Path,
    stage_type: StageType,
    stage_model: Option<&str>,
    stage_effort: Option<&str>,
) -> (String, String) {
    let project = read_models_config(work_dir);
    let user = UserConfig::load();
    resolve_from(&project, &user, stage_type, stage_model, stage_effort)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_the_stage_type_built_in_when_nothing_is_set() {
        let project = ModelsConfig::default();
        let user = UserConfig::default();
        let cases = [
            (StageType::Standard, "opus", "high"),
            (StageType::Knowledge, "opus", "medium"),
            (StageType::KnowledgeDistill, "sonnet", "high"),
            (StageType::IntegrationVerify, "opus", "xhigh"),
        ];
        for (stage_type, model, effort) in cases {
            assert_eq!(
                resolve_from(&project, &user, stage_type, None, None),
                (model.to_string(), effort.to_string())
            );
        }
    }

    #[test]
    fn a_project_key_wins_over_the_built_in() {
        let project = ModelsConfig::from_section_toml(
            "standard_model = \"sonnet\"\nstandard_effort = \"low\"\n",
        )
        .unwrap();
        let user = UserConfig::default();
        assert_eq!(
            resolve_from(&project, &user, StageType::Standard, None, None),
            ("sonnet".to_string(), "low".to_string())
        );
    }

    #[test]
    fn a_plan_field_wins_over_a_project_key() {
        let project = ModelsConfig::from_section_toml(
            "standard_model = \"sonnet\"\nstandard_effort = \"low\"\n",
        )
        .unwrap();
        let user = UserConfig::default();
        assert_eq!(
            resolve_from(
                &project,
                &user,
                StageType::Standard,
                Some("haiku"),
                Some("xhigh")
            ),
            ("haiku".to_string(), "xhigh".to_string())
        );
    }

    #[test]
    fn a_project_key_absent_from_a_present_section_falls_through() {
        // Only standard_model is set; standard_effort must still resolve to
        // the built-in rather than being shadowed by the present section.
        let project = ModelsConfig::from_section_toml("standard_model = \"sonnet\"\n").unwrap();
        let user = UserConfig::default();
        assert_eq!(
            resolve_from(&project, &user, StageType::Standard, None, None),
            ("sonnet".to_string(), "high".to_string())
        );
    }

    #[test]
    fn an_out_of_set_project_value_is_ignored() {
        let project =
            ModelsConfig::from_section_toml("standard_model = \"nonexistent\"\n").unwrap();
        let user = UserConfig::default();
        assert_eq!(
            resolve_from(&project, &user, StageType::Standard, None, None),
            ("opus".to_string(), "high".to_string())
        );
    }

    #[test]
    fn project_tier_wins_over_user_tier_and_falls_through_key_by_key() {
        let temp = tempfile::TempDir::new().unwrap();
        let work_dir = temp.path().join("work");
        std::fs::create_dir_all(&work_dir).unwrap();
        std::fs::write(
            work_dir.join("config.toml"),
            "[models]\nstandard_model = \"sonnet\"\n",
        )
        .unwrap();

        let user_config_path = temp.path().join("user-config.toml");
        std::fs::write(
            &user_config_path,
            "[models]\nstandard_model = \"haiku\"\nstandard_effort = \"low\"\n",
        )
        .unwrap();
        let _redirect = crate::user_config::redirect_user_config(user_config_path);

        // The project's standard_model wins over the user config's.
        // standard_effort, which the project section omits, falls through to
        // the user config rather than the built-in.
        assert_eq!(
            resolve_stage_model_effort(&work_dir, StageType::Standard, None, None),
            ("sonnet".to_string(), "low".to_string())
        );
    }
}
