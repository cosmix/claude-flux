//! Execution-graph/stage-loading plumbing, and the advisory source-graph
//! preflight: whether it runs, and what it publishes when it does.

use super::super::graph_loader::build_execution_graph;
use super::*;
use crate::fs::stage_loading::load_stages_from_work_dir;
use crate::orchestrator::OrchestratorResult;
use serial_test::serial;

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

#[test]
#[serial]
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

    super::super::checks::advisory_source_graph_preflight(root, &work_dir);

    assert_eq!(
        store.load_state().unwrap().semantic.revision,
        "sentinel-not-reconciled",
        "a base already published for HEAD must make the preflight a no-op; it reconciled instead"
    );
}

/// The headline behaviour of `advisory_source_graph_preflight`: on a clean
/// tree with no base published for HEAD yet, it publishes one, and that
/// layer describes real files rather than a zero-count degraded outcome
/// (`reconcile_source_graph` degrades silently on an inspection failure — see
/// `context/refresh/source_graph.rs` — so "a base layer exists" alone is not
/// enough). This is the base-only policy both `loom run` paths take.
#[test]
#[serial]
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

    super::super::checks::advisory_source_graph_preflight(root, &work_dir);

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

/// A dirty checkout still publishes a base from committed content. The
/// preflight remains base-only; semantic refresh owns local-overlay creation.
#[test]
#[serial]
fn preflight_on_a_dirty_tree_publishes_only_the_base() {
    use crate::context::graph_store::GraphStore;
    use crate::context::local_overlay::local_overlay_key;
    use crate::context::store::ContextStore;

    let temp = init_preflight_repo();
    let root = temp.path();
    let work_dir = WorkDir::new(root).unwrap();
    let store = ContextStore::open(&work_dir).unwrap();
    let graph_store = GraphStore::new(store.root(), work_dir.root());

    // Dirty a tracked file; the base must still come from committed content.
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

    super::super::checks::advisory_source_graph_preflight(root, &work_dir);

    assert!(
        graph_store.load_base(&head).unwrap().is_some(),
        "a dirty tree must publish the immutable base from committed content"
    );

    let project_root = work_dir.project_root().unwrap();
    let (plan, stage) = local_overlay_key(project_root);
    let overlay = graph_store.load_overlay(&plan, &stage).unwrap();
    assert!(
        overlay.is_none(),
        "the preflight must remain base-only; refresh publishes the overlay"
    );
}

/// STRUCTURAL guard, not an integration test, and deliberately so: both
/// insertion points are free functions with side effects and no injectable
/// seam, so the ordering cannot be observed at runtime without inventing one.
/// Rather than write a test whose name claims an ordering it cannot check,
/// this reads the two sources and pins the ordering textually.
#[test]
fn inputs_and_rename_are_committed_before_graph_publication_in_both_run_paths() {
    for (label, source) in [
        ("run/mod.rs", include_str!("../mod.rs")),
        ("run/foreground.rs", include_str!("../foreground.rs")),
    ] {
        let preflight = source
            .find("advisory_source_graph_preflight(")
            .unwrap_or_else(|| panic!("{label} must call advisory_source_graph_preflight"));
        let rename = source
            .find("mark_plan_in_progress(")
            .unwrap_or_else(|| panic!("{label} must call mark_plan_in_progress"));
        let inputs = source.find("require_committed_plan(").unwrap();
        assert!(
            inputs < rename && rename < preflight,
            "{label}: validate inputs, commit the rename, then publish the graph for that revision"
        );
    }
}
