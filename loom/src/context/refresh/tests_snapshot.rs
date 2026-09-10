use super::*;
use crate::context::graph_store::GraphStore;
use crate::context::local_overlay::local_overlay_key;
use crate::context::store::ContextStore;
use serial_test::serial;
use std::time::Duration;
use tempfile::TempDir;

fn git_ok(root: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", root.join(".loom-test-no-global"))
        .env("GIT_CONFIG_SYSTEM", root.join(".loom-test-no-system"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    git_ok(root, &["init", "-b", "main"]);
    git_ok(root, &["config", "user.email", "t@t.com"]);
    git_ok(root, &["config", "user.name", "t"]);
    std::fs::write(root.join("src.rs"), "fn committed() {}\n").unwrap();
    git_ok(root, &["add", "src.rs"]);
    git_ok(root, &["commit", "-m", "seed"]);
    temp
}

fn stores(temp: &TempDir) -> (ContextStore, GraphStore) {
    let state_root = temp.path().join(".loom");
    let store = ContextStore::with_root(state_root.join("cache/context-v1"));
    let graph_store = GraphStore::new(store.root(), &state_root.join("work"));
    (store, graph_store)
}

#[test]
#[serial]
fn base_is_built_once_then_reused_without_counters() {
    let temp = init_repo();
    let root = temp.path();
    let (store, graph_store) = stores(&temp);

    let first = ensure_snapshot(&store, &graph_store, root, SnapshotPolicy::BaseOnly).unwrap();
    let second = ensure_snapshot(&store, &graph_store, root, SnapshotPolicy::BaseOnly).unwrap();

    assert_eq!(first.action, SnapshotAction::Rebuilt);
    assert_eq!(second.action, SnapshotAction::Reused);
    assert_eq!(second.counters, SourceGraphCounters::default());
}

#[test]
#[serial]
fn unchanged_dirty_tree_reuses_the_current_local_overlay() {
    let temp = init_repo();
    let root = temp.path();
    let (store, graph_store) = stores(&temp);
    std::fs::write(root.join("src.rs"), "fn edited() {}\n").unwrap();

    let first = ensure_snapshot(&store, &graph_store, root, SnapshotPolicy::LocalCurrent).unwrap();
    let second = ensure_snapshot(&store, &graph_store, root, SnapshotPolicy::LocalCurrent).unwrap();

    assert_eq!(first.overlay, Some(local_overlay_key(root)));
    assert_ne!(first.action, SnapshotAction::Reused);
    assert_eq!(second.action, SnapshotAction::Reused);
    assert_eq!(second.counters, SourceGraphCounters::default());
}

#[test]
#[serial]
fn unchanged_stage_generation_reuses_the_overlay() {
    let temp = init_repo();
    let root = temp.path();
    let (store, graph_store) = stores(&temp);
    let policy = || SnapshotPolicy::StageOverlay {
        plan: "plan".to_string(),
        stage: "stage".to_string(),
    };

    ensure_snapshot(&store, &graph_store, root, policy()).unwrap();
    let second = ensure_snapshot(&store, &graph_store, root, policy()).unwrap();

    assert_eq!(second.action, SnapshotAction::Reused);
    assert_eq!(
        second.overlay,
        Some(("plan".to_string(), "stage".to_string()))
    );
}

#[test]
#[serial]
fn a_directory_without_git_is_unavailable_instead_of_erroring() {
    let temp = TempDir::new().unwrap();
    let (store, graph_store) = stores(&temp);

    let outcome = ensure_snapshot(
        &store,
        &graph_store,
        temp.path(),
        SnapshotPolicy::LocalCurrent,
    )
    .unwrap();

    assert_eq!(outcome.action, SnapshotAction::Unavailable);
    assert!(outcome
        .reason
        .contains("failed to inspect the working tree"));
}

/// Restores the working directory on drop, even if the test body panics -
/// `set_current_dir` is process-global, so a leaked temp cwd would corrupt
/// every later `#[serial]` test in this binary.
struct CwdGuard {
    original: std::path::PathBuf,
}

impl CwdGuard {
    fn new() -> Self {
        Self {
            original: std::env::current_dir().unwrap(),
        }
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.original).unwrap();
    }
}

/// `loom map` must answer in a checkout that never ran `loom init`: the
/// command owns its own snapshot ensure rather than requiring a
/// pre-populated `.loom/work`. `commands::map::execute` resolves its
/// project root from the process cwd (`WorkDir::new(".")`), so this drives
/// it from a real temp git repo with no state directory at all - the same
/// setup manually verified at the CLI (`loom map --outline src.rs` in a
/// fresh checkout).
/// The `MapArgs` an outline query needs, isolated from the fixture below so
/// the test that drives `loom map` stays focused on the checkout it runs in.
fn outline_src_rs_args() -> crate::commands::map::MapArgs {
    use crate::commands::map::MapArgs;
    use crate::context::source_graph::SourceEdgeKind;

    MapArgs {
        outline: Some("src.rs".to_string()),
        find_all: None,
        impact: None,
        callers: None,
        callees: None,
        depth: 3,
        kinds: vec![
            SourceEdgeKind::Contains,
            SourceEdgeKind::Imports,
            SourceEdgeKind::Calls,
            SourceEdgeKind::References,
            SourceEdgeKind::Implements,
            SourceEdgeKind::Extends,
        ],
        limit: 50,
        path: None,
        min_confidence: 0.0,
        json: true,
    }
}

#[test]
#[serial]
fn map_answers_in_a_checkout_that_never_ran_init() {
    use crate::commands::map::execute;

    let temp = init_repo();
    let root = temp.path();
    let _guard = CwdGuard::new();
    std::env::set_current_dir(root).unwrap();

    assert!(
        !root.join(".loom").join("work").exists(),
        "the fixture must start with no loom state; that absence is the \
         behavior under test"
    );

    execute(outline_src_rs_args()).expect("`loom map` must succeed with no .loom/work present");

    // `execute` prints its views rather than returning them, so assert the
    // observable side effect instead: a base snapshot must now be published
    // under the state directory it just created, for the HEAD it ran at.
    let head = working_tree(root).unwrap().head;
    let (_store, graph_store) = stores(&temp);
    let published = graph_store
        .load_base(&head)
        .unwrap()
        .expect("running `loom map` in a checkout with no `.loom/work` must publish a base");
    assert!(
        published.files.contains_key("src.rs"),
        "the outlined file must be represented in the published base: {published:?}"
    );
}

fn reused_base_outcome() -> SnapshotOutcome {
    SnapshotOutcome {
        action: SnapshotAction::Reused,
        reason: "base for abc12345 present; tree clean".to_string(),
        revision: "abc123456789".to_string(),
        generation: clean_generation("abc123456789"),
        overlay: None,
        counters: SourceGraphCounters::default(),
        elapsed: Duration::ZERO,
    }
}

fn updated_overlay_outcome() -> SnapshotOutcome {
    SnapshotOutcome {
        action: SnapshotAction::Updated,
        reason: "local overlay refreshed".to_string(),
        revision: "abc123456789".to_string(),
        generation: "dirty".to_string(),
        overlay: Some(("_local".to_string(), "map-loom".to_string())),
        counters: SourceGraphCounters {
            files_parsed: 3,
            files_reused: 1_571,
            files_deleted: 2,
            ..Default::default()
        },
        elapsed: Duration::from_millis(410),
    }
}

fn rebuilt_base_outcome() -> SnapshotOutcome {
    SnapshotOutcome {
        action: SnapshotAction::Rebuilt,
        reason: "base refreshed".to_string(),
        revision: "abc123456789".to_string(),
        generation: clean_generation("abc123456789"),
        overlay: None,
        counters: SourceGraphCounters {
            files_parsed: 1_575,
            ..Default::default()
        },
        elapsed: Duration::from_millis(12_300),
    }
}

fn unavailable_outcome() -> SnapshotOutcome {
    SnapshotOutcome {
        action: SnapshotAction::Unavailable,
        reason: "no HEAD".to_string(),
        revision: String::new(),
        generation: String::new(),
        overlay: None,
        counters: SourceGraphCounters::default(),
        elapsed: Duration::from_millis(20),
    }
}

#[test]
fn describe_uses_the_shared_advisory_shapes() {
    assert_eq!(
        reused_base_outcome().describe(),
        "source graph: reused base abc12345 (tree clean)"
    );
    assert_eq!(
        updated_overlay_outcome().describe(),
        "source graph: updated local overlay _local/map-loom (3 parsed, 1571 reused, 2 deleted; 0.41s)"
    );
    assert_eq!(
        rebuilt_base_outcome().describe(),
        "source graph: rebuilt base abc12345 (1575 parsed; 12.3s)"
    );
    assert_eq!(
        unavailable_outcome().describe(),
        "source graph: unavailable (no HEAD)"
    );
}
