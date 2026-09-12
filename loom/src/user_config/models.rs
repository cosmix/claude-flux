//! The `[models]` section of `~/.loom/config.toml`: the model and reasoning
//! effort each stage type's main-agent session runs on absent a plan field or
//! a project-tier override. Takes the same shape as `pressure.rs` for
//! `[pressure]`; split out of `mod.rs` to keep that file under Rule 17's
//! 400-line limit.

use anyhow::Result;
use toml_edit::DocumentMut;

use crate::models::stage::StageType;

use super::{parse, KeySpec, Origin, UserConfig};

/// The `[models]` section as the file set it. Eight keys, one model/effort
/// pair per [`StageType`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ModelsSection {
    standard_model: Option<String>,
    standard_effort: Option<String>,
    knowledge_model: Option<String>,
    knowledge_effort: Option<String>,
    knowledge_distill_model: Option<String>,
    knowledge_distill_effort: Option<String>,
    integration_verify_model: Option<String>,
    integration_verify_effort: Option<String>,
}

impl ModelsSection {
    /// Parse every `[models]` key out of `doc`, validating each against its
    /// value set.
    pub(super) fn parse(doc: &DocumentMut) -> Result<Self> {
        let efforts = crate::models::stage::ALLOWED_REASONING_EFFORTS;
        let models = crate::claude::CLAUDE_MODELS;
        Ok(Self {
            standard_model: parse::get_enum(doc, "models", "standard_model", models)?,
            standard_effort: parse::get_enum(doc, "models", "standard_effort", efforts)?,
            knowledge_model: parse::get_enum(doc, "models", "knowledge_model", models)?,
            knowledge_effort: parse::get_enum(doc, "models", "knowledge_effort", efforts)?,
            knowledge_distill_model: parse::get_enum(
                doc,
                "models",
                "knowledge_distill_model",
                models,
            )?,
            knowledge_distill_effort: parse::get_enum(
                doc,
                "models",
                "knowledge_distill_effort",
                efforts,
            )?,
            integration_verify_model: parse::get_enum(
                doc,
                "models",
                "integration_verify_model",
                models,
            )?,
            integration_verify_effort: parse::get_enum(
                doc,
                "models",
                "integration_verify_effort",
                efforts,
            )?,
        })
    }

    fn model_for(&self, stage_type: StageType) -> Option<&str> {
        match stage_type {
            StageType::Standard => self.standard_model.as_deref(),
            StageType::Knowledge => self.knowledge_model.as_deref(),
            StageType::KnowledgeDistill => self.knowledge_distill_model.as_deref(),
            StageType::IntegrationVerify => self.integration_verify_model.as_deref(),
        }
    }

    fn effort_for(&self, stage_type: StageType) -> Option<&str> {
        match stage_type {
            StageType::Standard => self.standard_effort.as_deref(),
            StageType::Knowledge => self.knowledge_effort.as_deref(),
            StageType::KnowledgeDistill => self.knowledge_distill_effort.as_deref(),
            StageType::IntegrationVerify => self.integration_verify_effort.as_deref(),
        }
    }
}

impl UserConfig {
    /// The model a `stage_type` main-agent session runs on absent a plan
    /// field and absent a project `[models]` section. Default: that stage
    /// type's [`StageType::default_model`].
    pub fn stage_model(&self, stage_type: StageType) -> &str {
        self.models
            .model_for(stage_type)
            .unwrap_or_else(|| stage_type.default_model())
    }

    /// The reasoning effort a `stage_type` main-agent session runs at absent
    /// a plan field and absent a project `[models]` section. Default: that
    /// stage type's [`StageType::default_reasoning_effort`].
    pub fn stage_reasoning_effort(&self, stage_type: StageType) -> &str {
        self.models
            .effort_for(stage_type)
            .unwrap_or_else(|| stage_type.default_reasoning_effort())
    }

    /// The rendered value and origin for a `models.*` key, or `None` when
    /// `spec` is not one — the arm `UserConfig::value_of` delegates to.
    pub(super) fn models_value_of(&self, spec: &KeySpec) -> Option<(String, Origin)> {
        let (stage_type, is_effort) = match spec.name {
            "models.standard_model" => (StageType::Standard, false),
            "models.standard_effort" => (StageType::Standard, true),
            "models.knowledge_model" => (StageType::Knowledge, false),
            "models.knowledge_effort" => (StageType::Knowledge, true),
            "models.knowledge_distill_model" => (StageType::KnowledgeDistill, false),
            "models.knowledge_distill_effort" => (StageType::KnowledgeDistill, true),
            "models.integration_verify_model" => (StageType::IntegrationVerify, false),
            "models.integration_verify_effort" => (StageType::IntegrationVerify, true),
            _ => return None,
        };
        let (value, set) = if is_effort {
            (
                self.stage_reasoning_effort(stage_type).to_string(),
                self.models.effort_for(stage_type),
            )
        } else {
            (
                self.stage_model(stage_type).to_string(),
                self.models.model_for(stage_type),
            )
        };
        Some((value, self.origin_of(set)))
    }

    /// The `[models]` block of `loom config --print`, every key resolved.
    /// Ends with a single newline — [`UserConfig::to_toml_string`] supplies
    /// the blank line separating sections.
    pub(super) fn models_toml(&self) -> String {
        format!(
            "[models]\nstandard_model = \"{}\"\nstandard_effort = \"{}\"\nknowledge_model = \"{}\"\nknowledge_effort = \"{}\"\nknowledge_distill_model = \"{}\"\nknowledge_distill_effort = \"{}\"\nintegration_verify_model = \"{}\"\nintegration_verify_effort = \"{}\"\n",
            self.stage_model(StageType::Standard),
            self.stage_reasoning_effort(StageType::Standard),
            self.stage_model(StageType::Knowledge),
            self.stage_reasoning_effort(StageType::Knowledge),
            self.stage_model(StageType::KnowledgeDistill),
            self.stage_reasoning_effort(StageType::KnowledgeDistill),
            self.stage_model(StageType::IntegrationVerify),
            self.stage_reasoning_effort(StageType::IntegrationVerify),
        )
    }
}
