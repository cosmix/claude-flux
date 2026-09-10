//! End-to-end coverage: a real source-graph overlay on disk, read all the way
//! through `retrieve_for_stage` into a packed `ItemKind::SourceNode` item.
//!
//! Every other source-channel test builds its `ResolvedGraph` by hand and
//! calls `pack`/`rank_source` directly (see `pack_source.rs`,
//! `rank_source.rs`, `rank_source_matching.rs`). None of them exercise
//! `load_resolved_graph`'s own wiring — the overlay address it computes, or
//! `retrieve_for_stage`'s call into it. If `load_resolved_graph` degraded to
//! `None` on every call, or resolved the wrong `(plan, stage)` address, every
//! one of those tests would still pass unchanged. This test writes a real
//! overlay file at the exact address `OverlayScope::Local` resolves to and
//! drives the whole pipeline against it, so it fails on either mistake, and
//! it fails too if `pack` ever stops emitting source items.

use super::retrieve::project_with_knowledge;
use crate::context::graph_store::{FileEntry, GraphLayer, GraphStore};
use crate::context::local_overlay::local_overlay_key;
use crate::context::refresh::{reconcile_source_graph, SourceGraphScope};
use crate::context::retrieve::{retrieve_for_stage, StageQuery};
use crate::context::schema::*;
use crate::context::store::ContextStore;
use crate::fs::work_dir::WorkDir;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A symbol distinctive enough that it cannot collide with anything in
/// `project_with_knowledge`'s prose, so a hit in the pack can only have come
/// from the source graph this test writes.
const DISTINCTIVE_SYMBOL: &str = "ZorbleFrobnicator";

/// One symbol node with a real, non-default span and signature.
fn distinctive_node() -> SourceNode {
    SourceNode {
        id: "src/zorble.rs#function:ZorbleFrobnicator".to_string(),
        kind: SourceNodeKind::Function,
        path: PathBuf::from("src/zorble.rs"),
        scope: vec![DISTINCTIVE_SYMBOL.to_string()],
        span: Span {
            start_byte: 40,
            end_byte: 96,
            line_start: 12,
            line_end: 14,
        },
        signature: "pub fn zorble_frobnicator() -> Widget".to_string(),
        body_hash: "sha256:zorble".to_string(),
        language: NodeLanguage::Rust,
        parser_version: "test+v1".to_string(),
        coverage: FileCoverage::Full,
    }
}

/// Write `node` as a real overlay layer at the address `OverlayScope::Local`
/// resolves for `root`, using the same API the production writer (`loom
/// map`, `commands/map.rs`) uses to write it.
fn write_local_overlay(root: &Path, node: &SourceNode) {
    let work_dir = WorkDir::new(root).unwrap();
    let project_root = work_dir.project_root().unwrap();
    let store = ContextStore::open(&work_dir).unwrap();
    let graph_store = GraphStore::new(store.root(), work_dir.root());
    let (plan, stage) = local_overlay_key(project_root);

    let mut files = BTreeMap::new();
    files.insert(
        node.path.to_string_lossy().into_owned(),
        FileEntry {
            content_hash: "sha256:file".to_string(),
            nodes: vec![node.clone()],
            edges: Vec::new(),
            coverage: FileCoverage::Full,
        },
    );
    let layer = GraphLayer {
        revision: "test-revision".to_string(),
        generation: String::new(),
        built_at: None,
        files,
        blob_index: BTreeMap::new(),
    };
    graph_store.save_overlay(&plan, &stage, &layer).unwrap();
}

#[test]
fn retrieve_for_stage_packs_a_source_node_from_a_real_overlay_on_disk() {
    let temp = project_with_knowledge();
    let root = temp.path();
    let node = distinctive_node();
    write_local_overlay(root, &node);

    let query = StageQuery::new(root, format!("Where is {DISTINCTIVE_SYMBOL} defined?"));
    let pack = retrieve_for_stage(&query, 500).unwrap();

    let item = pack
        .items
        .iter()
        .find(|item| item.id.as_str() == node.id)
        .unwrap_or_else(|| {
            panic!(
                "expected a source item for {}, got {:?}",
                node.id,
                pack.items
                    .iter()
                    .map(|item| item.id.as_str())
                    .collect::<Vec<_>>()
            )
        });

    assert_eq!(item.kind, ItemKind::SourceNode);
    assert_eq!(item.pointer.path, node.path);
    assert_eq!(item.pointer.line_start, Some(12));
    assert_eq!(item.pointer.line_end, Some(14));
}

