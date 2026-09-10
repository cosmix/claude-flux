//! End-to-end tests for [`super::reconcile_graph`] and
//! [`super::spawn_if_needed`], against real state-directory and git fixtures.

use super::*;
use crate::context::schema::{Channel, Freshness, OmissionSummary};
use crate::models::stage::Stage;
use serial_test::serial;
use std::sync::atomic::Ordering;
use tempfile::TempDir;

use super::lock::read_lock;

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

fn git_ok(root: &Path, args: &[&str]) {
    let out = isolated_git(root, args);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A temp git repo with a `.loom/work/` directory and one committed file,
/// ready for a checkout-scope reconcile.
fn init_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    git_ok(root, &["init", "-b", "main"]);
    git_ok(root, &["config", "user.email", "t@t.com"]);
    git_ok(root, &["config", "user.name", "t"]);

    std::fs::write(root.join("src.rs"), "fn main() {}\n").unwrap();
    git_ok(root, &["add", "src.rs"]);
    git_ok(root, &["commit", "-m", "seed"]);

    std::fs::create_dir_all(root.join(".loom").join("work")).unwrap();
    temp
}

fn head_sha(root: &Path) -> String {
    let out = isolated_git(root, &["rev-parse", "HEAD"]);
    assert!(out.status.success(), "rev-parse HEAD failed");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Point this process at `root` with no stage naming it.
fn enter_checkout(root: &Path) {
    std::env::remove_var("LOOM_STAGE_ID");
    std::env::set_var("LOOM_WORK_DIR", root.join(".loom").join("work"));
}

fn leave() {
    std::env::remove_var("LOOM_STAGE_ID");
    std::env::remove_var("LOOM_WORK_DIR");
}

#[test]
#[serial]
fn reconcile_graph_moves_a_stale_semantic_revision_to_head() {
    let temp = init_repo();
    let root = temp.path();

    let work_dir = WorkDir::new(root).unwrap();
    let store = ContextStore::open(&work_dir).unwrap();
    store
        .update_state(|state| {
            state.semantic = Freshness {
                revision: "stale-revision-not-head".to_string(),
                ..Freshness::default()
            };
        })
        .unwrap();

    enter_checkout(root);
    let result = reconcile_graph();
    leave();

    assert!(result.is_ok(), "reconcile_graph must always return Ok(())");
    let state = store.load_state().unwrap();
    assert_eq!(state.semantic.revision, head_sha(root));
    assert!(
        !state.semantic.stale,
        "a freshly published base must not read as stale"
    );
}

#[test]
#[serial]
fn reconcile_graph_leaves_a_finished_marker_after_a_successful_run() {
    let temp = init_repo();
    let root = temp.path();
    let work_dir = WorkDir::new(root).unwrap();
    let store = ContextStore::open(&work_dir).unwrap();
    let lock_path = reconcile_lock_path(&store);
    // A pre-existing in-progress claim, as `spawn_if_needed` would have left
    // before spawning this same process.
    assert!(claim_lock(
        &lock_path,
        unix_now(),
        std::process::id(),
        false
    ));

    enter_checkout(root);
    reconcile_graph().unwrap();
    leave();

    let (_, pid) = read_lock(&lock_path)
        .expect("reconcile_graph must leave a marker behind, never unlink the lock");
    assert_eq!(pid, 0, "a completed run's marker must carry pid 0");
}

#[test]
#[serial]
fn reconcile_graph_with_no_resolvable_work_dir_creates_nothing() {
    // A plain directory: no state directory, no `.git` anywhere above it, and
    // `LOOM_WORK_DIR` names a directory that does not exist either —
    // `WorkDir::new`'s upward search finds nothing and falls back to a path
    // that is not on disk. `HookTarget::from_environment`'s existence
    // check must catch this and yield `None` before `try_reconcile` ever
    // opens a `ContextStore` or claims the debounce lock — this is the
    // guard that keeps a stale `LOOM_WORK_DIR` pin (naming a since-deleted
    // state directory) from having a hook materialize a `.loom/work/` or
    // `.loom/cache` cache in a checkout that was never `loom init`ed.
    let temp = TempDir::new().unwrap();
    let work_dir = WorkDir::new(temp.path()).unwrap();
    let store = ContextStore::open(&work_dir).unwrap();
    let lock_path = reconcile_lock_path(&store);

    std::env::remove_var("LOOM_STAGE_ID");
    std::env::set_var("LOOM_WORK_DIR", temp.path());
    let result = reconcile_graph();
    leave();

    assert!(
        result.is_ok(),
        "an unresolvable target must degrade to Ok(()), never propagate"
    );
    assert!(
        read_lock(&lock_path).is_none(),
        "with no state directory resolvable at all, nothing should ever be written"
    );
    assert!(
        !store.root().exists(),
        "no cache directory should be created for a checkout with no state directory"
    );
}

#[test]
#[serial]
fn reconcile_graph_with_a_stale_loom_work_dir_pin_creates_nothing() {
    // The exact phantom-state-directory scenario: LOOM_WORK_DIR names the
    // state directory itself (not the project root), and that directory was
    // deleted after being pinned — e.g. a leftover
    // `.claude/settings.local.json` entry from an earlier `loom clean
    // --state`. `WorkDir::new` now resolves the hint to itself rather than
    // double-appending `.loom/work`, but that path still does not exist on
    // disk, so the existence check must still refuse to reconcile.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let stale_work_dir_path = root.join(".loom").join("work");
    assert!(!stale_work_dir_path.exists());

    let work_dir = WorkDir::new(&stale_work_dir_path).unwrap();
    assert_eq!(
        work_dir.root(),
        stale_work_dir_path,
        "a hint naming the state directory directly must resolve to itself"
    );
    let store = ContextStore::open(&work_dir).unwrap();
    let lock_path = reconcile_lock_path(&store);

    std::env::remove_var("LOOM_STAGE_ID");
    std::env::set_var("LOOM_WORK_DIR", &stale_work_dir_path);
    let result = reconcile_graph();
    leave();

    assert!(result.is_ok());
    assert!(
        !stale_work_dir_path.exists(),
        "a stale LOOM_WORK_DIR pin must never cause the state directory to be materialized"
    );
    assert!(
        read_lock(&lock_path).is_none(),
        "a stale pin naming a deleted state directory must not reach the debounce lock"
    );
}

#[test]
#[serial]
fn reconcile_graph_in_a_stage_reconciles_that_stages_overlay_through_ensure_snapshot() {
    let temp = init_repo();
    let root = temp.path();
    let work_dir_path = root.join(".loom").join("work");
    let stage = Stage {
        id: "reconcile-graph-stage".to_string(),
        name: "Reconcile Graph Stage".to_string(),
        plan_id: Some("test-plan".to_string()),
        ..Stage::default()
    };
    crate::verify::transitions::create_stage(&stage, &work_dir_path).unwrap();

    std::env::remove_var("LOOM_STAGE_ID");
    std::env::set_var("LOOM_WORK_DIR", &work_dir_path);
    std::env::set_var("LOOM_STAGE_ID", &stage.id);
    let result = reconcile_graph();
    leave();

    assert!(result.is_ok());

    let work_dir = WorkDir::new(root).unwrap();
    let store = ContextStore::open(&work_dir).unwrap();
    let graph_store = GraphStore::new(store.root(), work_dir.root());
    let overlay = graph_store
        .load_overlay("test-plan", &stage.id)
        .unwrap()
        .expect("the stage's own overlay must be written");
    assert_eq!(overlay.revision, head_sha(root));
    assert!(
        !overlay.generation.is_empty(),
        "ensure_snapshot must stamp the stage overlay's working-tree generation"
    );
}

/// A pack that would trip `spawn_if_needed`'s own `stale || degraded` gate.
fn degraded_pack() -> ContextPack {
    ContextPack {
        query: "query".to_string(),
        scope: vec![Channel::Source],
        budget_tokens: 100,
        estimated_tokens: 0,
        structural_freshness: Freshness::default(),
        semantic_freshness: Freshness::default(),
        items: Vec::new(),
        unmet_required: Vec::new(),
        omitted: OmissionSummary::default(),
        dropped_terms: Vec::new(),
        degraded: Some("source graph base deadbeef missing".to_string()),
    }
}

#[test]
#[serial]
fn spawn_if_needed_does_nothing_when_the_pack_is_healthy() {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir_all(temp.path().join(".loom").join("work")).unwrap();
    let root = temp.path();

    let healthy = ContextPack {
        degraded: None,
        ..degraded_pack()
    };
    spawn_if_needed(&healthy, root);

    let work_dir = WorkDir::new(root).unwrap();
    let store = ContextStore::open(&work_dir).unwrap();
    assert!(
        !reconcile_lock_path(&store).exists(),
        "a healthy pack must never claim the lock at all"
    );
}

#[test]
#[serial]
fn spawn_if_needed_leaves_a_young_live_lock_untouched() {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir_all(temp.path().join(".loom").join("work")).unwrap();
    let root = temp.path();

    let work_dir = WorkDir::new(root).unwrap();
    let store = ContextStore::open(&work_dir).unwrap();
    // A legitimate spawn target (Task 2's `allowed_to_spawn` gate), same as a
    // `loom map`'d repository — without this, `try_spawn` would refuse
    // before ever reaching the lock decision this test means to exercise.
    store.ensure().unwrap();
    let lock_path = reconcile_lock_path(&store);
    // Our own pid: guaranteed alive for the duration of this test, so the
    // Skip branch is the only one `try_spawn` can take — no subprocess is
    // ever launched by this test.
    let now = unix_now();
    assert!(claim_lock(&lock_path, now, std::process::id(), false));

    spawn_if_needed(&degraded_pack(), root);

    assert_eq!(
        read_lock(&lock_path),
        Some((now, std::process::id())),
        "a young lock owned by a live pid must be left exactly as it was"
    );
}

// ---------------------------------------------------------------------------
// `allowed_to_spawn` — the inferred-root gate. A refused target must never
// even claim the debounce lock; an allowed one must reach `claim_lock` (and,
// suppressed by the test guard below, `spawn_detached`) exactly as before.
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn spawn_is_refused_against_an_inferred_root_with_no_existing_cache() {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir_all(temp.path().join(".loom").join("work")).unwrap();
    let root = temp.path();
    leave(); // this test's whole premise is that LOOM_WORK_DIR is unset

    spawn_if_needed(&degraded_pack(), root);

    let work_dir = WorkDir::new(root).unwrap();
    let store = ContextStore::open(&work_dir).unwrap();
    assert!(
        !reconcile_lock_path(&store).exists(),
        "an inferred root with no existing context cache must be refused \
         before the debounce lock is ever touched"
    );
}

#[test]
#[serial]
fn spawn_is_allowed_against_an_inferred_root_that_already_has_a_cache() {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir_all(temp.path().join(".loom").join("work")).unwrap();
    let root = temp.path();
    leave();

    let work_dir = WorkDir::new(root).unwrap();
    let store = ContextStore::open(&work_dir).unwrap();
    // Same as a `loom map`'d repository: the context cache already exists,
    // so this root is a legitimate target even though nobody named it.
    store.ensure().unwrap();

    spawn_if_needed(&degraded_pack(), root);

    assert!(
        reconcile_lock_path(&store).exists(),
        "a root with an existing context cache must be allowed to spawn"
    );
}

#[test]
#[serial]
fn spawn_is_allowed_when_loom_work_dir_was_explicitly_set() {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir_all(temp.path().join(".loom").join("work")).unwrap();
    let root = temp.path();
    std::env::set_var("LOOM_WORK_DIR", root.join(".loom").join("work"));

    spawn_if_needed(&degraded_pack(), root);
    leave();

    let work_dir = WorkDir::new(root).unwrap();
    let store = ContextStore::open(&work_dir).unwrap();
    assert!(
        reconcile_lock_path(&store).exists(),
        "an explicitly set LOOM_WORK_DIR must never be refused as inferred, \
         regardless of whether a context cache already exists"
    );
}

// ---------------------------------------------------------------------------
// `spawn_detached` — the process-creation guard. A test build must never
// create a real, process-group-leading child; see `SPAWN_ENABLED`'s doc.
// ---------------------------------------------------------------------------

#[test]
#[serial]
fn spawn_detached_is_suppressed_in_a_test_build() {
    let before = SUPPRESSED_SPAWNS.load(Ordering::SeqCst);

    let result = spawn_detached(Path::new("/nonexistent-reconcile-graph-test-target"));

    assert!(
        result.is_ok(),
        "a suppressed spawn must still report Ok(())"
    );
    assert_eq!(
        SUPPRESSED_SPAWNS.load(Ordering::SeqCst),
        before + 1,
        "a test build must record a suppression instead of creating a real \
         child process — a regression here is exactly the incident this \
         guard exists to prevent"
    );
}
