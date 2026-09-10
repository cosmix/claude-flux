//! Tests for [`super::evaluate`]'s semantic-freshness comparison against
//! `git rev-parse HEAD` (`semantic_freshness_against_head`, `short_revision`).

use super::*;
use crate::context::store::StoreState;
use std::path::PathBuf;
use tempfile::TempDir;

/// Run one git command with ambient global/system config neutralized, so a
/// developer's or CI runner's `~/.gitconfig` cannot change test behavior —
/// mirrors `tests_source_graph.rs::isolated_git`.
fn isolated_git(root: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", root.join(".loom-test-no-global"))
        .env("GIT_CONFIG_SYSTEM", root.join(".loom-test-no-system"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap()
}

fn git_ok(root: &Path, args: &[&str]) {
    let out = isolated_git(root, args);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn head_sha(root: &Path) -> String {
    let out = isolated_git(root, &["rev-parse", "HEAD"]);
    assert!(out.status.success(), "rev-parse HEAD failed");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A git repo rooted at `temp.path()`, with `doc/loom/knowledge/` present so
/// `derive_project_root` recognizes the layout, and one committed file.
/// Returns `(repo, knowledge_root, first commit sha)`.
fn init_repo_with_knowledge_root() -> (TempDir, PathBuf, String) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    git_ok(&root, &["init", "-b", "main"]);
    git_ok(&root, &["config", "user.email", "t@t.com"]);
    git_ok(&root, &["config", "user.name", "t"]);

    let knowledge_root = root.join("doc/loom/knowledge");
    std::fs::create_dir_all(&knowledge_root).unwrap();
    std::fs::write(knowledge_root.join("architecture.md"), "# Architecture\n").unwrap();
    git_ok(&root, &["add", "doc"]);
    git_ok(&root, &["commit", "-m", "seed"]);

    let first = head_sha(&root);
    (temp, knowledge_root, first)
}

/// A `ContextStore` under `temp` with `state.json` pre-seeded to carry
/// `semantic_revision` and nothing else — structural stays default (empty,
/// never built), which is irrelevant to these tests.
fn seeded_store(temp: &TempDir, semantic_revision: &str) -> ContextStore {
    let store = ContextStore::with_root(temp.path().join("cache"));
    store
        .save_state(&StoreState {
            semantic: Freshness {
                revision: semantic_revision.to_string(),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
    store
}

#[test]
fn semantic_is_reported_stale_when_head_moves_past_the_stored_revision() {
    let (temp, knowledge_root, first) = init_repo_with_knowledge_root();
    let root = temp.path();

    std::fs::write(knowledge_root.join("patterns.md"), "# Patterns\n").unwrap();
    git_ok(root, &["add", "doc"]);
    git_ok(root, &["commit", "-m", "second"]);
    let second = head_sha(root);
    assert_ne!(first, second);

    let store = seeded_store(&temp, &first);

    let state = evaluate(&store, &knowledge_root).unwrap();

    assert!(state.semantic.stale);
    assert_eq!(
        state.semantic.revision, first,
        "revision must stay the STORED one — it names the base layer on disk"
    );
    let detail = state
        .semantic
        .detail
        .expect("a newly-stale semantic freshness needs a detail");
    assert!(
        detail.contains(&first[..8]),
        "detail must name the old short revision: {detail}"
    );
    assert!(
        detail.contains(&second[..8]),
        "detail must name the new short revision: {detail}"
    );
}

#[test]
fn semantic_stays_fresh_when_head_matches_the_stored_revision() {
    let (temp, knowledge_root, first) = init_repo_with_knowledge_root();
    let store = seeded_store(&temp, &first);

    let state = evaluate(&store, &knowledge_root).unwrap();

    assert!(!state.semantic.stale);
    assert_eq!(state.semantic.revision, first);
    assert!(state.semantic.detail.is_none());
}

#[test]
fn a_non_git_directory_passes_the_stored_semantic_freshness_through_unchanged() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let knowledge_root = root.join("doc/loom/knowledge");
    std::fs::create_dir_all(&knowledge_root).unwrap();
    std::fs::write(knowledge_root.join("architecture.md"), "# Architecture\n").unwrap();

    let store = ContextStore::with_root(root.join("cache"));
    let stored = Freshness {
        revision: "deadbeef".to_string(),
        stale: true,
        detail: Some("stale for an unrelated, pre-existing reason".to_string()),
        ..Default::default()
    };
    store
        .save_state(&StoreState {
            semantic: stored.clone(),
            ..Default::default()
        })
        .unwrap();

    let state = evaluate(&store, &knowledge_root).unwrap();

    assert_eq!(
        state.semantic, stored,
        "no git repository must leave the stored semantic freshness untouched"
    );
}

#[test]
fn an_empty_stored_semantic_revision_still_reports_never_built() {
    let (temp, knowledge_root, _first) = init_repo_with_knowledge_root();
    let store = ContextStore::with_root(temp.path().join("cache")); // no state.json written at all

    let state = evaluate(&store, &knowledge_root).unwrap();

    assert!(state.semantic.stale);
    assert!(state.semantic.revision.is_empty());
    assert_eq!(
        state.semantic.detail.as_deref(),
        Some("source graph not built; see the source-graph stage")
    );
}

#[test]
fn a_stored_revision_shorter_than_eight_characters_does_not_panic() {
    let (temp, knowledge_root, _first) = init_repo_with_knowledge_root();
    let store = seeded_store(&temp, "abc");

    let state = evaluate(&store, &knowledge_root).unwrap();

    assert!(state.semantic.stale);
    assert_eq!(state.semantic.revision, "abc");
    let detail = state
        .semantic
        .detail
        .expect("a newly-stale semantic freshness needs a detail");
    assert!(
        detail.contains("abc"),
        "a revision shorter than 8 chars must render whole, not panic: {detail}"
    );
}

#[test]
fn refresh_fingerprints_the_tree_once_per_call() {
    let (temp, knowledge_root, _first) = init_repo_with_knowledge_root();
    let store = ContextStore::with_root(temp.path().join("cache"));
    take_fingerprint_pass_count();

    refresh(&store, &knowledge_root, true).unwrap();

    assert_eq!(take_fingerprint_pass_count(), 1);
}
