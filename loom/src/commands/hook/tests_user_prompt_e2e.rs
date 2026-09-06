//! END TO END over a real project on disk.
//!
//! `tests_user_prompt.rs` exercises composition on a hand-built pack; these
//! tests drive `retrieve_for_prompt` — target resolution, retrieval,
//! suppression — against a real state-directory tree and a real source-graph
//! overlay, which is the only way to catch the hook silently resolving no
//! target at all. They mutate process environment and are therefore
//! `#[serial]`.
//!
//! Split out of `tests_user_prompt.rs` itself so that file stays under the
//! maintainability line limit; wired back in via `#[path =
//! "tests_user_prompt_e2e.rs"] mod e2e;` at the bottom of that file.

use super::super::retrieve_for_prompt;
use crate::context::graph_store::{FileEntry, GraphLayer, GraphStore};
use crate::context::local_overlay::local_overlay_key;
use crate::context::schema::{
    FileCoverage, ItemKind, NodeLanguage, SourceNode, SourceNodeKind, Span,
};
use crate::context::store::ContextStore;
use crate::fs::work_dir::WorkDir;
use serial_test::serial;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A symbol distinctive enough that a hit on it can only have come from the
/// overlay these tests write.
const DISTINCTIVE_SYMBOL: &str = "ZorbleFrobnicator";

/// A question long enough to clear `MIN_PROMPT_CHARS`, aimed at that symbol.
fn distinctive_prompt() -> String {
    format!("Where is {DISTINCTIVE_SYMBOL} defined and what calls it?")
}

/// A checkout with a `.loom/work/` directory and NO knowledge tree: an
/// ordinary repository that `loom map` has run in but `loom init` never has.
fn mapped_project_without_knowledge() -> TempDir {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir_all(temp.path().join(".loom").join("work")).unwrap();
    write_local_overlay(temp.path());
    temp
}

/// Write one source node into the working-tree overlay, at the address
/// `local_overlay_key` computes — the same one `loom map` writes and the same
/// one a stage-less `StageQuery` reads.
fn write_local_overlay(root: &Path) {
    let node = SourceNode {
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
    };

    let work_dir = WorkDir::new(root).unwrap();
    let store = ContextStore::open(&work_dir).unwrap();
    let graph_store = GraphStore::new(store.root(), work_dir.root());
    let (plan, stage) = local_overlay_key(work_dir.project_root().unwrap());

    let mut files = BTreeMap::new();
    files.insert(
        node.path.to_string_lossy().into_owned(),
        FileEntry {
            content_hash: "sha256:file".to_string(),
            nodes: vec![node],
            edges: Vec::new(),
            coverage: FileCoverage::Full,
        },
    );
    graph_store
        .save_overlay(
            &plan,
            &stage,
            &GraphLayer {
                revision: "test-revision".to_string(),
                built_at: None,
                files,
            },
        )
        .unwrap();
}

/// Point the hook at `root` with no stage naming it — a plain Claude Code
/// session in a mapped repository.
fn enter_checkout(root: &Path) {
    std::env::remove_var("LOOM_STAGE_ID");
    std::env::set_var("LOOM_WORK_DIR", root);
}

fn leave() {
    std::env::remove_var("LOOM_STAGE_ID");
    std::env::remove_var("LOOM_WORK_DIR");
}

