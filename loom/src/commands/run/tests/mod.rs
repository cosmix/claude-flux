//! Shared fixtures for the run command tests, split into preflight behavior
//! and the snapshot reuse-regression suite.

use crate::fs::work_dir::WorkDir;
use crate::models::stage::Stage;
use crate::plan::schema::{
    Implementers, LoomConfig, LoomMetadata, SandboxConfig, StageDefinition, StageSandboxConfig,
};
use crate::verify::serialize_stage_to_markdown;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

mod preflight;
mod snapshot_reuse;

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

/// Run one git setup command with ambient global/system config neutralized, so
/// a developer's or CI runner's `~/.gitconfig` cannot change test behaviour.
fn run_git(root: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", root.join(".loom-test-no-global"))
        .env("GIT_CONFIG_SYSTEM", root.join(".loom-test-no-system"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A temp git repo with one committed file and an initialised `.loom/work/`,
/// as the preflight expects to find.
fn init_preflight_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    run_git(root, &["init", "-b", "main"]);
    run_git(root, &["config", "user.email", "t@t.com"]);
    run_git(root, &["config", "user.name", "t"]);
    fs::write(root.join("src.rs"), "fn main() {}\n").unwrap();
    run_git(root, &["add", "src.rs"]);
    run_git(root, &["commit", "-m", "seed"]);
    fs::create_dir_all(root.join(".loom").join("work")).unwrap();
    temp
}

fn head_sha(root: &Path) -> String {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", root.join(".loom-test-no-global"))
        .env("GIT_CONFIG_SYSTEM", root.join(".loom-test-no-system"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn snapshot_stores(
    root: &Path,
) -> (
    WorkDir,
    crate::context::store::ContextStore,
    crate::context::graph_store::GraphStore,
) {
    let work_dir = WorkDir::new(root).unwrap();
    let store = crate::context::store::ContextStore::open(&work_dir).unwrap();
    store.ensure().unwrap();
    let graph_store = crate::context::graph_store::GraphStore::new(store.root(), work_dir.root());
    (work_dir, store, graph_store)
}

fn ensure(
    root: &Path,
    store: &crate::context::store::ContextStore,
    graph_store: &crate::context::graph_store::GraphStore,
    policy: crate::context::refresh::SnapshotPolicy,
) -> crate::context::refresh::SnapshotOutcome {
    crate::context::refresh::ensure_snapshot(store, graph_store, root, policy)
}
