//! Layer building, and when an entry is reused rather than re-parsed:
//! unchanged content, changed content, deleted-from-overlay pruning, and
//! parser-version staleness.

use super::*;
use crate::context::refresh::BoxedExtractor;
use crate::context::source_graph::body_hash;

#[test]
fn an_unchanged_overlay_rerun_writes_no_bytes() {
    let temp = init_repo();
    let root = temp.path();
    let (store, graph_store) = stores(&temp);
    let scope = || SourceGraphScope::Overlay {
        plan: "plan-b".to_string(),
        stage: "stage-b".to_string(),
    };

    reconcile_source_graph(&store, &graph_store, root, scope()).unwrap();
    let overlay_path = graph_store.overlay_path("plan-b", "stage-b");
    let before = std::fs::read(&overlay_path).unwrap();

    let second = reconcile_source_graph(&store, &graph_store, root, scope()).unwrap();
    let after = std::fs::read(&overlay_path).unwrap();

    assert_eq!(second.counters.bytes_serialized, 0);
    assert_eq!(
        before, after,
        "an unchanged incremental run must not rewrite the overlay"
    );
}

#[test]
fn base_scope_publishes_once_and_a_republish_is_refused_without_erroring() {
    let temp = init_repo();
    let root = temp.path();
    let (store, graph_store) = stores(&temp);
    let revision = head_sha(root);
    let scope = || SourceGraphScope::Base {
        revision: revision.clone(),
    };

    let first = reconcile_source_graph(&store, &graph_store, root, scope()).unwrap();
    assert_eq!(first.counters.files_parsed, 2);

    // A second build for the same revision must not error, even though the
    // base layer is already published and therefore immutable.
    let second = reconcile_source_graph(&store, &graph_store, root, scope()).unwrap();
    assert_eq!(second.counters.files_parsed, 0);
    assert_eq!(second.counters.files_reused, 2);

    let published = graph_store.load_base(&revision).unwrap().unwrap();
    assert_eq!(published.files.len(), 2);
}

#[test]
fn a_changed_file_is_re_extracted_and_the_new_content_lands_in_the_layer() {
    let temp = init_repo();
    let root = temp.path();
    let (store, graph_store) = stores(&temp);
    let scope = || SourceGraphScope::Overlay {
        plan: "plan-c".to_string(),
        stage: "stage-c".to_string(),
    };

    let first = reconcile_source_graph(&store, &graph_store, root, scope()).unwrap();
    assert_eq!(first.counters.files_parsed, 2);

    let new_bytes = b"a much longer body that is definitely not the seed content\n";
    std::fs::write(root.join("docs").join("notes.txt"), new_bytes).unwrap();

    let second = reconcile_source_graph(&store, &graph_store, root, scope()).unwrap();
    assert_eq!(second.counters.files_parsed, 1);
    assert_eq!(second.counters.files_reused, 1);

    let overlay = graph_store
        .load_overlay("plan-c", "stage-c")
        .unwrap()
        .unwrap();
    let entry = overlay.files.get("docs/notes.txt").unwrap();
    assert_eq!(
        entry.content_hash,
        body_hash(new_bytes),
        "a changed file must be re-extracted, not reused from the stale cache entry"
    );
}

#[test]
fn overlay_delta_prunes_files_identical_to_the_base_and_keeps_changed_ones() {
    let temp = init_repo();
    let root = temp.path();
    let (store, graph_store) = stores(&temp);
    let revision = head_sha(root);

    // Publish a base at the current (clean) revision.
    let base_scope = SourceGraphScope::Base {
        revision: revision.clone(),
    };
    reconcile_source_graph(&store, &graph_store, root, base_scope).unwrap();

    // Change one tracked file in the working tree without committing.
    std::fs::write(root.join("docs").join("notes.txt"), "changed\n").unwrap();

    let overlay_scope = SourceGraphScope::Overlay {
        plan: "plan-d".to_string(),
        stage: "stage-d".to_string(),
    };
    reconcile_source_graph(&store, &graph_store, root, overlay_scope).unwrap();

    let overlay = graph_store
        .load_overlay("plan-d", "stage-d")
        .unwrap()
        .unwrap();
    assert!(
        !overlay.files.contains_key("src.rs"),
        "an unchanged file must be pruned from the overlay delta"
    );
    assert!(
        overlay.files.contains_key("docs/notes.txt"),
        "a changed file must remain in the overlay delta"
    );
}

#[test]
fn a_documentation_only_commit_reuses_every_source_entry_without_reading_it() {
    let temp = init_repo();
    let root = temp.path();
    let (store, graph_store) = stores(&temp);
    reconcile_source_graph(&store, &graph_store, root, base_scope(root)).unwrap();
    git_ok(root, &["commit", "--allow-empty", "-m", "docs-only"]);

    let outcome = reconcile_source_graph(&store, &graph_store, root, base_scope(root)).unwrap();

    assert_eq!(outcome.counters.files_parsed, 0);
    assert_eq!(outcome.counters.files_hashed, 0);
    assert_eq!(outcome.counters.files_reused, 2);
}

