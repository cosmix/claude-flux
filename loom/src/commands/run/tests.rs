//! Tests for the run command module.

use super::graph_loader::build_execution_graph;
use crate::fs::stage_loading::load_stages_from_work_dir;
use crate::fs::work_dir::WorkDir;
use crate::models::stage::Stage;
use crate::orchestrator::OrchestratorResult;
use crate::plan::schema::{
    Implementers, LoomConfig, LoomMetadata, SandboxConfig, StageDefinition, StageSandboxConfig,
};
use crate::verify::serialize_stage_to_markdown;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Build markdown+frontmatter for a stage file the way `serialize_stage_to_markdown`
/// writes a real `<state-dir>/stages/*.md` file: a fully-populated [`Stage`] (every
/// runtime field present, e.g. `status`, `created_at`) with the given id/name.
/// `load_stages_from_work_dir` parses this shape, not a bare `StageDefinition`,
/// so fixtures here must be built this way rather than hand-writing plan-style
/// partial YAML.
fn stage_markdown(id: &str, name: &str) -> String {
    let mut stage = Stage::new(name.to_string(), None);
    stage.id = id.to_string();
    serialize_stage_to_markdown(&stage).unwrap()
}

fn create_test_plan(dir: &Path, stages: Vec<StageDefinition>) -> PathBuf {
    let metadata = LoomMetadata {
        loom: LoomConfig {
            version: 1,
            auto_merge: None,
            sandbox: SandboxConfig::default(),
            change_impact: None,
            adjudication: None,
            context_ceiling_tokens: None,
            subagent_ceiling_tokens: None,
            stages,
        },
    };

    let yaml = serde_yaml::to_string(&metadata).unwrap();
    let plan_content = format!(
        "# Test Plan\n\n## Overview\n\nTest plan\n\n<!-- loom METADATA -->\n```yaml\n{yaml}```\n<!-- END loom METADATA -->\n"
    );

    let plan_path = dir.join("test-plan.md");
    fs::write(&plan_path, plan_content).unwrap();
    plan_path
}

fn setup_work_dir_with_plan(temp_dir: &TempDir) -> (PathBuf, WorkDir) {
    let work_dir = WorkDir::new(temp_dir.path()).unwrap();
    work_dir.initialize().unwrap();

    let stage_def = StageDefinition {
        id: "test-stage".to_string(),
        name: "Test Stage".to_string(),
        description: None,
        dependencies: vec![],
        parallel_group: None,
        acceptance: vec![crate::plan::schema::AcceptanceCriterion::Simple(
            "echo ok".to_string(),
        )],
        setup: vec![],
        files: vec![],
        auto_merge: None,
        working_dir: ".".to_string(),
        stage_type: None,
        artifacts: vec![],
        wiring: vec![],
        wiring_tests: vec![],
        dead_code_check: None,
        before_stage: vec![],
        after_stage: vec![],
        context_ceiling_tokens: None,
        removed_context_budget: None,
        plan_overview: None,
        sandbox: StageSandboxConfig::default(),
        execution_mode: None,
        bug_fix: None,
        regression_test: None,
        model: None,
        reasoning_effort: None,
        code_review: None,
        ultracode: false,
        implementers: Implementers::default(),
        subagent_timeout_secs: None,
    };

    let plan_path = create_test_plan(temp_dir.path(), vec![stage_def]);

    let config_content = format!(
        "[plan]\nsource_path = \"{}\"\nplan_id = \"test-plan\"\nplan_name = \"Test Plan\"\n",
        plan_path.display()
    );
    fs::write(work_dir.root().join("config.toml"), config_content).unwrap();

    (plan_path, work_dir)
}

#[test]
fn test_build_execution_graph_no_config() {
    let temp_dir = TempDir::new().unwrap();
    let work_dir = WorkDir::new(temp_dir.path()).unwrap();
    work_dir.initialize().unwrap();

    let result = build_execution_graph(&work_dir);

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("No active plan"));
}

