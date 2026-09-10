//! Tests for `loom handoff`: identity resolution, and the turn-ending
//! transition that a sandboxed worktree session cannot always write.

use super::*;
use serial_test::serial;

/// Restores cwd on drop. `execute()` resolves its work dir from the process
/// cwd, so the test below must mutate process-global state and clean up
/// after itself even on panic (mirrors `commands/memory/handlers/tests.rs`).
struct CwdGuard {
    original_dir: std::path::PathBuf,
}

impl CwdGuard {
    fn new() -> Self {
        Self {
            original_dir: env::current_dir().unwrap(),
        }
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        env::set_current_dir(&self.original_dir).unwrap();
    }
}

#[test]
fn test_resolve_stage_id_from_arg() {
    let stage_arg = Some("test-stage".to_string());
    let result = resolve_stage_id(&stage_arg);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "test-stage");
}

#[test]
#[serial]
fn test_resolve_stage_id_from_env() {
    let original = env::var("LOOM_STAGE_ID").ok();
    env::set_var("LOOM_STAGE_ID", "env-stage");
    let stage_arg = None;
    let result = resolve_stage_id(&stage_arg);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "env-stage");
    // Restore original value
    match original {
        Some(val) => env::set_var("LOOM_STAGE_ID", val),
        None => env::remove_var("LOOM_STAGE_ID"),
    }
}

#[test]
#[serial]
fn test_resolve_stage_id_missing() {
    let original = env::var("LOOM_STAGE_ID").ok();
    env::remove_var("LOOM_STAGE_ID");
    let stage_arg = None;
    let result = resolve_stage_id(&stage_arg);
    assert!(result.is_err());
    // Restore original value
    if let Some(val) = original {
        env::set_var("LOOM_STAGE_ID", val);
    }
}

#[test]
fn test_resolve_session_id_from_arg() {
    let session_arg = Some("test-session".to_string());
    let result = resolve_session_id(&session_arg);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "test-session");
}

#[test]
fn test_build_handoff_content() {
    let session_id = "test-session".to_string();
    let stage_id = "test-stage".to_string();

    let content = HandoffContent::new(session_id.clone(), stage_id.clone())
        .with_goals("Test goals".to_string())
        .with_current_branch(Some("main".to_string()))
        .with_files_modified(vec!["file1.rs".to_string(), "file2.rs".to_string()]);

    assert_eq!(content.session_id, session_id);
    assert_eq!(content.stage_id, stage_id);
    assert_eq!(content.goals, "Test goals");
    assert_eq!(content.current_branch, Some("main".to_string()));
    assert_eq!(content.files_modified.len(), 2);
}

/// `--trigger ceiling` is the agent saying "I am out of room": it must end
/// the turn, not just leave a document behind. Without the transition the
/// daemon never kills the session, and the stage sits Executing behind an
/// agent that has already stopped working.
#[test]
fn ceiling_trigger_marks_an_executing_stage_needing_handoff() {
    use crate::verify::transitions::create_stage;

    let temp = tempfile::tempdir().unwrap();
    let work_dir = temp.path();
    let mut stage = Stage::new("ceiling".to_string(), None);
    stage.id = "ceiling".to_string();
    stage.status = StageStatus::Executing;
    create_stage(&stage, work_dir).unwrap();

    end_turn_for_handoff("ceiling", work_dir, Path::new("ceiling-handoff-001.md")).unwrap();

    let reloaded = load_stage("ceiling", work_dir).unwrap();
    assert_eq!(reloaded.status, StageStatus::NeedsHandoff);
}

/// A stage that already moved on has an authority of its own; a late
/// CLI-side write must not drag it back out of a terminal state.
#[test]
fn ceiling_trigger_leaves_a_stage_that_already_moved_on() {
    use crate::verify::transitions::create_stage;

    let temp = tempfile::tempdir().unwrap();
    let work_dir = temp.path();
    let mut stage = Stage::new("done".to_string(), None);
    stage.id = "done".to_string();
    stage.status = StageStatus::Completed;
    create_stage(&stage, work_dir).unwrap();

    end_turn_for_handoff("done", work_dir, Path::new("done-handoff-001.md"))
        .expect("a stage that already moved on is a benign no-op, not a failure");

    let reloaded = load_stage("done", work_dir).unwrap();
    assert_eq!(reloaded.status, StageStatus::Completed);
}

