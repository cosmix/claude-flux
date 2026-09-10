use super::*;
use std::fs;
use tempfile::TempDir;

fn block(reason: &str) -> StageRequest {
    StageRequest::Block {
        reason: reason.to_string(),
    }
}

fn dispute(criterion_index: usize) -> StageRequest {
    StageRequest::Dispute {
        criterion_index,
        reason: "the criterion names a binary this stage never builds".to_string(),
        evidence_commit: Some("abc1234".to_string()),
        failure_output: Some("command not found".to_string()),
    }
}

#[test]
fn append_then_read_round_trips_both_variants() {
    let dir = TempDir::new().unwrap();

    append_to_spool(dir.path(), &block("criterion 4 is unrunnable")).unwrap();
    append_to_spool(dir.path(), &dispute(2)).unwrap();
    let pending = read_pending(dir.path()).unwrap();

    assert_eq!(
        pending,
        vec![block("criterion 4 is unrunnable"), dispute(2)]
    );
}

#[test]
fn a_reason_containing_newlines_still_occupies_one_line() {
    let dir = TempDir::new().unwrap();

    append_to_spool(dir.path(), &block("line one\nline two\nline three")).unwrap();

    let raw = fs::read_to_string(spool_path(dir.path())).unwrap();
    assert_eq!(raw.lines().count(), 1, "spool line count: {raw:?}");
    assert_eq!(
        read_pending(dir.path()).unwrap(),
        vec![block("line one\nline two\nline three")]
    );
}

#[test]
fn the_payload_carries_no_stage_id() {
    // Attribution comes from the worktree the daemon drains, never from the
    // payload; a stage_id field here would be a forgeable claim.
    let dir = TempDir::new().unwrap();
    append_to_spool(dir.path(), &block("stuck")).unwrap();

    let raw = fs::read_to_string(spool_path(dir.path())).unwrap();

    assert!(
        !raw.contains("stage_id") && !raw.contains("session_id"),
        "the spooled payload must not claim an identity: {raw}"
    );
}

#[test]
fn reading_a_worktree_with_no_spool_creates_nothing() {
    let dir = TempDir::new().unwrap();

    assert!(read_pending(dir.path()).unwrap().is_empty());
    assert!(
        !dir.path().join(".loom").exists(),
        "the common no-spool case must not create the .loom directory"
    );
}

#[test]
fn a_malformed_line_is_skipped_counted_and_does_not_block_the_next_request() {
    let dir = TempDir::new().unwrap();
    append_to_spool(dir.path(), &block("first")).unwrap();

    // Hand-corrupt the spool with a line that isn't valid JSON, plus one that
    // is valid JSON but not a StageRequest, then queue a good request after.
    let path = spool_path(dir.path());
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    use std::io::Write;
    writeln!(file, "not valid json").unwrap();
    writeln!(file, r#"{{"request":"detonate"}}"#).unwrap();
    drop(file);
    append_to_spool(dir.path(), &block("second")).unwrap();

    let mut sunk = Vec::new();
    let outcome = drain_spool(dir.path(), &mut |request| {
        sunk.push(request.clone());
        Ok(())
    })
    .unwrap();

    assert_eq!(outcome.applied, 2);
    assert_eq!(outcome.skipped, 2);
    assert_eq!(sunk, vec![block("first"), block("second")]);
}

#[test]
fn blank_lines_are_ignored_not_counted_malformed() {
    let dir = TempDir::new().unwrap();
    let path = spool_path(dir.path());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let line = serde_json::to_string(&block("surrounded by blanks")).unwrap();
    fs::write(&path, format!("\n\n{line}\n\n")).unwrap();

    let outcome = drain_spool(dir.path(), &mut |_| Ok(())).unwrap();

    assert_eq!(outcome.applied, 1);
    assert_eq!(outcome.skipped, 0);
}

#[test]
fn a_sink_error_leaves_the_whole_batch_pending_for_the_next_pass() {
    let dir = TempDir::new().unwrap();
    append_to_spool(dir.path(), &block("first")).unwrap();
    append_to_spool(dir.path(), &block("second")).unwrap();

    let error = drain_spool(dir.path(), &mut |_| {
        anyhow::bail!("stage file is unreadable")
    })
    .unwrap_err();

    assert!(error.to_string().contains("unreadable"));
    assert_eq!(
        read_pending(dir.path()).unwrap().len(),
        2,
        "a failed pass must not truncate: the batch redelivers next tick"
    );
}

#[test]
fn an_append_past_the_size_cap_is_refused() {
    let dir = TempDir::new().unwrap();
    let path = spool_path(dir.path());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "x".repeat(SPOOL_MAX_BYTES as usize)).unwrap();

    let error = append_to_spool(dir.path(), &block("one too many")).unwrap_err();

    assert!(
        error.to_string().contains("cap"),
        "the refusal must name the cap: {error}"
    );
}

#[test]
fn a_request_larger_than_the_daemon_accepts_is_refused_before_it_is_written() {
    // Parity with the socket path's MAX_REQUEST_BYTES frame limit: the spool
    // must not become the weaker gate.
    let dir = TempDir::new().unwrap();
    let oversized = block(&"x".repeat(MAX_REQUEST_BYTES + 1));

    let error = append_to_spool(dir.path(), &oversized).unwrap_err();

    assert!(
        error.to_string().contains("limit the daemon accepts"),
        "the refusal must name the daemon's limit: {error}"
    );
    assert!(
        !spool_path(dir.path()).exists(),
        "a refused request must not have been partially written"
    );
}

const VICTIM: &str = "outside the worktree; a drain must never read or truncate this\n";

/// A file named `name` outside any worktree, holding `VICTIM`; the returned
/// guard keeps its directory alive.
fn victim_outside(name: &str) -> (TempDir, std::path::PathBuf) {
    let outside = TempDir::new().unwrap();
    let victim = outside.path().join(name);
    fs::write(&victim, VICTIM).unwrap();
    (outside, victim)
}

/// A worktree holding an empty `.loom/` directory, ready for a planted spool.
fn worktree_with_loom_dir() -> TempDir {
    let worktree = TempDir::new().unwrap();
    fs::create_dir_all(worktree.path().join(".loom")).unwrap();
    worktree
}

#[test]
fn drain_refuses_a_spool_symlinked_outside_the_worktree() {
    let worktree = worktree_with_loom_dir();
    let (_outside, victim) = victim_outside("victim.txt");
    std::os::unix::fs::symlink(&victim, spool_path(worktree.path())).unwrap();

    let error = drain_spool(worktree.path(), &mut |_| Ok(())).unwrap_err();

    assert!(
        format!("{error:#}").contains("was not drained"),
        "{error:#}"
    );
    assert_eq!(fs::read_to_string(&victim).unwrap(), VICTIM);
}

#[test]
fn drain_refuses_a_spool_under_a_symlinked_loom_directory() {
    let worktree = TempDir::new().unwrap();
    let (outside, victim) = victim_outside("stage-request-spool.jsonl");
    std::os::unix::fs::symlink(outside.path(), worktree.path().join(".loom")).unwrap();

    let error = drain_spool(worktree.path(), &mut |_| Ok(())).unwrap_err();

    assert!(
        format!("{error:#}").contains("was not drained"),
        "{error:#}"
    );
    assert_eq!(fs::read_to_string(&victim).unwrap(), VICTIM);
}