#[test]
fn test_build_execution_graph_from_config() {
    let temp_dir = TempDir::new().unwrap();
    let (_plan_path, work_dir) = setup_work_dir_with_plan(&temp_dir);

    let result = build_execution_graph(&work_dir);

    assert!(result.is_ok());
    let (_graph, _sandbox) = result.unwrap();
}

#[test]
fn test_build_execution_graph_missing_plan_file() {
    let temp_dir = TempDir::new().unwrap();
    let work_dir = WorkDir::new(temp_dir.path()).unwrap();
    work_dir.initialize().unwrap();

    let config_content =
        "[plan]\nsource_path = \"/nonexistent/plan.md\"\nplan_id = \"test\"\nplan_name = \"Test\"\n";
    fs::write(work_dir.root().join("config.toml"), config_content).unwrap();

    let result = build_execution_graph(&work_dir);

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn test_load_stages_from_work_dir_empty() {
    let temp_dir = TempDir::new().unwrap();
    let stages_dir = temp_dir.path().join("stages");
    fs::create_dir(&stages_dir).unwrap();

    let result = load_stages_from_work_dir(&stages_dir);

    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

#[test]
fn test_load_stages_from_work_dir_with_stages() {
    let temp_dir = TempDir::new().unwrap();
    let stages_dir = temp_dir.path().join("stages");
    fs::create_dir(&stages_dir).unwrap();

    let stage_content = stage_markdown("stage-1", "Test Stage");

    fs::write(stages_dir.join("0-stage-1.md"), stage_content).unwrap();

    let result = load_stages_from_work_dir(&stages_dir);

    assert!(result.is_ok());
    let stages = result.unwrap();
    assert_eq!(stages.len(), 1);
    assert_eq!(stages[0].id, "stage-1");
}

#[test]
fn test_load_stages_from_work_dir_ignores_non_markdown() {
    let temp_dir = TempDir::new().unwrap();
    let stages_dir = temp_dir.path().join("stages");
    fs::create_dir(&stages_dir).unwrap();

    fs::write(stages_dir.join("readme.txt"), "Not a stage").unwrap();

    let result = load_stages_from_work_dir(&stages_dir);

    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

#[test]
fn test_load_stages_from_work_dir_skips_invalid() {
    let temp_dir = TempDir::new().unwrap();
    let stages_dir = temp_dir.path().join("stages");
    fs::create_dir(&stages_dir).unwrap();

    let valid_stage = stage_markdown("valid", "Valid");
    fs::write(stages_dir.join("valid.md"), valid_stage).unwrap();
    fs::write(stages_dir.join("invalid.md"), "Invalid content").unwrap();

    let result = load_stages_from_work_dir(&stages_dir);

    assert!(result.is_ok());
    let stages = result.unwrap();
    assert_eq!(stages.len(), 1);
    assert_eq!(stages[0].id, "valid");
}

#[test]
fn test_orchestrator_result_success() {
    let result = OrchestratorResult {
        completed_stages: vec!["stage-1".to_string(), "stage-2".to_string()],
        failed_stages: vec![],
        needs_handoff: vec![],
        total_sessions_spawned: 2,
        started_at: chrono::Utc::now(),
        completed_at: chrono::Utc::now(),
    };

    assert!(result.is_success());
}

#[test]
fn test_orchestrator_result_with_failures() {
    let result = OrchestratorResult {
        completed_stages: vec!["stage-1".to_string()],
        failed_stages: vec!["stage-2".to_string()],
        needs_handoff: vec![],
        total_sessions_spawned: 2,
        started_at: chrono::Utc::now(),
        completed_at: chrono::Utc::now(),
    };

    assert!(!result.is_success());
}

#[test]
fn test_orchestrator_result_with_handoffs() {
    let result = OrchestratorResult {
        completed_stages: vec![],
        failed_stages: vec![],
        needs_handoff: vec!["stage-1".to_string()],
        total_sessions_spawned: 1,
        started_at: chrono::Utc::now(),
        completed_at: chrono::Utc::now(),
    };

    assert!(!result.is_success());
}
