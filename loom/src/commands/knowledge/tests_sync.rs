//! Tests for `commands/knowledge/sync.rs`.
//!
//! `sync`'s CWD-dependent tests (it resolves `.` via `resolve()`/`WorkDir::new`,
//! same as `update`/`replace_section` in `tests.rs`) reuse that module's
//! `setup_test_env`/`make_legacy` helpers rather than duplicating them —
//! `tests_legacy.rs` already sets the precedent for reaching across sibling
//! test modules with `super::tests::...`; this module is one level deeper
//! (`sync::tests`, not a direct child of `commands::knowledge`), hence
//! `super::super::tests::...`.

use super::super::tests::{make_legacy, setup_test_env};
use super::*;
use serial_test::serial;
use std::fs;
use std::path::PathBuf;

/// The tier-2 topic [`sync_refreshes_the_index_on_an_already_hierarchical_directory_and_still_reports_upgraded_false`]
/// grows, and the row shape `INDEX.md` records it with. The line counts are
/// exact: the seeded body is five `str::lines()` lines (heading, blank,
/// blurb, blank, one line of prose) and the growth appends five more, so the
/// row's trailing column must move 5 -> 10. Getting that arithmetic wrong
/// would make the test pass against an index that never refreshed.
const GROWN_TOPIC: &str = "architecture/grows.md";

fn topic_row(lines: usize) -> String {
    format!("| [grows]({GROWN_TOPIC}) | A topic that will grow. | {lines} |")
}

/// Build an already-hierarchical knowledge directory holding a five-line
/// tier-2 topic, with an `INDEX.md` that has already recorded it at that size.
///
/// The baseline index write is not incidental: `initialize()` writes an index
/// BEFORE the topic file exists, so without it the "stale" index would not
/// mention the topic at all and the test could not distinguish a refresh from
/// a first-time write.
///
/// Returns the knowledge root and the topic's absolute path. The caller is
/// already inside `test_dir` as its working directory.
fn hierarchical_dir_with_indexed_topic(test_dir: &std::path::Path) -> (PathBuf, PathBuf) {
    KnowledgeDir::new(".")
        .initialize()
        .expect("Failed to initialize knowledge");

    let knowledge_root = test_dir.join("doc/loom/knowledge");
    let topic_path = knowledge_root.join(GROWN_TOPIC);
    fs::create_dir_all(topic_path.parent().unwrap()).unwrap();
    fs::write(
        &topic_path,
        "# Grows\n\n> A topic that will grow.\n\nOne line.\n",
    )
    .unwrap();

    KnowledgeDir::from_root(&knowledge_root)
        .write_index()
        .expect("baseline write_index failed");
    let stale_index = fs::read_to_string(knowledge_root.join(INDEX_FILENAME)).unwrap();
    assert!(
        stale_index.contains(&topic_row(5)),
        "baseline index must record the topic's original 5-line count, got:\n{stale_index}"
    );

    (knowledge_root, topic_path)
}

/// An already-hierarchical directory whose tier-2 file changed size on disk —
/// exactly what a direct Edit/Write session (CLAUDE.md Rule 12) leaves behind
/// — gets a refreshed `INDEX.md` after `sync`, and `upgrade_flat_layout` (the
/// boolean `sync` reports as `upgraded`) still reads `false`. This is the gap
/// `sync` used to leave open entirely: before this fix, `INDEX.md` never
/// moved past whatever the last upgrade or `update`/`replace-section` call
/// had left it at. See [`hierarchical_dir_with_indexed_topic`] and
/// [`GROWN_TOPIC`] for the fixture this asserts against.
#[test]
#[serial]
fn sync_refreshes_the_index_on_an_already_hierarchical_directory_and_still_reports_upgraded_false()
{
    let (_temp_dir, test_dir) = setup_test_env();
    let original_dir = std::env::current_dir().expect("Failed to get current dir");
    std::env::set_current_dir(&test_dir).expect("Failed to change dir");

    let (knowledge_root, topic_path) = hierarchical_dir_with_indexed_topic(&test_dir);

    // `upgrade_flat_layout` on an already-hierarchical directory: unchanged
    // behavior this fix must not disturb.
    let upgraded = upgrade_flat_layout(&knowledge_root).expect("upgrade_flat_layout failed");
    assert!(
        !upgraded,
        "an already-hierarchical directory must not report an upgrade"
    );

    // Grow the topic file directly on disk, the way an interactive Edit/Write
    // session leaves it — no `update`/`replace-section` call, so nothing else
    // in the pipeline would refresh the index on its own.
    let mut grown = fs::read_to_string(&topic_path).unwrap();
    grown.push_str("Two.\nThree.\nFour.\nFive.\nSix.\n");
    fs::write(&topic_path, &grown).unwrap();

    let result = sync(true, false);
    std::env::set_current_dir(original_dir).expect("Failed to restore dir");
    result.expect("sync must succeed on an already-hierarchical directory");

    let refreshed_index = fs::read_to_string(knowledge_root.join(INDEX_FILENAME)).unwrap();
    assert!(
        refreshed_index.contains(&topic_row(10)),
        "sync must refresh INDEX.md's line count for a resized tier-2 file, got:\n{refreshed_index}"
    );
    // The stale row's ABSENCE matters as much as the fresh row's presence: an
    // index that merely appended would satisfy the assertion above.
    assert!(
        !refreshed_index.contains(&topic_row(5)),
        "the stale 5-line row must not still be present after a refresh, got:\n{refreshed_index}"
    );
}

