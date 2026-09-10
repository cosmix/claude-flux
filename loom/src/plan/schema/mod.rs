//! Plan YAML schema definitions and validation

mod detect;
mod structural_checks;
mod types;
mod validation;
mod validation_suite;

#[cfg(test)]
mod tests;

pub use detect::{detect_stage_type, detect_stage_type_from_id_name};
pub(crate) use structural_checks::extract_brief_paths;
pub use types::{
    AcceptanceCriterion, AdjudicationConfig, ChangeImpactConfig, ChangeImpactPolicy,
    CodeReviewConfig, CommandConfinement, DeadCodeCheck, FilesystemConfig, Implementer,
    Implementers, LinuxConfig, LoomConfig, LoomMetadata, NetworkConfig, PermissionMode,
    RegressionTest, SandboxConfig, StageDefinition, StageSandboxConfig, StageType, SuccessCriteria,
    TruthCheck, ValidationError, WiringCheck, WiringTest, ALLOWED_REASONING_EFFORTS,
};
pub use validation::{
    check_knowledge_recommendations, check_sandbox_recommendations, unsafe_plan_reasons, validate,
    validate_structural_preflight,
};