/// A checkout with a `.loom/work/` directory and NO `doc/loom/knowledge/`: a
/// repository `loom map` has run in, but which has no curated knowledge tree.
fn project_without_knowledge() -> TempDir {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir_all(temp.path().join(".loom").join("work")).unwrap();
    temp
}

/// The source channel needs no chunk catalog, so an absent knowledge tree must
/// degrade retrieval to source-only rather than failing it. `resolve_roots`
/// used to `bail!` here, which meant a repository with a perfectly good source
/// graph got nothing at all — including from the prompt hook, which is the
/// surface most likely to be pointed at a project that has never been
/// `loom init`ed.
#[test]
fn retrieve_for_stage_packs_source_nodes_with_no_knowledge_tree_at_all() {
    let temp = project_without_knowledge();
    let root = temp.path();
    let node = distinctive_node();
    write_local_overlay(root, &node);

    let query = StageQuery::new(root, format!("Where is {DISTINCTIVE_SYMBOL} defined?"));
    let pack = retrieve_for_stage(&query, 500).expect("a missing knowledge tree is not an error");

    let ids: Vec<&str> = pack.items.iter().map(|item| item.id.as_str()).collect();
    assert!(
        ids.contains(&node.id.as_str()),
        "the source node must still be packed: {ids:?}"
    );
    assert!(
        pack.items
            .iter()
            .all(|item| item.kind == ItemKind::SourceNode),
        "with no catalog there is nothing but source nodes to pack"
    );

    // Honest about the layer that does not exist: never built, never "current".
    assert!(
        pack.structural_freshness.stale,
        "a structural layer over no knowledge tree must not read as current"
    );
    assert!(pack.structural_freshness.revision.is_empty());
    let detail = pack.structural_freshness.detail.as_deref().unwrap_or("");
    assert!(
        detail.contains("no knowledge directory"),
        "the pack must say WHY the structural layer is empty, got: {detail}"
    );
}

// A.11: `ContextPack::degraded` fires only when `state.json` names a
// semantic revision AND the RESOLVED graph — base plus this query's overlay —
// has nothing to answer with: no base was found for that revision, AND no
// overlay covered for it either (`retrieve::graph::degraded_reason`). A
// missing base alone is NOT degraded — older caches may still contain only a
// `_local` overlay, and that overlay can fully cover the requested checkout;
// "no base for the current revision, served from the overlay" remains a
// healthy readable state. Getting this wrong is not just a display bug: `degraded` is a
// live input to `reconcile_graph::spawn_if_needed` (`stale OR degraded`), so
// misreporting it means every prompt against a dirty tree trips a background
// full-repository rebuild.
//
// These write `state.json` directly with `ContextStore::update_state` rather
// than running a real `reconcile_source_graph`, so each test isolates one
// branch instead of depending on git/dirty-tree behavior neither
// `retrieve_for_stage` nor this fixture own.

/// THE regression case this predicate exists to get right: no base was ever
/// published for the recorded revision, but a real overlay covers it — the
/// ordinary, healthy state of a dirty working tree, not a degradation. Before
/// the two-part fix, this read as `degraded: Some(..)` for every dirty-tree
/// checkout, forever.
#[test]
fn retrieve_for_stage_is_not_degraded_when_an_overlay_covers_a_missing_base() {
    let temp = project_with_knowledge();
    let root = temp.path();
    let node = distinctive_node();
    write_local_overlay(root, &node);

    let work_dir = WorkDir::new(root).unwrap();
    let store = ContextStore::open(&work_dir).unwrap();
    store
        .update_state(|state| {
            state.semantic = Freshness {
                revision: "deadbeef00".to_string(),
                ..Freshness::default()
            };
        })
        .unwrap();

    let query = StageQuery::new(root, format!("Where is {DISTINCTIVE_SYMBOL} defined?"));
    let pack = retrieve_for_stage(&query, 500).unwrap();

    assert_eq!(
        pack.degraded, None,
        "an overlay that covers a missing base must never read as degraded"
    );
    assert!(
        pack.items.iter().any(|item| item.id.as_str() == node.id),
        "the overlay's own content must still be packed: {:?}",
        pack.items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>()
    );
}

