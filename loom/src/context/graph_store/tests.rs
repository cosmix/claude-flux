use super::*;
use std::time::{Duration, SystemTime};
use tempfile::TempDir;

/// Write a base layer for `revision` directly (bypassing `publish_base`, so
/// none of these fixture writes trigger a prune of their own), then back-date
/// its mtime by `seconds_ago` so [`GraphStore::prune_base_graphs`] has a
/// stable, distinct "most recently modified" ordering to sort by. Returns the
/// layer's path.
fn write_dated_base(store: &GraphStore, revision: &str, seconds_ago: u64) -> PathBuf {
    let path = store.base_path(revision);
    let layer = GraphLayer {
        revision: revision.to_string(),
        generation: String::new(),
        blob_index: BTreeMap::new(),
        ..GraphLayer::default()
    };
    write_layer(&path, &layer).unwrap();

    let modified = SystemTime::now() - Duration::from_secs(seconds_ago);
    std::fs::File::open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    path
}

/// Sorted file names directly inside `store`'s base directory, for
/// before/after comparisons that must prove NOTHING changed.
fn base_dir_listing(store: &GraphStore) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(store.base_dir())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn entry(hash: &str) -> FileEntry {
    FileEntry {
        content_hash: hash.to_string(),
        coverage: FileCoverage::Full,
        ..FileEntry::default()
    }
}

fn store(temp: &TempDir) -> GraphStore {
    GraphStore::new(&temp.path().join("cache"), &temp.path().join("work"))
}

#[test]
fn an_overlay_entry_shadows_the_base_entry_for_the_same_path() {
    let temp = TempDir::new().unwrap();
    let store = store(&temp);

    let mut base = GraphLayer {
        revision: "rev1".to_string(),
        generation: String::new(),
        blob_index: BTreeMap::new(),
        ..GraphLayer::default()
    };
    base.files.insert("src/a.rs".to_string(), entry("base-a"));
    base.files.insert("src/b.rs".to_string(), entry("base-b"));
    assert!(store.publish_base("rev1", &base).unwrap());

    let mut overlay = GraphLayer::default();
    overlay
        .files
        .insert("src/a.rs".to_string(), entry("overlay-a"));
    store.save_overlay("plan", "stage", &overlay).unwrap();

    let resolved = store.resolved("rev1", Some(("plan", "stage"))).unwrap();
    assert_eq!(resolved.files["src/a.rs"].content_hash, "overlay-a");
    assert_eq!(resolved.files["src/b.rs"].content_hash, "base-b");
    assert_eq!(resolved.overlaid.len(), 1);
    assert!(resolved.overlaid.contains("src/a.rs"));
}

#[test]
fn a_tombstone_in_the_overlay_hides_the_base_entry() {
    let temp = TempDir::new().unwrap();
    let store = store(&temp);
    let mut base = GraphLayer {
        revision: "rev1".to_string(),
        ..Default::default()
    };
    base.files.insert("src/lib.rs".to_string(), entry("base"));
    store.publish_base("rev1", &base).unwrap();

    let mut overlay = GraphLayer {
        revision: "rev1".to_string(),
        ..Default::default()
    };
    overlay
        .files
        .insert("src/lib.rs".to_string(), FileEntry::tombstone());
    store.save_overlay("plan", "stage", &overlay).unwrap();

    let resolved = store.resolved("rev1", Some(("plan", "stage"))).unwrap();
    assert!(!resolved.files.contains_key("src/lib.rs"));
    assert!(!resolved.overlaid.contains("src/lib.rs"));
}

#[test]
fn a_tombstone_for_a_path_the_base_lacks_is_dropped_at_persist() {
    let temp = TempDir::new().unwrap();
    let store = store(&temp);
    store
        .publish_base(
            "rev1",
            &GraphLayer {
                revision: "rev1".to_string(),
                ..Default::default()
            },
        )
        .unwrap();
    let mut overlay = GraphLayer {
        revision: "rev1".to_string(),
        ..Default::default()
    };
    overlay
        .files
        .insert("never-existed.rs".to_string(), FileEntry::tombstone());

    store.save_overlay("plan", "stage", &overlay).unwrap();

    let persisted = store.load_overlay("plan", "stage").unwrap().unwrap();
    assert!(!persisted.files.contains_key("never-existed.rs"));
}