#[test]
#[serial]
fn a_session_with_no_stage_at_all_still_gets_a_brief() {
    let temp = mapped_project_without_knowledge();
    enter_checkout(temp.path());

    let emission = retrieve_for_prompt(distinctive_prompt(), None);

    leave();
    let emission = emission.expect("a mapped checkout answers even with no stage and no knowledge");
    assert_eq!(
        emission.target.plan,
        crate::context::local_overlay::LOCAL_PLAN_KEY,
        "a stage-less session is filed under the working-tree overlay"
    );
    // The brief no longer prints a source item's raw `<path>#<kind>:<scope>`
    // id verbatim - it parses the id into a path/name/kind bullet (see
    // `orchestrator::signals::format::brief::render_source_entry`), so the
    // node's presence is checked by its rendered path and name instead.
    assert!(
        emission.payload.contains("`src/zorble.rs`")
            && emission.payload.contains("`ZorbleFrobnicator`"),
        "the source node must reach the payload: {}",
        emission.payload
    );
    assert!(
        emission
            .handed_over
            .items
            .iter()
            .all(|item| item.kind == ItemKind::SourceNode),
        "no knowledge tree means a source-only brief"
    );
    assert!(
        emission.payload.contains("loom knowledge context --query")
            && !emission.payload.contains("--stage"),
        "a stage-less session must not point the reader at a stage that does not exist: {}",
        emission.payload
    );
}

#[test]
#[serial]
fn a_second_local_prompt_is_suppressed_once_the_first_is_recorded() {
    let temp = mapped_project_without_knowledge();
    enter_checkout(temp.path());

    let first = retrieve_for_prompt(distinctive_prompt(), Some("session-a"));
    if let Some(emission) = &first {
        emission
            .target
            .record(&emission.recipient, &emission.handed_over);
    }
    let second = retrieve_for_prompt(distinctive_prompt(), Some("session-a"));

    leave();
    assert!(first.is_some(), "the first prompt is answered");
    assert!(
        second.is_none(),
        "the same units in the same epoch must not be handed over twice"
    );
}

#[test]
#[serial]
fn a_different_session_is_not_suppressed_by_the_first_sessions_delivery() {
    let temp = mapped_project_without_knowledge();
    enter_checkout(temp.path());

    let first = retrieve_for_prompt(distinctive_prompt(), Some("session-a"));
    if let Some(emission) = &first {
        emission
            .target
            .record(&emission.recipient, &emission.handed_over);
    }
    let second = retrieve_for_prompt(distinctive_prompt(), Some("session-b"));

    leave();
    assert!(first.is_some(), "the first session is answered");
    assert!(
        second.is_some(),
        "a different session's empty context window must not inherit session-a's suppression"
    );
}

#[test]
#[serial]
fn a_hook_payload_with_no_session_id_still_answers_and_suppresses_a_repeat() {
    let temp = mapped_project_without_knowledge();
    enter_checkout(temp.path());

    let first = retrieve_for_prompt(distinctive_prompt(), None);
    if let Some(emission) = &first {
        emission
            .target
            .record(&emission.recipient, &emission.handed_over);
    }
    let second = retrieve_for_prompt(distinctive_prompt(), None);

    leave();
    assert!(first.is_some(), "a session-less prompt is still answered");
    assert!(
        second.is_none(),
        "today's shared 'nosession' behaviour: a repeat with no session id is still suppressed"
    );
}

#[test]
#[serial]
fn a_stage_session_is_still_keyed_to_its_stage() {
    let temp = mapped_project_without_knowledge();
    let work_dir = temp.path().join(".loom").join("work");
    let stage = crate::models::stage::Stage {
        id: "prompt-hook-stage".to_string(),
        name: "Prompt Hook Stage".to_string(),
        plan_id: Some("test-plan".to_string()),
        ..crate::models::stage::Stage::default()
    };
    crate::verify::transitions::create_stage(&stage, &work_dir).unwrap();

    std::env::set_var("LOOM_WORK_DIR", &work_dir);
    std::env::set_var("LOOM_STAGE_ID", &stage.id);
    let emission = retrieve_for_prompt(distinctive_prompt(), None);
    leave();

    let emission = emission.expect("a stage in a mapped checkout is answered too");
    assert_eq!(emission.target.stage_id, stage.id);
    assert_eq!(
        emission.target.plan, "test-plan",
        "the stage record's plan is the delivery key, not the local overlay's"
    );
    assert!(
        emission
            .payload
            .contains(&format!("loom knowledge context --stage {}", stage.id)),
        "the brief points back at the stage that asked: {}",
        emission.payload
    );
}
