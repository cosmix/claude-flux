use std::path::{Path, PathBuf};

use serial_test::serial;

use crate::fs::memory::{self, MemoryEntry, MemoryEntryType};
use crate::fs::stage_request;
use crate::models::stage::{Stage, StageStatus};
use crate::models::worktree::Worktree;
use crate::orchestrator::core::OrchestratorConfig;
use crate::plan::ExecutionGraph;
use crate::telemetry::{self, TelemetryEvent, TelemetryRecord};
use crate::verify::transitions::save_stage;

use super::Orchestrator;

/// `Orchestrator::new` eagerly constructs a `NativeBackend`, so it fails on a
/// headless runner with no terminal emulator installed. Pinning
/// `LOOM_TERMINAL` maps a name straight to an emulator without probing the
/// host. Serialized because the detection tests mutate the same process-global.
fn pin_terminal_env() -> Option<std::ffi::OsString> {
    let saved = std::env::var_os("LOOM_TERMINAL");
    // SAFETY: the test is serialized and restores the original value below.
    unsafe { std::env::set_var("LOOM_TERMINAL", "xterm") };
    saved
}

fn restore_terminal_env(saved: Option<std::ffi::OsString>) {
    match saved {
        // SAFETY: the serialized test is restoring its saved value.
        Some(value) => unsafe { std::env::set_var("LOOM_TERMINAL", value) },
        // SAFETY: the serialized test is restoring the variable's absence.
        None => unsafe { std::env::remove_var("LOOM_TERMINAL") },
    }
}

fn orchestrator_for(work_dir: &Path, repo_root: &Path) -> Orchestrator {
    let config = OrchestratorConfig {
        work_dir: work_dir.to_path_buf(),
        repo_root: repo_root.to_path_buf(),
        enable_skill_routing: false,
        ..Default::default()
    };
    let saved = pin_terminal_env();
    let constructed = Orchestrator::new(config, ExecutionGraph::build(Vec::new()).unwrap());
    restore_terminal_env(saved);
    constructed.unwrap()
}

/// Save a minimal stage record under `work_dir`, in the given status.
fn save_stage_with_status(work_dir: &Path, stage_id: &str, status: StageStatus) {
    let mut stage = Stage::new(stage_id.to_string(), None);
    stage.id = stage_id.to_string();
    stage.status = status;
    save_stage(&stage, work_dir).unwrap();
}

/// The worktree directory `drain_stage_spools` would derive for `stage_id`.
/// A plain directory is enough: spool operations are pure filesystem, no
/// git worktree required.
fn worktree_dir(repo_root: &Path, stage_id: &str) -> PathBuf {
    Worktree::worktree_path(repo_root, stage_id)
}

/// Append one note-type entry directly to a worktree's spool file, as the
/// sandboxed CLI fallback would.
fn spool_note(worktree_root: &Path, content: &str) {
    let entry = MemoryEntry::new(MemoryEntryType::Note, content.to_string());
    memory::append_to_spool(worktree_root, &entry).unwrap();
}

fn spool_telemetry(worktree_root: &Path, event: TelemetryEvent) {
    telemetry::spool::append_to_spool(
        worktree_root,
        &TelemetryRecord {
            at: chrono::Utc::now(),
            event,
        },
    )
    .unwrap();
}

#[test]
#[serial]
fn pending_entry_lands_in_journal_and_spool_is_emptied() {
    let temp = tempfile::tempdir().unwrap();
    let work_dir = temp.path().join(".loom").join("work");
    let stage_id = "spool-basic";
    save_stage_with_status(&work_dir, stage_id, StageStatus::Executing);
    let worktree_root = worktree_dir(temp.path(), stage_id);
    spool_note(&worktree_root, "found a pattern worth remembering");

    let mut orchestrator = orchestrator_for(&work_dir, temp.path());
    orchestrator.drain_stage_spools();

    let journal = memory::read_journal(&work_dir, stage_id).unwrap();
    assert_eq!(journal.entries.len(), 1);
    assert_eq!(
        journal.entries[0].content,
        "found a pattern worth remembering"
    );

    let pending = memory::read_pending(&worktree_root).unwrap();
    assert!(
        pending.is_empty(),
        "spool must be emptied after a successful drain"
    );
}

