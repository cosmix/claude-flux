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

#[test]
fn test_preflight_silent_when_base_exists() {
    use crate::context::graph_store::{GraphLayer, GraphStore};
    use crate::context::store::ContextStore;

    let temp = init_preflight_repo();
    let root = temp.path();
    let work_dir = WorkDir::new(root).unwrap();
    let store = ContextStore::open(&work_dir).unwrap();
    store.ensure().unwrap();
    let graph_store = GraphStore::new(store.root(), work_dir.root());

    let head = String::from_utf8_lossy(
        &std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_string();

    // A base already published for HEAD, plus a sentinel semantic revision in
    // the store's state. A reconcile would overwrite that revision with HEAD
    // (`persist_semantic_freshness`), so the sentinel surviving is what proves
    // the preflight short-circuited instead of walking the tree.
    graph_store
        .publish_base(
            &head,
            // Only `revision` matters to this test; the rest default.
            &GraphLayer {
                revision: head.clone(),
                ..Default::default()
            },
        )
        .unwrap();
    store
        .update_state(|state| state.semantic.revision = "sentinel-not-reconciled".to_string())
        .unwrap();

    super::checks::advisory_source_graph_preflight(root, &work_dir, false);

    assert_eq!(
        store.load_state().unwrap().semantic.revision,
        "sentinel-not-reconciled",
        "a base already published for HEAD must make the preflight a no-op; it reconciled instead"
    );
}

/// The headline behaviour of `advisory_source_graph_preflight`: on a clean
/// tree with no base published for HEAD yet, it publishes one, and that
/// layer describes real files rather than a zero-count degraded outcome
/// (`reconcile_source_graph` degrades silently on refusal — see
/// `context/refresh/source_graph.rs` — so "a base layer exists" alone is not
/// enough). This is the `allow_overlay_fallback=false` branch both `loom run`
/// paths take.
#[test]
fn test_preflight_publishes_a_base_layer_with_real_files_on_a_clean_tree() {
    use crate::context::graph_store::GraphStore;
    use crate::context::store::ContextStore;

    let temp = init_preflight_repo();
    let root = temp.path();
    let work_dir = WorkDir::new(root).unwrap();
    let store = ContextStore::open(&work_dir).unwrap();
    let graph_store = GraphStore::new(store.root(), work_dir.root());

    let head = String::from_utf8_lossy(
        &std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_string();

    assert!(
        graph_store.load_base(&head).unwrap().is_none(),
        "the fixture repo must start with no base layer published for HEAD"
    );

    super::checks::advisory_source_graph_preflight(root, &work_dir, false);

    let published = graph_store
        .load_base(&head)
        .unwrap()
        .expect("a clean tree with no existing base must publish one at HEAD");
    assert!(
        !published.files.is_empty(),
        "a published base with no extracted files is indistinguishable from \
         publishing nothing at all: {published:?}"
    );
    assert!(
        published.files.contains_key("src.rs"),
        "the committed fixture file must be represented in the published \
         layer: {published:?}"
    );
}

/// The `allow_overlay_fallback=true` branch — the one `loom init` takes, and
/// the one path with zero prior coverage: on a dirty tree (a base publish is
/// refused) it must fall back to the working-tree overlay at the SAME address
/// retrieval reads by default (`local_overlay_key`), and that overlay must
/// describe real extracted files, not an empty degraded layer. It must also
/// leave no base layer published for HEAD, since the base was refused, not
/// skipped.
#[test]
fn test_preflight_falls_back_to_local_overlay_on_a_dirty_tree_when_allowed() {
    use crate::context::graph_store::GraphStore;
    use crate::context::local_overlay::local_overlay_key;
    use crate::context::store::ContextStore;

    let temp = init_preflight_repo();
    let root = temp.path();
    let work_dir = WorkDir::new(root).unwrap();
    let store = ContextStore::open(&work_dir).unwrap();
    let graph_store = GraphStore::new(store.root(), work_dir.root());

    // Dirty a TRACKED file without committing: the refusal `dirty_tree_reason`
    // checks runs with `--untracked-files=no`, so an untracked scratch file
    // would not trigger it.
    fs::write(root.join("src.rs"), "fn main() { /* dirty */ }\n").unwrap();

    let head = String::from_utf8_lossy(
        &std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_string();

    super::checks::advisory_source_graph_preflight(root, &work_dir, true);

    assert!(
        graph_store.load_base(&head).unwrap().is_none(),
        "a dirty tree must never publish an immutable base layer for HEAD"
    );

    let project_root = work_dir.project_root().unwrap();
    let (plan, stage) = local_overlay_key(project_root);
    let overlay = graph_store
        .load_overlay(&plan, &stage)
        .unwrap()
        .expect("the dirty-tree fallback must write the working-tree overlay");
    assert!(
        !overlay.files.is_empty(),
        "the fallback overlay must describe real extracted files, not an \
         empty degraded layer: {overlay:?}"
    );
    assert!(
        overlay.files.contains_key("src.rs"),
        "the dirtied tracked file must appear in the overlay: {overlay:?}"
    );
}

/// STRUCTURAL guard, not an integration test, and deliberately so: both
/// insertion points are free functions with side effects and no injectable
/// seam, so the ordering cannot be observed at runtime without inventing one.
/// Rather than write a test whose name claims an ordering it cannot check,
/// this reads the two sources and pins the ordering textually.
#[test]
fn preflight_is_called_before_the_plan_rename_in_both_run_paths() {
    for (label, source) in [
        ("run/mod.rs", include_str!("mod.rs")),
        ("run/foreground.rs", include_str!("foreground.rs")),
    ] {
        let preflight = source
            .find("advisory_source_graph_preflight(")
            .unwrap_or_else(|| panic!("{label} must call advisory_source_graph_preflight"));
        let rename = source
            .find("mark_plan_in_progress(")
            .unwrap_or_else(|| panic!("{label} must call mark_plan_in_progress"));
        assert!(
            preflight < rename,
            "{label}: the source-graph preflight must run BEFORE mark_plan_in_progress - the \
             plan rename dirties a tracked file, and a base layer is refused on any dirty tree, \
             so a publish after the rename is refused every time"
        );
    }
}