/// A `state.json` revision with no matching `graph/base/<rev>.json` on disk
/// AND no overlay to cover for it — the resolved graph is genuinely empty, the
/// one case `degraded_reason` exists to surface.
#[test]
fn retrieve_for_stage_reports_degraded_when_nothing_covers_the_missing_base() {
    let temp = project_with_knowledge();
    let root = temp.path();
    let work_dir = WorkDir::new(root).unwrap();
    let store = ContextStore::open(&work_dir).unwrap();
    store
        .update_state(|state| {
            state.semantic = Freshness {
                revision: "deadbeef00".to_string(),
                ..Freshness::default()
            };
        })
        .unwrap();

    let query = StageQuery::new(root, "anything at all");
    let pack = retrieve_for_stage(&query, 500).unwrap();

    let degraded = pack
        .degraded
        .as_deref()
        .expect("a semantic revision with no base file must be reported degraded");
    assert!(
        degraded.contains("deadbeef"),
        "the message must name the missing revision, got: {degraded}"
    );
}

/// A published base for the exact revision `state.json` names — the honest,
/// healthy case `Semantic: current` describes. Deliberately publishes a base
/// with an EMPTY `files` map: a project with no matching source files still
/// has a real, current base, so this must stay `None` on `files` content
/// alone — `degraded_reason` has to check `base_revision`, not just whether
/// the resolved graph happens to have files.
#[test]
fn retrieve_for_stage_is_not_degraded_when_the_semantic_base_exists() {
    let temp = project_with_knowledge();
    let root = temp.path();
    let work_dir = WorkDir::new(root).unwrap();
    let store = ContextStore::open(&work_dir).unwrap();
    let graph_store = GraphStore::new(store.root(), work_dir.root());

    let revision = "cafef00dcafef00d";
    let layer = GraphLayer {
        revision: revision.to_string(),
        generation: String::new(),
        built_at: None,
        files: BTreeMap::new(),
        blob_index: BTreeMap::new(),
    };
    graph_store.publish_base(revision, &layer).unwrap();
    store
        .update_state(|state| {
            state.semantic = Freshness {
                revision: revision.to_string(),
                ..Freshness::default()
            };
        })
        .unwrap();

    let query = StageQuery::new(root, "anything at all");
    let pack = retrieve_for_stage(&query, 500).unwrap();

    assert_eq!(
        pack.degraded, None,
        "a published base for the recorded revision must never read as degraded"
    );
}

/// No `state.json` write at all: the semantic layer stays at its empty
/// default revision, which reads as "never built", not "degraded".
#[test]
fn retrieve_for_stage_is_not_degraded_when_the_semantic_layer_was_never_built() {
    let temp = project_with_knowledge();
    let query = StageQuery::new(temp.path(), "anything at all");

    let pack = retrieve_for_stage(&query, 500).unwrap();

    assert_eq!(
        pack.degraded, None,
        "an empty semantic revision means never built, not degraded"
    );
}

#[test]
fn retrieval_reports_the_working_tree_stale_when_the_overlay_generation_moved() {
    let temp = project_with_knowledge();
    let root = temp.path();
    let git = |args: &[&str]| {
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
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.email", "t@t.com"]);
    git(&["config", "user.name", "t"]);
    std::fs::write(root.join("src.rs"), "fn before_edit() {}\n").unwrap();
    git(&["add", "doc", "src.rs"]);
    git(&["commit", "-m", "seed"]);

    let work_dir = WorkDir::new(root).unwrap();
    let store = ContextStore::open(&work_dir).unwrap();
    let graph_store = GraphStore::new(store.root(), work_dir.root());
    let (plan, stage) = local_overlay_key(root);
    reconcile_source_graph(
        &store,
        &graph_store,
        root,
        SourceGraphScope::Overlay { plan, stage },
    )
    .unwrap();
    std::fs::write(root.join("src.rs"), "fn after_edit() {}\n").unwrap();

    let pack = retrieve_for_stage(&StageQuery::new(root, "after edit"), 500).unwrap();

    assert!(pack.semantic_freshness.stale);
    assert_eq!(
        pack.semantic_freshness.detail.as_deref(),
        Some("working tree changed since the source graph overlay was built")
    );
}
