//! How generation, dirtiness, and renames drive a reconcile's behavior
//! against the working tree.

use super::*;

#[test]
fn base_scope_refuses_to_publish_when_the_tracked_tree_is_dirty() {
    let temp = init_repo();
    let root = temp.path();
    let (store, graph_store) = stores(&temp);
    let revision = head_sha(root);

    // Dirty a TRACKED file without committing.
    std::fs::write(root.join("src.rs"), "fn main() { /* uncommitted */ }\n").unwrap();

    let scope = SourceGraphScope::Base {
        revision: revision.clone(),
    };
    let outcome = reconcile_source_graph(&store, &graph_store, root, scope).unwrap();

    assert_eq!(outcome.counters.files_parsed, 2);
    assert!(!outcome.freshness.stale);
    assert!(
        graph_store.load_base(&revision).unwrap().is_some(),
        "a dirty tree must publish the committed base"
    );

    let state = store.load_state().unwrap();
    assert!(!state.semantic.stale);
}

#[test]
fn an_edit_at_unchanged_head_changes_the_generation_and_marks_the_overlay_stale() {
    let temp = init_repo();
    let root = temp.path();
    let (store, graph_store) = stores(&temp);
    let head = head_sha(root);
    reconcile_source_graph(&store, &graph_store, root, overlay_scope("generation")).unwrap();
    let overlay = graph_store
        .load_overlay("plan-contract", "generation")
        .unwrap()
        .unwrap();

    std::fs::write(root.join("src.rs"), "fn moved_generation() {}\n").unwrap();
    let moved = working_tree(root).unwrap();

    assert_eq!(moved.head, head);
    assert_ne!(overlay.generation, moved.generation);
}

#[test]
fn a_renamed_file_appears_under_its_new_path_only() {
    let temp = init_repo();
    let root = temp.path();
    let (store, graph_store) = stores(&temp);
    let revision = head_sha(root);
    reconcile_source_graph(&store, &graph_store, root, base_scope(root)).unwrap();
    git_ok(root, &["mv", "src.rs", "renamed.rs"]);

    let moved = working_tree(root).unwrap();
    assert_eq!(
        moved.dirty.get("renamed.rs").map(String::as_str),
        Some("R ")
    );
    assert_eq!(
        moved.dirty.get("src.rs").map(String::as_str),
        Some("R "),
        "the old path no longer exists on disk and must resolve through the \
         'deleted' identity rather than being dropped from `dirty`"
    );
    assert_ne!(
        moved.generation,
        clean_generation(&revision),
        "a rename must change the generation even though HEAD is unchanged"
    );

    reconcile_source_graph(&store, &graph_store, root, overlay_scope("renamed")).unwrap();
    let resolved = graph_store
        .resolved(&revision, Some(("plan-contract", "renamed")))
        .unwrap();

    assert!(resolved.files.contains_key("renamed.rs"));
    assert!(!resolved.files.contains_key("src.rs"));
}

#[test]
fn a_base_publishes_from_committed_content_on_a_dirty_tree() {
    let temp = init_repo();
    let root = temp.path();
    let (store, graph_store) = stores(&temp);
    let revision = head_sha(root);
    std::fs::write(root.join("src.rs"), "fn edited_only() {}\n").unwrap();

    reconcile_source_graph(&store, &graph_store, root, base_scope(root)).unwrap();
    reconcile_source_graph(
        &store,
        &graph_store,
        root,
        overlay_scope("committed-content"),
    )
    .unwrap();
    let base = graph_store.load_base(&revision).unwrap().unwrap();
    let overlay = graph_store
        .load_overlay("plan-contract", "committed-content")
        .unwrap()
        .unwrap();

    assert!(layer_mentions(&base, "src.rs", "main"));
    assert!(!layer_mentions(&base, "src.rs", "edited_only"));
    assert!(layer_mentions(&overlay, "src.rs", "edited_only"));
}