#[test]
#[serial]
fn telemetry_spool_is_drained_with_the_memory_spool() {
    let temp = tempfile::tempdir().unwrap();
    let work_dir = temp.path().join(".loom").join("work");
    let stage_id = "memory-and-telemetry";
    save_stage_with_status(&work_dir, stage_id, StageStatus::Executing);
    let worktree_root = worktree_dir(temp.path(), stage_id);
    spool_note(&worktree_root, "a final memory note");
    let event = TelemetryEvent::PromptAbstained {
        stage_id: Some(stage_id.to_string()),
        session_id: Some("session-1".to_string()),
        reason: "floor".to_string(),
    };
    spool_telemetry(&worktree_root, event.clone());

    let mut orchestrator = orchestrator_for(&work_dir, temp.path());
    orchestrator.drain_stage_spools();

    assert_eq!(
        memory::read_journal(&work_dir, stage_id)
            .unwrap()
            .entries
            .len(),
        1
    );
    let records = telemetry::read_events(&work_dir).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].event, event);
    assert!(telemetry::spool::read_pending(&worktree_root)
        .unwrap()
        .is_empty());
}

#[test]
#[serial]
fn entry_is_attributed_to_the_worktree_it_was_spooled_in_not_a_sibling_stage() {
    let temp = tempfile::tempdir().unwrap();
    let work_dir = temp.path().join(".loom").join("work");
    save_stage_with_status(&work_dir, "stage-a", StageStatus::Executing);
    save_stage_with_status(&work_dir, "stage-b", StageStatus::Executing);
    let worktree_a = worktree_dir(temp.path(), "stage-a");
    spool_note(&worktree_a, "belongs to stage-a only");
    // stage-b never spooled anything.

    let mut orchestrator = orchestrator_for(&work_dir, temp.path());
    orchestrator.drain_stage_spools();

    let journal_a = memory::read_journal(&work_dir, "stage-a").unwrap();
    assert_eq!(journal_a.entries.len(), 1);
    assert_eq!(journal_a.entries[0].content, "belongs to stage-a only");

    assert!(
        !memory::memory_file_path(&work_dir, "stage-b").exists(),
        "a stage with no spool must not gain an entry from a sibling's spool"
    );
}

#[test]
#[serial]
fn stage_with_no_spool_file_is_a_silent_no_op() {
    let temp = tempfile::tempdir().unwrap();
    let work_dir = temp.path().join(".loom").join("work");
    let stage_id = "no-spool";
    save_stage_with_status(&work_dir, stage_id, StageStatus::Executing);
    // No .loom/memory-spool.jsonl ever written for this stage.

    let mut orchestrator = orchestrator_for(&work_dir, temp.path());
    orchestrator.drain_stage_spools();

    assert!(
        !memory::memory_file_path(&work_dir, stage_id).exists(),
        "no journal should be created when nothing was ever spooled"
    );
}

#[test]
#[serial]
fn drain_still_happens_for_a_stage_that_is_not_executing() {
    let temp = tempfile::tempdir().unwrap();
    let work_dir = temp.path().join(".loom").join("work");
    let stage_id = "already-completed";
    save_stage_with_status(&work_dir, stage_id, StageStatus::Completed);
    let worktree_root = worktree_dir(temp.path(), stage_id);
    spool_note(
        &worktree_root,
        "recorded moments before loom stage complete exited",
    );

    let mut orchestrator = orchestrator_for(&work_dir, temp.path());
    orchestrator.drain_stage_spools();

    let journal = memory::read_journal(&work_dir, stage_id).unwrap();
    assert_eq!(
        journal.entries.len(),
        1,
        "a Completed (not Executing) stage must still be drained - its last \
         entries land right before `loom stage complete` exits"
    );
}

#[test]
#[serial]
fn an_invalid_entry_is_skipped_without_blocking_a_valid_entry_and_the_spool_is_truncated() {
    let temp = tempfile::tempdir().unwrap();
    let work_dir = temp.path().join(".loom").join("work");
    let stage_id = "mixed-spool";
    save_stage_with_status(&work_dir, stage_id, StageStatus::Executing);
    let worktree_root = worktree_dir(temp.path(), stage_id);
    spool_note(&worktree_root, &"x".repeat(2001)); // fails validate_content
    spool_note(&worktree_root, "a good entry");

    let mut orchestrator = orchestrator_for(&work_dir, temp.path());
    orchestrator.drain_stage_spools();

    let journal = memory::read_journal(&work_dir, stage_id).unwrap();
    assert_eq!(
        journal.entries.len(),
        1,
        "the over-long entry must be skipped without blocking the good one"
    );
    assert_eq!(journal.entries[0].content, "a good entry");

    let pending = memory::read_pending(&worktree_root).unwrap();
    assert!(
        pending.is_empty(),
        "the spool must still be truncated even though one entry was invalid"
    );
}