#[test]
fn reading_without_a_stage_sees_only_the_base() {
    let temp = TempDir::new().unwrap();
    let store = store(&temp);

    let mut base = GraphLayer::default();
    base.files.insert("src/a.rs".to_string(), entry("base-a"));
    store.publish_base("rev1", &base).unwrap();

    let mut overlay = GraphLayer::default();
    overlay
        .files
        .insert("src/a.rs".to_string(), entry("overlay-a"));
    store.save_overlay("plan", "stage", &overlay).unwrap();

    let resolved = store.resolved("rev1", None).unwrap();
    assert_eq!(resolved.files["src/a.rs"].content_hash, "base-a");
    assert!(resolved.overlaid.is_empty());
}

#[test]
fn a_published_base_layer_is_immutable() {
    let temp = TempDir::new().unwrap();
    let store = store(&temp);

    let mut first = GraphLayer::default();
    first.files.insert("src/a.rs".to_string(), entry("first"));
    assert!(store.publish_base("rev1", &first).unwrap());

    let mut second = GraphLayer::default();
    second.files.insert("src/a.rs".to_string(), entry("second"));
    assert!(
        !store.publish_base("rev1", &second).unwrap(),
        "re-publishing the same revision must be refused"
    );

    let loaded = store.load_base("rev1").unwrap().unwrap();
    assert_eq!(loaded.files["src/a.rs"].content_hash, "first");
}

#[test]
fn two_stages_overlays_never_see_each_other() {
    let temp = TempDir::new().unwrap();
    let store = store(&temp);

    let mut one = GraphLayer::default();
    one.files.insert("src/a.rs".to_string(), entry("stage-one"));
    store.save_overlay("plan", "one", &one).unwrap();

    let mut two = GraphLayer::default();
    two.files.insert("src/a.rs".to_string(), entry("stage-two"));
    store.save_overlay("plan", "two", &two).unwrap();

    let view_one = store.resolved("rev1", Some(("plan", "one"))).unwrap();
    let view_two = store.resolved("rev1", Some(("plan", "two"))).unwrap();
    assert_eq!(view_one.files["src/a.rs"].content_hash, "stage-one");
    assert_eq!(view_two.files["src/a.rs"].content_hash, "stage-two");
}

#[test]
fn discarding_an_absent_overlay_succeeds() {
    let temp = TempDir::new().unwrap();
    let store = store(&temp);
    store.discard_overlay("plan", "never-existed").unwrap();
}

#[test]
fn discarding_an_overlay_leaves_sibling_records_in_the_shared_directory() {
    let temp = TempDir::new().unwrap();
    let store = store(&temp);

    let overlay = GraphLayer::default();
    store.save_overlay("plan", "stage", &overlay).unwrap();

    let dir = store.overlay_dir("plan", "stage");
    let session_retrieval_dir = dir.join("session-retrieval");
    std::fs::create_dir_all(&session_retrieval_dir).unwrap();
    let dirty_paths = dir.join("dirty-paths.json");
    std::fs::write(&dirty_paths, "{}").unwrap();
    let delivery_record = session_retrieval_dir.join("some-stage.json");
    std::fs::write(&delivery_record, "{}").unwrap();

    store.discard_overlay("plan", "stage").unwrap();

    assert!(
        !store.overlay_path("plan", "stage").exists(),
        "the graph layer file must be removed"
    );
    assert!(
        dirty_paths.exists(),
        "the edit recorder's file must survive discard_overlay"
    );
    assert!(
        delivery_record.exists(),
        "the delivery record must survive discard_overlay"
    );
}

#[test]
fn a_missing_base_resolves_to_an_overlay_only_view() {
    let temp = TempDir::new().unwrap();
    let store = store(&temp);

    let mut overlay = GraphLayer::default();
    overlay.files.insert("src/a.rs".to_string(), entry("only"));
    store.save_overlay("plan", "stage", &overlay).unwrap();

    let resolved = store
        .resolved("never-built", Some(("plan", "stage")))
        .unwrap();
    assert_eq!(resolved.files.len(), 1);
    assert_eq!(resolved.base_revision, "");
}

