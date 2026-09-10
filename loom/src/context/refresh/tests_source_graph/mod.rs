//! Shared fixtures for [`super::reconcile_source_graph`], [`super::mark_semantic_stale`],
//! and [`super::parser_version_matches`], split by theme across this
//! directory's submodules: enumeration, working-tree generation, layer
//! building and reuse/counters, and tombstones and freshness.

use super::*;
use crate::context::graph_store::FileEntry;
use crate::context::source_graph::{FileCoverage, NodeLanguage, SourceNode};
use crate::context::store::StoreState;
use tempfile::TempDir;

mod enumeration;
mod layer_reuse;
mod tombstones_freshness;
mod working_tree;

/// A single-node cache entry stamped with `parser_version`, for exercising
/// [`super::parser_version_matches`] directly without a full extraction.
fn entry_with_parser_version(path: &Path, parser_version: &str) -> FileEntry {
    let node: SourceNode = extract::file_node(
        path,
        b"irrelevant to version matching",
        NodeLanguage::Other("test".to_string()),
        parser_version.to_string(),
        &FileCoverage::Full,
    );
    FileEntry {
        content_hash: "irrelevant".to_string(),
        nodes: vec![node],
        edges: Vec::new(),
        coverage: FileCoverage::Full,
    }
}

/// Run one git command with ambient global/system config neutralized, so a
/// developer's or CI runner's `~/.gitconfig` cannot change test behavior.
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

/// Run one git setup command and assert it succeeded.
fn git_ok(root: &Path, args: &[&str]) {
    let out = isolated_git(root, args);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A temp git repo with two committed files: a `.rs` (which a real extractor
/// may claim) and a `.txt` (which none does, so it stays file-level-only).
fn init_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    git_ok(root, &["init", "-b", "main"]);
    git_ok(root, &["config", "user.email", "t@t.com"]);
    git_ok(root, &["config", "user.name", "t"]);

    std::fs::write(root.join("src.rs"), "fn main() {}\n").unwrap();
    std::fs::create_dir_all(root.join("docs")).unwrap();
    std::fs::write(root.join("docs").join("notes.txt"), "hello\n").unwrap();
    git_ok(root, &["add", "src.rs", "docs/notes.txt"]);
    git_ok(root, &["commit", "-m", "seed"]);

    temp
}

fn head_sha(root: &Path) -> String {
    let out = isolated_git(root, &["rev-parse", "HEAD"]);
    assert!(out.status.success(), "rev-parse HEAD failed");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn stores(temp: &TempDir) -> (ContextStore, GraphStore) {
    let state_root = temp.path().join(".loom");
    let store = ContextStore::with_root(state_root.join("cache/context-v1"));
    let graph_store = GraphStore::new(store.root(), &state_root.join("work"));
    (store, graph_store)
}

/// A `Freshness` carrying only a recognizable `revision`, for state.json
/// lost-update regression tests that need distinct per-field values.
fn freshness(revision: &str) -> Freshness {
    Freshness {
        revision: revision.to_string(),
        ..Default::default()
    }
}

/// A `StoreState` with distinct, recognizable revisions in each field, for
/// tests that assert one field survives a locked update to another.
fn seeded_state(structural: &str, semantic: &str, catalog_revision: &str) -> StoreState {
    StoreState {
        structural: freshness(structural),
        semantic: freshness(semantic),
        catalog_revision: catalog_revision.to_string(),
    }
}

fn overlay_scope(name: &str) -> SourceGraphScope {
    SourceGraphScope::Overlay {
        plan: "plan-contract".to_string(),
        stage: name.to_string(),
    }
}

fn base_scope(root: &Path) -> SourceGraphScope {
    SourceGraphScope::Base {
        revision: head_sha(root),
    }
}

fn layer_mentions(layer: &GraphLayer, path: &str, needle: &str) -> bool {
    layer.files.get(path).is_some_and(|entry| {
        entry
            .nodes
            .iter()
            .any(|node| node.signature.contains(needle))
    })
}

/// [`init_repo`] plus the `doc/loom/knowledge/` tree `refresh` requires: it
/// derives the project root by walking three ancestors up from the knowledge
/// root and refuses to guess when that layout does not match.
fn init_repo_with_knowledge() -> TempDir {
    let temp = init_repo();
    let root = temp.path();
    let knowledge = root.join("doc").join("loom").join("knowledge");
    std::fs::create_dir_all(&knowledge).unwrap();
    std::fs::write(
        knowledge.join("architecture.md"),
        "# Architecture\n\nOne section, so the catalog has something to ingest.\n",
    )
    .unwrap();
    git_ok(root, &["add", "doc/loom/knowledge/architecture.md"]);
    git_ok(root, &["commit", "-m", "knowledge"]);
    // `refresh` resolves the graph store through `WorkDir::new(project_root)`,
    // which yields `<root>/.loom/work` once that directory exists. Creating it
    // here pins the layer location instead of letting the upward search find
    // some ancestor's `.loom/work`.
    std::fs::create_dir_all(root.join(".loom").join("work")).unwrap();
    temp
}

/// The store and graph store `refresh` itself will construct for `root`, so a
/// test reads back the layer that the real call actually wrote.
fn refresh_stores(root: &Path) -> (ContextStore, GraphStore) {
    let store = ContextStore::with_root(root.join(".loom/cache/context-v1"));
    store.ensure().unwrap();
    let graph_store = GraphStore::new(store.root(), &root.join(".loom").join("work"));
    (store, graph_store)
}