/// A flat (pre-hierarchy) directory still upgrades to hierarchical and
/// reports `true` — unchanged by this fix, since `sync`'s new
/// refresh-on-already-hierarchical step only runs on the `false` branch.
/// `upgrade_flat_layout` takes an explicit path, so no CWD manipulation is
/// needed here — only `sync` itself resolves `.`.
#[test]
fn sync_flat_directory_still_upgrades_and_reports_upgraded_true() {
    let temp = tempfile::TempDir::new().unwrap();
    let project_root = temp.path();
    KnowledgeDir::new(project_root)
        .initialize()
        .expect("Failed to initialize knowledge");
    make_legacy(project_root);

    let knowledge_root = project_root.join("doc/loom/knowledge");
    assert_eq!(
        KnowledgeDir::from_root(&knowledge_root).layout(),
        KnowledgeLayout::Legacy,
        "make_legacy must have removed INDEX.md"
    );

    let upgraded = upgrade_flat_layout(&knowledge_root).expect("upgrade_flat_layout failed");

    assert!(upgraded, "a flat directory must report the upgrade as true");
    assert_eq!(
        KnowledgeDir::from_root(&knowledge_root).layout(),
        KnowledgeLayout::Hierarchical,
        "the directory must now read as hierarchical"
    );
}

/// A pre-hierarchy directory's `make_legacy` fixture (INDEX.md removed after
/// `initialize()`) also still upgrades through the full `sync()` entry point,
/// not just through `upgrade_flat_layout` called directly — this is the path
/// `loom knowledge sync` actually runs.
#[test]
#[serial]
fn sync_upgrades_a_legacy_directory_end_to_end() {
    let (_temp_dir, test_dir) = setup_test_env();
    let original_dir = std::env::current_dir().expect("Failed to get current dir");
    std::env::set_current_dir(&test_dir).expect("Failed to change dir");

    KnowledgeDir::new(".")
        .initialize()
        .expect("Failed to initialize knowledge");
    make_legacy(&test_dir);

    let knowledge_root = test_dir.join("doc/loom/knowledge");
    assert_eq!(
        KnowledgeDir::from_root(&knowledge_root).layout(),
        KnowledgeLayout::Legacy
    );

    let result = sync(true, false);
    std::env::set_current_dir(original_dir).expect("Failed to restore dir");
    result.expect("sync must upgrade a legacy directory");

    assert_eq!(
        KnowledgeDir::from_root(&knowledge_root).layout(),
        KnowledgeLayout::Hierarchical,
        "sync must have created INDEX.md"
    );
}

