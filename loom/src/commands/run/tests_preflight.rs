//! Source graph startup preflight tests.

use crate::fs::work_dir::WorkDir;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

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
fn inputs_and_rename_are_committed_before_graph_publication_in_both_run_paths() {
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
        let inputs = source.find("require_committed_plan(").unwrap();
        assert!(
            inputs < rename && rename < preflight,
            "{label}: validate inputs, commit the rename, then publish the graph for that revision"
        );
    }
}
