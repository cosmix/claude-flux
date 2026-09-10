//! Which files a reconcile counts and represents: tracked, untracked,
//! unreadable, and symlinked (never followed).

use super::*;
use crate::context::source_graph::{FileCoverage, SourceNodeKind};
use std::os::unix::fs::PermissionsExt;

#[test]
fn overlay_scope_extracts_every_tracked_file_including_an_unsupported_language() {
    let temp = init_repo();
    let root = temp.path();
    let (store, graph_store) = stores(&temp);

    let scope = SourceGraphScope::Overlay {
        plan: "plan-a".to_string(),
        stage: "stage-a".to_string(),
    };
    let outcome = reconcile_source_graph(&store, &graph_store, root, scope).unwrap();

    assert_eq!(outcome.counters.files_parsed, 2);
    assert!(!outcome.freshness.stale);

    let overlay = graph_store
        .load_overlay("plan-a", "stage-a")
        .unwrap()
        .unwrap();
    assert!(overlay.files.contains_key("src.rs"));

    let txt_entry = overlay
        .files
        .get("docs/notes.txt")
        .expect("the .txt file must still be represented");
    assert!(
        txt_entry
            .nodes
            .iter()
            .any(|node| node.kind == SourceNodeKind::File),
        "a file with no extractor must still keep a file-level node"
    );
}

#[test]
fn an_untracked_source_file_is_indexed_in_the_overlay_and_absent_from_the_base() {
    let temp = init_repo();
    let root = temp.path();
    let (store, graph_store) = stores(&temp);
    let revision = head_sha(root);
    reconcile_source_graph(&store, &graph_store, root, base_scope(root)).unwrap();
    std::fs::write(root.join("new_source.rs"), "fn untracked_symbol() {}\n").unwrap();

    let outcome =
        reconcile_source_graph(&store, &graph_store, root, overlay_scope("untracked")).unwrap();
    let base = graph_store.load_base(&revision).unwrap().unwrap();
    let overlay = graph_store
        .load_overlay("plan-contract", "untracked")
        .unwrap()
        .unwrap();

    assert_eq!(outcome.counters.files_untracked, 1);
    assert!(overlay.files.contains_key("new_source.rs"));
    assert!(!base.files.contains_key("new_source.rs"));
}

#[test]
fn an_unreadable_file_survives_as_a_reported_lexical_only_entry() {
    let temp = init_repo();
    let root = temp.path();
    let (store, graph_store) = stores(&temp);

    let restricted = root.join("src.rs");
    let original_perms = std::fs::metadata(&restricted).unwrap().permissions();
    std::fs::set_permissions(&restricted, std::fs::Permissions::from_mode(0o000)).unwrap();

    // Root (and some sandboxes - the DEFAULT in most CI containers) ignore
    // file permission bits entirely, in which case this environment cannot
    // exercise the unreadable-file path at all. Skip loudly rather than
    // silently `return`ing a green pass that asserted nothing, and make the
    // skip a hard failure when `LOOM_TEST_REQUIRE_UNREADABLE_FILE=1` is set,
    // mirroring `tests/e2e/tmux_backend.rs`'s `LOOM_E2E_REQUIRE_TMUX`.
    let still_readable = std::fs::read(&restricted).is_ok();

    let scope = SourceGraphScope::Overlay {
        plan: "plan-e".to_string(),
        stage: "stage-e".to_string(),
    };
    let outcome = reconcile_source_graph(&store, &graph_store, root, scope);

    std::fs::set_permissions(&restricted, original_perms).unwrap();

    if still_readable {
        if std::env::var("LOOM_TEST_REQUIRE_UNREADABLE_FILE").as_deref() == Ok("1") {
            panic!(
                "an_unreadable_file_survives_as_a_reported_lexical_only_entry: this \
                 environment does not enforce 0o000 file permissions (running as root, or a \
                 sandbox that ignores mode bits), so the unreadable-file path was never \
                 exercised (LOOM_TEST_REQUIRE_UNREADABLE_FILE=1 demands a real run)"
            );
        }
        eprintln!(
            "SKIP an_unreadable_file_survives_as_a_reported_lexical_only_entry: this \
             environment does not enforce 0o000 file permissions (running as root, or a \
             sandbox that ignores mode bits), so the unreadable-file path was never \
             exercised (set LOOM_TEST_REQUIRE_UNREADABLE_FILE=1 to fail instead)"
        );
        return;
    }

    let outcome = outcome.unwrap();
    assert_eq!(
        outcome.counters.files_enumerated, 2,
        "the unreadable file must still be represented"
    );

    let overlay = graph_store
        .load_overlay("plan-e", "stage-e")
        .unwrap()
        .unwrap();
    let entry = overlay
        .files
        .get("src.rs")
        .expect("an unreadable file must not vanish from the layer");
    match &entry.coverage {
        FileCoverage::LexicalOnly { detail } => assert!(detail.contains("unreadable")),
        other => panic!("expected LexicalOnly coverage naming the failure, got {other:?}"),
    }
}

