use super::pending::{pending_report, strict_should_fail};
use super::{change, note, resolve};
use crate::fs::memory::{append_to_spool, read_journal, MemoryEntry, MemoryEntryType};
use serial_test::serial;
use std::env;
use std::process::Command;
use tempfile::TempDir;

fn init_git_repo() -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    let run_git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(temp_dir.path())
            .output()
            .unwrap()
    };
    run_git(&["init", "--initial-branch=main"]);
    run_git(&["config", "user.email", "test@test.com"]);
    run_git(&["config", "user.name", "Test"]);
    temp_dir
}

struct EnvGuard {
    original_dir: std::path::PathBuf,
    original_stage_id: Option<String>,
    original_session_id: Option<String>,
}

impl EnvGuard {
    fn new() -> Self {
        Self {
            original_dir: env::current_dir().unwrap(),
            original_stage_id: env::var("LOOM_STAGE_ID").ok(),
            original_session_id: env::var("LOOM_SESSION_ID").ok(),
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        env::set_current_dir(&self.original_dir).unwrap();
        match &self.original_stage_id {
            Some(value) => env::set_var("LOOM_STAGE_ID", value),
            None => env::remove_var("LOOM_STAGE_ID"),
        }
        match &self.original_session_id {
            Some(value) => env::set_var("LOOM_SESSION_ID", value),
            None => env::remove_var("LOOM_SESSION_ID"),
        }
    }
}

fn record_note_and_id(repo: &TempDir, stage: &str, text: &str) -> String {
    env::set_var("LOOM_STAGE_ID", stage);
    note(text.to_string(), Vec::new(), None).unwrap();
    read_journal(&repo.path().join(".loom/work"), stage)
        .unwrap()
        .entries[0]
        .id
        .clone()
}

#[test]
#[serial]
fn resolve_records_a_receipt_referencing_the_event() {
    let _guard = EnvGuard::new();
    let repo = init_git_repo();
    env::set_current_dir(repo.path()).unwrap();
    let event_id = record_note_and_id(&repo, "resolve-stage", "remember me");

    resolve(
        event_id.clone(),
        "promoted".to_string(),
        Some("architecture/memory.md#receipts".to_string()),
        None,
        None,
    )
    .unwrap();

    let journal = read_journal(&repo.path().join(".loom/work"), "resolve-stage").unwrap();
    let receipt = journal.entries[1].receipt.as_ref().unwrap();
    assert_eq!(receipt.event_id, event_id);
    assert_eq!(
        journal.entries[1].content,
        "promoted into architecture/memory.md#receipts"
    );
}

#[test]
#[serial]
fn resolve_refuses_an_unknown_event_id() {
    let _guard = EnvGuard::new();
    let repo = init_git_repo();
    env::set_current_dir(repo.path()).unwrap();
    std::fs::create_dir_all(repo.path().join(".loom/work")).unwrap();
    let event_id = "00000000000000000000000000000000".to_string();

    let error = resolve(
        event_id.clone(),
        "discarded".to_string(),
        None,
        Some("obsolete".to_string()),
        None,
    )
    .unwrap_err();

    assert!(error.to_string().contains(&event_id));
}

#[test]
#[serial]
fn promoted_requires_a_target_and_discarded_requires_a_reason() {
    let _guard = EnvGuard::new();
    let repo = init_git_repo();
    env::set_current_dir(repo.path()).unwrap();
    let event_id = record_note_and_id(&repo, "requirements-stage", "requirements");

    let promoted = resolve(event_id.clone(), "promoted".to_string(), None, None, None).unwrap_err();
    let discarded = resolve(event_id, "discarded".to_string(), None, None, None).unwrap_err();

    assert!(promoted.to_string().contains("--target"));
    assert!(discarded.to_string().contains("--reason"));
}

#[test]
#[serial]
fn pending_lists_unreceipted_notes_and_omits_changes_and_receipted_entries() {
    let _guard = EnvGuard::new();
    let repo = init_git_repo();
    env::set_current_dir(repo.path()).unwrap();
    let stage = "pending-stage";
    let settled_id = record_note_and_id(&repo, stage, "settled");
    note("still pending".to_string(), Vec::new(), None).unwrap();
    change("implementation detail".to_string(), Vec::new(), None).unwrap();
    resolve(
        settled_id,
        "discarded".to_string(),
        None,
        Some("no longer useful".to_string()),
        None,
    )
    .unwrap();

    let report = pending_report(&repo.path().join(".loom/work"), None).unwrap();

    assert_eq!(report.pending.len(), 1);
    assert_eq!(report.pending[0].entry.content, "still pending");
    assert_eq!(report.changes_without_receipt, 1);
    assert_eq!(report.receipts, 1);
}

#[test]
fn pending_strict_exits_non_zero_only_when_something_is_pending() {
    assert!(!strict_should_fail(false, 1));
    assert!(!strict_should_fail(true, 0));
    assert!(strict_should_fail(true, 1));
}

#[test]
#[serial]
fn pending_merges_the_current_worktrees_spool() {
    let _guard = EnvGuard::new();
    let repo = init_git_repo();
    let stage = "spool-stage";
    let worktree_root = repo.path().join(".worktrees").join(stage);
    let work_dir = worktree_root.join(".loom/work");
    std::fs::create_dir_all(&work_dir).unwrap();
    env::set_current_dir(&worktree_root).unwrap();
    env::set_var("LOOM_STAGE_ID", stage);
    append_to_spool(
        &worktree_root,
        &MemoryEntry::new(MemoryEntryType::Note, "spooled note".to_string()),
    )
    .unwrap();

    let report = pending_report(&work_dir, None).unwrap();

    assert_eq!(report.pending.len(), 1);
    assert_eq!(report.pending[0].entry.content, "spooled note");
    assert_eq!(report.pending[0].stage, stage);
}