/// Append one block request directly to a worktree's request spool, as the
/// sandboxed CLI fallback would when its socket syscalls are denied.
fn spool_block(worktree_root: &Path, reason: &str) {
    stage_request::append_to_spool(
        worktree_root,
        &stage_request::StageRequest::Block {
            reason: reason.to_string(),
        },
    )
    .unwrap();
}

#[test]
#[serial]
fn a_spooled_block_is_applied_on_the_poll_tick_and_the_spool_is_emptied() {
    let temp = tempfile::tempdir().unwrap();
    let work_dir = temp.path().join(".loom").join("work");
    let stage_id = "request-basic";
    save_stage_with_status(&work_dir, stage_id, StageStatus::Executing);
    let worktree_root = worktree_dir(temp.path(), stage_id);
    spool_block(
        &worktree_root,
        "criterion 2 names a binary this plan never builds",
    );

    let mut orchestrator = orchestrator_for(&work_dir, temp.path());
    orchestrator.drain_stage_spools();

    let stage = crate::verify::transitions::load_stage(stage_id, &work_dir).unwrap();
    assert_eq!(stage.status, StageStatus::Blocked);
    assert_eq!(
        stage.close_reason.as_deref(),
        Some("criterion 2 names a binary this plan never builds")
    );
    assert!(
        stage_request::read_pending(&worktree_root)
            .unwrap()
            .is_empty(),
        "the request spool must be emptied after a successful drain"
    );
}

#[test]
#[serial]
fn a_request_is_attributed_to_the_worktree_it_was_spooled_in_not_a_sibling_stage() {
    let temp = tempfile::tempdir().unwrap();
    let work_dir = temp.path().join(".loom").join("work");
    save_stage_with_status(&work_dir, "req-stage-a", StageStatus::Executing);
    save_stage_with_status(&work_dir, "req-stage-b", StageStatus::Executing);
    spool_block(
        &worktree_dir(temp.path(), "req-stage-a"),
        "only stage-a is stuck",
    );
    // req-stage-b never spooled anything.

    let mut orchestrator = orchestrator_for(&work_dir, temp.path());
    orchestrator.drain_stage_spools();

    assert_eq!(
        crate::verify::transitions::load_stage("req-stage-a", &work_dir)
            .unwrap()
            .status,
        StageStatus::Blocked
    );
    assert_eq!(
        crate::verify::transitions::load_stage("req-stage-b", &work_dir)
            .unwrap()
            .status,
        StageStatus::Executing,
        "a stage with no spool must not be blocked by a sibling's request"
    );
}

#[test]
#[serial]
fn one_stages_unappliable_request_does_not_stop_another_stages_drain() {
    let temp = tempfile::tempdir().unwrap();
    let work_dir = temp.path().join(".loom").join("work");
    // A Completed stage refuses the block; the refusal must not leak into the
    // sibling's pass.
    save_stage_with_status(&work_dir, "req-refused", StageStatus::Completed);
    save_stage_with_status(&work_dir, "req-applied", StageStatus::Executing);
    spool_block(&worktree_dir(temp.path(), "req-refused"), "too late");
    spool_block(&worktree_dir(temp.path(), "req-applied"), "genuinely stuck");

    let mut orchestrator = orchestrator_for(&work_dir, temp.path());
    orchestrator.drain_stage_spools();

    assert_eq!(
        crate::verify::transitions::load_stage("req-refused", &work_dir)
            .unwrap()
            .status,
        StageStatus::Completed
    );
    assert_eq!(
        crate::verify::transitions::load_stage("req-applied", &work_dir)
            .unwrap()
            .status,
        StageStatus::Blocked
    );
}

#[test]
#[serial]
fn a_stage_with_only_a_memory_spool_is_unaffected_by_the_request_drain() {
    let temp = tempfile::tempdir().unwrap();
    let work_dir = temp.path().join(".loom").join("work");
    let stage_id = "memory-only";
    save_stage_with_status(&work_dir, stage_id, StageStatus::Executing);
    let worktree_root = worktree_dir(temp.path(), stage_id);
    spool_note(&worktree_root, "a note, and no control request");

    let mut orchestrator = orchestrator_for(&work_dir, temp.path());
    orchestrator.drain_stage_spools();

    assert_eq!(
        crate::verify::transitions::load_stage(stage_id, &work_dir)
            .unwrap()
            .status,
        StageStatus::Executing
    );
    assert_eq!(
        memory::read_journal(&work_dir, stage_id)
            .unwrap()
            .entries
            .len(),
        1
    );
}