/// An `INDEX.md` write failure on an already-hierarchical directory must NOT
/// fail `sync` — the catalog rebuild is the substantive work, and the index
/// is a cosmetic, best-effort refresh (see `sync`'s and
/// `refresh_index_best_effort`'s doc comments).
///
/// Unix-only: the failure is induced by removing write permission on the
/// knowledge root, which has no direct Windows equivalent worth building for
/// a single edge-case regression test.
#[cfg(unix)]
#[test]
#[serial]
fn sync_does_not_fail_when_the_index_write_fails() {
    use std::os::unix::fs::PermissionsExt;

    let (_temp_dir, test_dir) = setup_test_env();
    let original_dir = std::env::current_dir().expect("Failed to get current dir");
    std::env::set_current_dir(&test_dir).expect("Failed to change dir");

    KnowledgeDir::new(".")
        .initialize()
        .expect("Failed to initialize knowledge");
    let knowledge_root = test_dir.join("doc/loom/knowledge");
    let changed_topic = knowledge_root.join("architecture/index-write-must-run.md");
    fs::create_dir_all(changed_topic.parent().unwrap()).unwrap();
    fs::write(
        changed_topic,
        "# Changed\n\n> Makes the generated index differ.\n",
    )
    .unwrap();

    // Read + execute only, no write: `write_index`'s crash-atomic temp-file
    // create needs write permission on the PARENT directory to succeed, so
    // this fails only the index write. Reads (the catalog rebuild that
    // follows) stay unaffected — read and execute bits are untouched.
    let mut perms = fs::metadata(&knowledge_root).unwrap().permissions();
    perms.set_mode(0o555);
    fs::set_permissions(&knowledge_root, perms).expect("failed to lock down permissions");

    let result = sync(true, false);

    // Restore write access before anything else can fail: an assertion
    // panicking below must not leave a read-only directory behind for
    // `TempDir`'s `Drop` to choke on.
    let mut perms = fs::metadata(&knowledge_root).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&knowledge_root, perms).expect("failed to restore permissions");
    std::env::set_current_dir(original_dir).expect("Failed to restore dir");

    result.expect(
        "an INDEX.md write failure on an already-hierarchical directory must not fail sync",
    );
}

#[test]
fn semantic_json_includes_snapshot_action_counters_and_elapsed_time() {
    let semantic = SemanticOutcome::skipped(Default::default(), "--structural-only");

    let json = semantic_json(&semantic);

    assert_eq!(json["action"], "skipped");
    assert_eq!(json["parsed"], 0);
    assert_eq!(json["reused"], 0);
    assert_eq!(json["deleted"], 0);
    assert_eq!(json["elapsed_ms"], 0);
}

/// A context-catalog write failure (BUG 4, `loom-bugs.txt`) must still fail
/// `sync` — the catalog rebuild is real work — but the error it returns must
/// say the knowledge index step already completed, not read as if `sync` did
/// nothing at all. `INDEX.md` itself must still be on disk, unaffected by a
/// failure two steps later in an unrelated cache directory.
///
/// The catalog write is forced to fail by occupying its own persisted path
/// (`.loom/cache/context-v1/catalog.json`) with a directory: `save_catalog`
/// (`context/store.rs`) writes its temp file into the still-writable parent
/// and then renames it onto that path, so the write itself succeeds and only
/// the final rename fails (`EISDIR`) — the same "catalog write failed" shape
/// the bug report hit, without needing a read-only filesystem.
#[test]
#[serial]
fn sync_reports_the_index_step_completed_when_the_context_catalog_write_fails() {
    let (_temp_dir, test_dir) = setup_test_env();
    let original_dir = std::env::current_dir().expect("Failed to get current dir");
    std::env::set_current_dir(&test_dir).expect("Failed to change dir");

    KnowledgeDir::new(".")
        .initialize()
        .expect("Failed to initialize knowledge");

    let catalog_path = test_dir
        .join(crate::context::store::CACHE_RELATIVE_DIR)
        .join(crate::context::store::CATALOG_FILE);
    fs::create_dir_all(&catalog_path).expect("failed to occupy catalog.json with a directory");

    let result = sync(true, false);
    std::env::set_current_dir(original_dir).expect("Failed to restore dir");

    let error = result.expect_err("a context catalog write failure must still fail sync");
    let rendered = format!("{error:#}");
    assert!(
        rendered.contains("knowledge index step completed"),
        "error must say the index step already completed, got:\n{rendered}"
    );
    assert!(
        rendered.contains("context catalog"),
        "error must name the context catalog as the failing half, got:\n{rendered}"
    );

    let index_path = test_dir.join("doc/loom/knowledge").join(INDEX_FILENAME);
    assert!(
        index_path.exists(),
        "INDEX.md must still be on disk after the later catalog step fails"
    );
}