#[test]
fn prune_base_graphs_keeps_the_new_one_the_protected_one_and_the_keep_most_recent() {
    let temp = TempDir::new().unwrap();
    let store = store(&temp);

    // Six pre-existing base layers, oldest first. `old_a` is also the
    // revision `state.json` currently names (protected), despite being the
    // oldest of the six — proposal A.14's own worked example.
    let old_a = write_dated_base(&store, "old-a", 600);
    let old_b = write_dated_base(&store, "old-b", 500);
    let old_c = write_dated_base(&store, "old-c", 400);
    let recent_a = write_dated_base(&store, "recent-a", 300);
    let recent_b = write_dated_base(&store, "recent-b", 200);
    let recent_c = write_dated_base(&store, "recent-c", 100);
    let brand_new = write_dated_base(&store, "brand-new", 0);

    store.prune_base_graphs(3, &["brand-new", "old-a"]).unwrap();

    assert!(brand_new.exists(), "the just-written revision must survive");
    assert!(
        old_a.exists(),
        "the state.json-referenced revision must survive despite being the oldest file"
    );
    assert!(
        recent_a.exists(),
        "among the 3 most recent unprotected files"
    );
    assert!(
        recent_b.exists(),
        "among the 3 most recent unprotected files"
    );
    assert!(
        recent_c.exists(),
        "among the 3 most recent unprotected files"
    );
    assert!(
        !old_b.exists(),
        "older than the 3 most recent, and unprotected"
    );
    assert!(
        !old_c.exists(),
        "older than the 3 most recent, and unprotected"
    );
}

#[test]
fn republishing_an_existing_revision_prunes_nothing_and_returns_false() {
    let temp = TempDir::new().unwrap();
    let store = store(&temp);

    // Seed enough old layers that a real prune (default keep = 3) would
    // remove some of them, so "the listing did not change" is meaningful.
    for i in 0..5u64 {
        write_dated_base(&store, &format!("filler-{i}"), 1000 - i);
    }

    let layer = GraphLayer {
        revision: "rev1".to_string(),
        generation: String::new(),
        blob_index: BTreeMap::new(),
        ..GraphLayer::default()
    };
    assert!(store.publish_base("rev1", &layer).unwrap());
    let listing_after_first_publish = base_dir_listing(&store);

    assert!(
        !store.publish_base("rev1", &layer).unwrap(),
        "re-publishing the same revision must be refused"
    );

    assert_eq!(
        base_dir_listing(&store),
        listing_after_first_publish,
        "a refused re-publish must not prune anything"
    );
}

#[test]
fn a_protected_revision_survives_even_when_it_is_the_oldest_file() {
    let temp = TempDir::new().unwrap();
    let store = store(&temp);

    let oldest = write_dated_base(&store, "ancient", 1000);
    write_dated_base(&store, "b", 400);
    write_dated_base(&store, "c", 300);
    write_dated_base(&store, "d", 200);
    write_dated_base(&store, "e", 100);

    // keep = 2 would normally drop "ancient" first — it is the oldest of all
    // five — if it were not explicitly protected.
    store.prune_base_graphs(2, &["ancient"]).unwrap();

    assert!(
        oldest.exists(),
        "a protected revision must survive regardless of age"
    );
}

#[test]
fn an_entry_that_cannot_be_removed_does_not_fail_the_prune() {
    let temp = TempDir::new().unwrap();
    let store = store(&temp);

    // `fs::remove_file` unconditionally fails on a directory, portably and
    // regardless of privilege level: unlink is gated by the PARENT
    // directory's write permission, not the target's own mode bits, so
    // chmod-ing the candidate file itself would not simulate an unremovable
    // entry (root and several sandboxes ignore mode bits entirely — see
    // `refresh/tests_source_graph.rs`'s `an_unreadable_file_...` test), and
    // chmod-ing the whole base directory would also block the legitimate
    // writes this test needs to succeed. A directory masquerading as a
    // `<revision>.json` layer gives a deterministic, portable "this
    // candidate cannot be removed" case with no such caveat.
    let stray_dir = store.base_dir().join("stray.json");
    std::fs::create_dir_all(&stray_dir).unwrap();

    let survivor = write_dated_base(&store, "survivor", 0);

    let result = store.prune_base_graphs(0, &[]);

    assert!(
        result.is_ok(),
        "an unremovable candidate must not fail the whole prune"
    );
    assert!(
        stray_dir.exists(),
        "the unremovable entry is still there: remove_file failed on it, as expected"
    );
    assert!(
        !survivor.exists(),
        "a removable candidate must still be pruned despite the sibling failure"
    );
}