/// A function name that exists only in the file outside the repository a
/// planted symlink points at, so a node carrying it proves a read followed it.
const LEAKED_SYMBOL: &str = "leaked_secret_symbol";

/// Plant `root/<name>` as a symlink to a Rust file outside the repository
/// defining `LEAKED_SYMBOL`; the returned guard keeps that file alive.
fn plant_outside_symlink(root: &Path, name: &str) -> TempDir {
    let outside = TempDir::new().unwrap();
    let secret = outside.path().join("secret.rs");
    std::fs::write(&secret, format!("fn {LEAKED_SYMBOL}() {{}}\n")).unwrap();
    std::os::unix::fs::symlink(&secret, root.join(name)).unwrap();
    outside
}

fn leaked<'a>(mut nodes: impl Iterator<Item = &'a SourceNode>) -> bool {
    nodes.any(|node| node.signature.contains(LEAKED_SYMBOL) || node.id.contains(LEAKED_SYMBOL))
}

#[test]
fn a_committed_symlink_is_left_out_of_the_base_and_never_followed() {
    let temp = init_repo();
    let root = temp.path();
    let _outside = plant_outside_symlink(root, "leak.rs");
    git_ok(root, &["add", "leak.rs"]);
    git_ok(root, &["commit", "-m", "symlink"]);
    let (store, graph_store) = stores(&temp);

    let outcome = reconcile_source_graph(&store, &graph_store, root, base_scope(root)).unwrap();
    let base = graph_store.load_base(&head_sha(root)).unwrap().unwrap();

    assert!(!base.files.contains_key("leak.rs"));
    assert!(!leaked(base.nodes()));
    assert_eq!(
        outcome.counters.files_parsed, 2,
        "only src.rs and docs/notes.txt parse"
    );
    assert!(
        layer_mentions(&base, "src.rs", "main"),
        "a regular file must still parse"
    );
}

#[test]
fn an_untracked_symlink_is_left_out_of_the_overlay() {
    let temp = init_repo();
    let root = temp.path();
    let _outside = plant_outside_symlink(root, "leak.rs");
    let (store, graph_store) = stores(&temp);

    let outcome =
        reconcile_source_graph(&store, &graph_store, root, overlay_scope("link")).unwrap();
    let overlay = graph_store
        .load_overlay("plan-contract", "link")
        .unwrap()
        .unwrap();

    assert!(!leaked(overlay.nodes()));
    assert_eq!(
        outcome.counters.files_parsed, 2,
        "the symlink must not be parsed"
    );
    assert!(
        !overlay.files.contains_key("leak.rs"),
        "an untracked symlink must never enter the overlay, not even as an unreadable entry"
    );
    assert!(
        layer_mentions(&overlay, "src.rs", "main"),
        "a regular file must still parse"
    );
}

#[test]
fn a_committed_symlink_does_not_count_as_deleted() {
    let temp = init_repo();
    let root = temp.path();
    let _outside = plant_outside_symlink(root, "leak.rs");
    git_ok(root, &["add", "leak.rs"]);
    git_ok(root, &["commit", "-m", "symlink"]);
    let (store, graph_store) = stores(&temp);
    reconcile_source_graph(&store, &graph_store, root, base_scope(root)).unwrap();

    let outcome =
        reconcile_source_graph(&store, &graph_store, root, overlay_scope("clean")).unwrap();

    assert_eq!(
        outcome.counters.files_deleted, 0,
        "a committed symlink the base never tracked as a regular file must not tombstone"
    );
    assert_eq!(
        outcome.counters.files_enumerated, 2,
        "only src.rs and docs/notes.txt count as enumerated"
    );
}

#[test]
fn a_file_staged_as_a_symlink_is_tombstoned_over_the_base() {
    let temp = init_repo();
    let root = temp.path();
    let (store, graph_store) = stores(&temp);
    let revision = head_sha(root);
    reconcile_source_graph(&store, &graph_store, root, base_scope(root)).unwrap();
    std::fs::remove_file(root.join("src.rs")).unwrap();
    let _outside = plant_outside_symlink(root, "src.rs");
    git_ok(root, &["add", "src.rs"]);

    reconcile_source_graph(&store, &graph_store, root, overlay_scope("became-link")).unwrap();
    let resolved = graph_store
        .resolved(&revision, Some(("plan-contract", "became-link")))
        .unwrap();

    assert!(
        !resolved.files.contains_key("src.rs"),
        "the base's regular src.rs must not show through once the index tracks a symlink"
    );
    assert!(!leaked(resolved.nodes()));
}
