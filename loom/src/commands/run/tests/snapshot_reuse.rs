//! Snapshot reuse regressions across the two real call sites
//! (`advisory_source_graph_preflight`'s base-only policy and `refresh`'s
//! local-current policy): unchanged trees, documentation-only commits,
//! source commits, a cleaned state dir, and an extractor-version bump all
//! reuse or re-parse exactly the files they should.

use super::*;
use serial_test::serial;

#[test]
#[serial]
fn sync_then_init_on_an_unchanged_clean_tree_reuses_without_reading() {
    use crate::context::refresh::{SnapshotAction, SnapshotPolicy};

    let temp = init_preflight_repo();
    let root = temp.path();
    let (_work_dir, store, graph_store) = snapshot_stores(root);
    ensure(root, &store, &graph_store, SnapshotPolicy::LocalCurrent);

    let init = ensure(root, &store, &graph_store, SnapshotPolicy::BaseOnly);

    assert_eq!(init.action, SnapshotAction::Reused);
    assert_eq!(init.counters.files_hashed, 0);
}

#[test]
#[serial]
fn sync_then_init_on_an_unchanged_dirty_tree_reuses_the_current_overlay() {
    use crate::context::local_overlay::local_overlay_key;
    use crate::context::refresh::{SnapshotAction, SnapshotPolicy};

    let temp = init_preflight_repo();
    let root = temp.path();
    let (_work_dir, store, graph_store) = snapshot_stores(root);
    fs::write(root.join("src.rs"), "fn dirty_symbol() {}\n").unwrap();
    ensure(root, &store, &graph_store, SnapshotPolicy::LocalCurrent);
    let (plan, stage) = local_overlay_key(root);
    let before = graph_store.load_overlay(&plan, &stage).unwrap().unwrap();

    let init = ensure(root, &store, &graph_store, SnapshotPolicy::BaseOnly);
    let after_init = graph_store.load_overlay(&plan, &stage).unwrap().unwrap();
    let query = ensure(root, &store, &graph_store, SnapshotPolicy::LocalCurrent);

    assert_eq!(init.action, SnapshotAction::Reused);
    assert_eq!(before, after_init);
    assert_eq!(query.action, SnapshotAction::Reused);
}

#[test]
#[serial]
fn a_documentation_only_commit_publishes_a_new_base_reusing_every_source_entry() {
    use crate::context::refresh::{SnapshotAction, SnapshotPolicy};

    let temp = init_preflight_repo();
    let root = temp.path();
    let (_work_dir, store, graph_store) = snapshot_stores(root);
    ensure(root, &store, &graph_store, SnapshotPolicy::BaseOnly);
    run_git(
        root,
        &["commit", "--allow-empty", "-m", "documentation only"],
    );

    let outcome = ensure(root, &store, &graph_store, SnapshotPolicy::BaseOnly);

    assert_eq!(outcome.action, SnapshotAction::Updated);
    assert_eq!(outcome.counters.files_parsed, 0);
    assert_eq!(outcome.counters.files_reused, 1);
}

#[test]
#[serial]
fn a_source_commit_parses_only_the_changed_file() {
    use crate::context::refresh::SnapshotPolicy;

    let temp = init_preflight_repo();
    let root = temp.path();
    let (_work_dir, store, graph_store) = snapshot_stores(root);
    ensure(root, &store, &graph_store, SnapshotPolicy::BaseOnly);
    fs::write(root.join("src.rs"), "fn changed_symbol() {}\n").unwrap();
    run_git(root, &["add", "src.rs"]);
    run_git(root, &["commit", "-m", "source change"]);

    let outcome = ensure(root, &store, &graph_store, SnapshotPolicy::BaseOnly);

    assert_eq!(outcome.counters.files_parsed, 1);
}

#[test]
#[serial]
fn explicit_cleanup_keeps_the_base_and_reuses_it() {
    use crate::context::refresh::{SnapshotAction, SnapshotPolicy};

    let temp = init_preflight_repo();
    let root = temp.path();
    let (work_dir, store, graph_store) = snapshot_stores(root);
    ensure(root, &store, &graph_store, SnapshotPolicy::BaseOnly);
    fs::remove_dir_all(work_dir.root()).unwrap();
    let (_new_work_dir, new_store, new_graph_store) = snapshot_stores(root);

    let outcome = ensure(root, &new_store, &new_graph_store, SnapshotPolicy::BaseOnly);

    assert_eq!(outcome.action, SnapshotAction::Reused);
}

#[test]
#[serial]
fn an_extractor_change_at_the_same_head_reparses() {
    use crate::context::refresh::{SnapshotAction, SnapshotPolicy};
    use crate::context::store::canonical_json;

    let temp = init_preflight_repo();
    let root = temp.path();
    let (_work_dir, store, graph_store) = snapshot_stores(root);
    ensure(root, &store, &graph_store, SnapshotPolicy::BaseOnly);
    let head = head_sha(root);
    let mut base = graph_store.load_base(&head).unwrap().unwrap();
    for entry in base.files.values_mut() {
        for node in &mut entry.nodes {
            node.parser_version.push_str("-obsolete");
        }
    }
    fs::write(graph_store.base_path(&head), canonical_json(&base).unwrap()).unwrap();

    let outcome = ensure(root, &store, &graph_store, SnapshotPolicy::BaseOnly);

    assert_eq!(outcome.action, SnapshotAction::Rebuilt);
    assert_eq!(outcome.counters.files_parsed, 1);
}

#[test]
#[serial]
fn a_dirty_checkout_yields_a_base_with_the_committed_symbol_and_a_local_overlay_with_the_edit() {
    use crate::context::local_overlay::local_overlay_key;
    use crate::context::refresh::SnapshotPolicy;

    let temp = init_preflight_repo();
    let root = temp.path();
    let (_work_dir, store, graph_store) = snapshot_stores(root);
    fs::write(root.join("src.rs"), "fn edited_symbol() {}\n").unwrap();

    let outcome = ensure(root, &store, &graph_store, SnapshotPolicy::LocalCurrent);
    let base = graph_store.load_base(&outcome.revision).unwrap().unwrap();
    let (plan, stage) = local_overlay_key(root);
    let resolved = graph_store
        .resolved(&outcome.revision, Some((&plan, &stage)))
        .unwrap();

    assert!(base.files["src.rs"]
        .nodes
        .iter()
        .any(|node| node.signature.contains("main")));
    assert!(resolved.files["src.rs"]
        .nodes
        .iter()
        .any(|node| node.signature.contains("edited_symbol")));
}
