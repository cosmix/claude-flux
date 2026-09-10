//! Tombstoning a deleted file, and the freshness/state.json bookkeeping
//! `reconcile_source_graph` and `refresh` share around it.

use super::*;
use crate::context::source_graph::FileCoverage;

#[test]
fn a_deleted_tracked_file_is_tombstoned_and_absent_from_the_resolved_graph() {
    let temp = init_repo();
    let root = temp.path();
    let (store, graph_store) = stores(&temp);
    let revision = head_sha(root);
    reconcile_source_graph(&store, &graph_store, root, base_scope(root)).unwrap();
    std::fs::remove_file(root.join("src.rs")).unwrap();

    let outcome =
        reconcile_source_graph(&store, &graph_store, root, overlay_scope("deleted")).unwrap();
    let overlay = graph_store
        .load_overlay("plan-contract", "deleted")
        .unwrap()
        .unwrap();
    let resolved = graph_store
        .resolved(&revision, Some(("plan-contract", "deleted")))
        .unwrap();

    assert_eq!(outcome.counters.files_deleted, 1);
    assert_eq!(
        overlay.files.get("src.rs").map(|entry| &entry.coverage),
        Some(&FileCoverage::Deleted)
    );
    assert!(!resolved.files.contains_key("src.rs"));
}

#[test]
fn mark_semantic_stale_sets_stale_and_detail_and_survives_no_prior_state() {
    let temp = TempDir::new().unwrap();
    let store = ContextStore::with_root(temp.path().join("cache"));

    mark_semantic_stale(&store, "sibling merge invalidated the semantic layer").unwrap();

    let state = store.load_state().unwrap();
    assert!(state.semantic.stale);
    assert_eq!(
        state.semantic.detail.as_deref(),
        Some("sibling merge invalidated the semantic layer")
    );
}

#[test]
fn rebuild_and_persist_and_persist_semantic_freshness_do_not_revert_each_others_fields() {
    // Regression test for the state.json lost-update bug: `rebuild_and_persist`
    // owns `structural`/`catalog_revision` and `persist_semantic_freshness` owns
    // `semantic` — run both real update paths against one seeded state and
    // confirm neither's locked read-modify-write reverts the field the other
    // just wrote (the old unlocked load-then-save clobbered whichever field the
    // caller's stale in-memory snapshot did not carry forward).
    let temp = TempDir::new().unwrap();
    let store = ContextStore::with_root(temp.path().join("cache"));
    let knowledge_root = temp.path().join("knowledge");
    std::fs::create_dir_all(&knowledge_root).unwrap();
    store
        .save_state(&seeded_state("", "seed-semantic", "seed-catalog"))
        .unwrap();

    // A stale `semantic` snapshot, as `evaluate` would have captured it before
    // a concurrent semantic update landed — the old bug wrote this straight
    // back to disk, reverting whatever `semantic` actually held.
    let stale_semantic = freshness("stale-snapshot-from-evaluate");
    let fingerprints = crate::context::fingerprint::fingerprint_tree(&knowledge_root).unwrap();
    crate::context::refresh::rebuild_and_persist(
        &store,
        &knowledge_root,
        stale_semantic,
        fingerprints,
    )
    .unwrap();

    let after_rebuild = store.load_state().unwrap();
    assert_eq!(
        after_rebuild.semantic.revision, "seed-semantic",
        "rebuild_and_persist must not overwrite `semantic` with its stale snapshot"
    );
    assert_ne!(
        after_rebuild.catalog_revision, "seed-catalog",
        "rebuild_and_persist must still update the field it owns"
    );

    persist_semantic_freshness(&store, "fresh-semantic-revision".to_string()).unwrap();

    let after_semantic = store.load_state().unwrap();
    assert_eq!(
        after_semantic.catalog_revision, after_rebuild.catalog_revision,
        "persist_semantic_freshness must not revert catalog_revision"
    );
    assert_eq!(after_semantic.semantic.revision, "fresh-semantic-revision");
}

#[test]
fn update_state_leaves_fields_the_closure_does_not_assign_untouched() {
    // Pins the `update_state` invariant directly, one level below the two
    // real call sites exercised above: a closure that assigns only one field
    // must not disturb any other field, because the read that seeds it is
    // fresh and inside the same lock as the write.
    let temp = TempDir::new().unwrap();
    let store = ContextStore::with_root(temp.path().join("cache"));

    let seeded = seeded_state("seed-structural", "seed-semantic", "seed-catalog");
    store.save_state(&seeded).unwrap();

    store
        .update_state(|state| state.catalog_revision = "updated-catalog".to_string())
        .unwrap();

    let after = store.load_state().unwrap();
    assert_eq!(after.catalog_revision, "updated-catalog");
    assert_eq!(
        after.structural, seeded.structural,
        "update_state must not disturb a field the closure did not assign"
    );
    assert_eq!(
        after.semantic, seeded.semantic,
        "update_state must not disturb a field the closure did not assign"
    );
}