/// The defect this whole path exists for: a worktree session's sandbox
/// grants the state directory's `handoffs/` but not its `stages/`, so the transition write
/// fails while the document lands. Reporting that as a warning and exiting
/// 0 told the agent its handoff was complete; the stage stayed `Executing`
/// and the daemon's status-triggered recovery never armed.
#[test]
fn a_failed_transition_is_an_error_that_says_the_document_stands() {
    let temp = tempfile::tempdir().unwrap();
    let work_dir = temp.path();
    // No stage record at all: `update_stage` cannot read-modify-write one,
    // which is the same failure class as being unable to write it.
    let error = end_turn_for_handoff(
        "unwritable",
        work_dir,
        Path::new("/w/.loom/work/handoffs/unwritable-handoff-002.md"),
    )
    .expect_err("a transition that did not happen must not report success");

    let rendered = format!("{error:#}");
    assert!(
        rendered.contains("Could not mark stage 'unwritable' NeedsHandoff"),
        "{rendered}"
    );
    assert!(
        rendered.contains("unwritable-handoff-002.md"),
        "the agent must be told the document it wrote still stands: {rendered}"
    );
    assert!(
        rendered.contains("End your turn now"),
        "the message must not read as 'retry the handoff': {rendered}"
    );
}

/// The defect: journals are `memory/<stage_id>.md`, but `execute()` used to
/// read `memory/<session_id>.md`, so a CLI-triggered handoff (the pre-compact
/// hook, CLAUDE.md Rule 3) always carried an empty memory section even when
/// the stage journal held real entries.
#[test]
#[serial]
fn a_cli_handoff_for_a_stage_with_a_journal_carries_its_memory() {
    use crate::fs::memory::{append_entry, MemoryEntry, MemoryEntryType};
    use crate::verify::transitions::create_stage;

    let _guard = CwdGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let work_dir = temp.path().join(".loom").join("work");
    std::fs::create_dir_all(&work_dir).unwrap();
    std::fs::write(work_dir.join("config.toml"), "").unwrap();

    let mut stage = Stage::new("journaled".to_string(), None);
    stage.id = "journaled".to_string();
    create_stage(&stage, &work_dir).unwrap();

    let entry = MemoryEntry::new(
        MemoryEntryType::Note,
        "found: the handoff wiring test itself".to_string(),
    );
    append_entry(&work_dir, "journaled", &entry).unwrap();

    env::set_current_dir(temp.path()).unwrap();
    execute(
        Some("journaled".to_string()),
        Some("some-session".to_string()),
        "manual".to_string(),
        None,
    )
    .unwrap();

    let handoffs_dir = work_dir.join("handoffs");
    let handoff_file = std::fs::read_dir(&handoffs_dir)
        .unwrap()
        .find_map(|entry| entry.ok().map(|e| e.path()))
        .expect("execute() should have written a handoff file");
    let handoff_contents = std::fs::read_to_string(&handoff_file).unwrap();

    assert!(
        handoff_contents.contains("## Stage Memory"),
        "a CLI handoff for a stage with a journal must carry it: {handoff_contents}"
    );
}

/// `--trigger ceiling` is the only trigger that asks for a takedown, so it
/// is the only one whose document may carry the origin the daemon's handoff
/// watch acts on. A precompact or session_end document must not look like a
/// request to end the turn.
#[test]
fn only_the_ceiling_trigger_stamps_the_agent_ceiling_origin() {
    assert_eq!(
        origin_for(CEILING_TRIGGER),
        Some(HandoffOrigin::AgentCeiling)
    );
    for routine in ["precompact", "session_end", "manual"] {
        assert_eq!(
            origin_for(routine),
            None,
            "'{routine}' documents a session that keeps working"
        );
    }
}