#[test]
fn a_source_commit_parses_exactly_the_changed_file() {
    let temp = init_repo();
    let root = temp.path();
    let (store, graph_store) = stores(&temp);
    reconcile_source_graph(&store, &graph_store, root, base_scope(root)).unwrap();
    std::fs::write(root.join("src.rs"), "fn committed_change() {}\n").unwrap();
    git_ok(root, &["add", "src.rs"]);
    git_ok(root, &["commit", "-m", "source change"]);

    let outcome = reconcile_source_graph(&store, &graph_store, root, base_scope(root)).unwrap();

    assert_eq!(outcome.counters.files_parsed, 1);
    assert_eq!(outcome.counters.files_hashed, 1);
    assert_eq!(outcome.counters.files_reused, 1);
}

#[test]
fn an_extractor_version_change_reparses_everything_at_the_same_head() {
    let temp = init_repo();
    let root = temp.path();
    let (store, graph_store) = stores(&temp);
    reconcile_source_graph(&store, &graph_store, root, overlay_scope("parser-version")).unwrap();
    let mut overlay = graph_store
        .load_overlay("plan-contract", "parser-version")
        .unwrap()
        .unwrap();
    for entry in overlay.files.values_mut() {
        for node in &mut entry.nodes {
            node.parser_version.push_str("-obsolete");
        }
    }
    graph_store
        .save_overlay("plan-contract", "parser-version", &overlay)
        .unwrap();

    let outcome =
        reconcile_source_graph(&store, &graph_store, root, overlay_scope("parser-version"))
            .unwrap();

    assert_eq!(outcome.counters.files_parsed, 2);
    assert_eq!(outcome.counters.files_hashed, 2);
    assert_eq!(outcome.counters.files_reused, 0);
}

#[test]
fn counters_distinguish_parsed_from_reused() {
    let temp = init_repo();
    let root = temp.path();
    let (store, graph_store) = stores(&temp);
    let first =
        reconcile_source_graph(&store, &graph_store, root, overlay_scope("counters")).unwrap();
    let second =
        reconcile_source_graph(&store, &graph_store, root, overlay_scope("counters")).unwrap();

    assert_eq!(first.counters.files_parsed, 2);
    assert_eq!(first.counters.files_reused, 0);
    assert_eq!(second.counters.files_parsed, 0);
    assert_eq!(second.counters.files_reused, 2);
}

#[test]
fn a_lexical_fallback_node_stays_current_when_no_extractor_claims_the_path() {
    // Regression test: before the fix, `current` was `None` for a path no
    // extractor supports, and `None == Some(&node.parser_version)` was always
    // false — so every lexically-extracted file (.md, .txt, ...) was judged
    // stale and re-extracted on every incremental build, forever.
    let path = Path::new("docs/notes.txt");
    let entry = entry_with_parser_version(path, extract::lexical::LEXICAL_PARSER_VERSION);
    let extractors = extract::registry();

    assert!(
        parser_version_matches(&entry, &extractors, path),
        "a node that already took the lexical fallback must stay current \
         when no extractor claims the path today"
    );
}

#[test]
fn a_node_from_a_now_missing_extractor_is_still_treated_as_stale() {
    // Guards the case the lexical-fallback fix must not break: a cached node
    // produced by a real extractor (e.g. tree-sitter Rust) that is no longer
    // registered today (e.g. built with `--no-default-features`) must still
    // invalidate, even though no extractor claims the path either.
    let path = Path::new("src.rs");
    let entry = entry_with_parser_version(path, "rust-grammar+deadbeefcafe+v1");
    let extractors: Vec<BoxedExtractor> = Vec::new();

    assert!(
        !parser_version_matches(&entry, &extractors, path),
        "nodes from an extractor that is no longer registered must not be reused"
    );
}

#[test]
fn a_matching_version_from_a_present_extractor_is_current() {
    let path = Path::new("src.rs");
    let extractors = extract::registry();
    let current_version = extractors
        .iter()
        .find(|extractor| extractor.supports(path))
        .expect("the rust extractor must claim a .rs path")
        .cache_identity()
        .to_parser_version();
    let entry = entry_with_parser_version(path, &current_version);

    assert!(parser_version_matches(&entry, &extractors, path));
}

#[test]
fn a_mismatched_version_from_a_present_extractor_is_stale() {
    let path = Path::new("src.rs");
    let extractors = extract::registry();
    let current_version = extractors
        .iter()
        .find(|extractor| extractor.supports(path))
        .expect("the rust extractor must claim a .rs path")
        .cache_identity()
        .to_parser_version();
    let entry = entry_with_parser_version(path, &format!("{current_version}-stale"));

    assert!(!parser_version_matches(&entry, &extractors, path));
}

#[test]
fn an_entry_with_no_nodes_is_always_current() {
    let entry = FileEntry::default();
    let extractors = extract::registry();

    assert!(parser_version_matches(
        &entry,
        &extractors,
        Path::new("whatever.rs")
    ));
}