#[test]
fn test_clean_tree_publishes_base() {
    let temp = init_repo_with_knowledge();
    let root = temp.path();
    let (store, graph_store) = refresh_stores(root);

    let outcome = crate::context::refresh::refresh(
        &store,
        &root.join("doc").join("loom").join("knowledge"),
        false,
    )
    .unwrap();

    let head = head_sha(root);
    match &outcome.semantic.layer {
        crate::context::refresh::SemanticLayer::Base { revision } => assert_eq!(*revision, head),
        other => panic!("a clean tree must publish a base layer, got {other:?}"),
    }
    assert!(
        outcome.semantic.counters.files_enumerated > 0 && outcome.semantic.nodes > 0,
        "a base publish must report the layer it actually built, got {:?}",
        outcome.semantic
    );
    assert!(
        graph_store.load_base(&head).unwrap().is_some(),
        "the base layer must be readable back at the revision it was published for"
    );
    assert!(
        std::fs::read_dir(graph_store.base_dir())
            .unwrap()
            .next()
            .is_some(),
        "a base publish must leave a layer file under graph/base/"
    );
}

#[test]
fn test_dirty_tree_falls_back_to_local_overlay() {
    let temp = init_repo_with_knowledge();
    let root = temp.path();
    let (store, _graph_store) = refresh_stores(root);

    // Modify a tracked file so refresh publishes both committed and working snapshots.
    std::fs::write(root.join("src.rs"), "fn main() { let x = 1; }\n").unwrap();

    let outcome = crate::context::refresh::refresh(
        &store,
        &root.join("doc").join("loom").join("knowledge"),
        false,
    )
    .unwrap();

    let (expected_plan, expected_stage) = crate::context::local_overlay::local_overlay_key(root);
    match &outcome.semantic.layer {
        crate::context::refresh::SemanticLayer::BaseAndLocalOverlay {
            revision,
            plan,
            stage,
        } => {
            assert_eq!(*revision, head_sha(root));
            assert_eq!(*plan, expected_plan);
            assert_eq!(*stage, expected_stage);
        }
        other => panic!("a dirty tree must publish a base and local overlay, got {other:?}"),
    }
    assert!(
        outcome.semantic.counters.files_enumerated > 0 && outcome.semantic.nodes > 0,
        "the reconcile must report the snapshots it built: {:?}",
        outcome.semantic
    );
}

#[test]
fn test_dirty_tree_overlay_is_readable_through_local_scope() {
    // THE REGRESSION GUARD FOR THE WHOLE FALLBACK. Writing an overlay nobody
    // can read is indistinguishable from writing nothing, so this resolves the
    // graph the way retrieval does - through `local_overlay_key` - and proves
    // the address the writer used is the address the reader looks at.
    let temp = init_repo_with_knowledge();
    let root = temp.path();
    let (store, graph_store) = refresh_stores(root);

    std::fs::write(root.join("src.rs"), "fn main() { let x = 1; }\n").unwrap();

    let outcome = crate::context::refresh::refresh(
        &store,
        &root.join("doc").join("loom").join("knowledge"),
        false,
    )
    .unwrap();

    assert!(
        graph_store.load_base(&head_sha(root)).unwrap().is_some(),
        "the committed base must be published even while the working tree is dirty"
    );

    let (plan, stage) = crate::context::local_overlay::local_overlay_key(root);
    let resolved = graph_store
        .resolved(
            &outcome.semantic.freshness.revision,
            Some((plan.as_str(), stage.as_str())),
        )
        .unwrap();

    assert!(
        !resolved.files.is_empty(),
        "the overlay the dirty-tree fallback wrote must be readable through the local scope"
    );
    assert!(
        resolved.nodes().count() > 0,
        "a readable overlay with no nodes would still leave retrieval with nothing"
    );
}

#[test]
fn test_structural_only_skips_semantic_layer() {
    let temp = init_repo_with_knowledge();
    let root = temp.path();
    let (store, graph_store) = refresh_stores(root);

    let outcome = crate::context::refresh::refresh(
        &store,
        &root.join("doc").join("loom").join("knowledge"),
        true,
    )
    .unwrap();

    match &outcome.semantic.layer {
        crate::context::refresh::SemanticLayer::Skipped { reason } => assert!(
            reason.contains("structural-only"),
            "the skip must name the flag that caused it, got {reason:?}"
        ),
        other => panic!("--structural-only must not touch the semantic layer, got {other:?}"),
    }
    assert_eq!(outcome.semantic.counters.files_enumerated, 0);
    assert_eq!(outcome.semantic.nodes, 0);

    let (plan, stage) = crate::context::local_overlay::local_overlay_key(root);
    assert!(
        graph_store.load_base(&head_sha(root)).unwrap().is_none(),
        "--structural-only must publish no base layer"
    );
    assert!(
        graph_store.load_overlay(&plan, &stage).unwrap().is_none(),
        "--structural-only must write no overlay either"
    );
}
