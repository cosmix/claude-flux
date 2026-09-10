use super::*;
use crate::fs::memory::types::MemoryEntryType;
use std::fs;
use tempfile::TempDir;

#[test]
fn append_then_read_round_trips() {
    let dir = TempDir::new().unwrap();
    let entry = MemoryEntry::new(MemoryEntryType::Note, "found a thing".to_string());

    append_to_spool(dir.path(), &entry).unwrap();
    let pending = read_pending(dir.path()).unwrap();

    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].content, "found a thing");
    assert_eq!(pending[0].entry_type, MemoryEntryType::Note);
}

#[test]
fn multi_entry_ordering_is_preserved() {
    let dir = TempDir::new().unwrap();
    for i in 0..5 {
        let entry = MemoryEntry::new(MemoryEntryType::Note, format!("entry {i}"));
        append_to_spool(dir.path(), &entry).unwrap();
    }

    let pending = read_pending(dir.path()).unwrap();

    let contents: Vec<_> = pending.iter().map(|e| e.content.as_str()).collect();
    assert_eq!(
        contents,
        vec!["entry 0", "entry 1", "entry 2", "entry 3", "entry 4"]
    );
}

#[test]
fn malformed_line_is_skipped_while_good_lines_survive() {
    let dir = TempDir::new().unwrap();
    append_to_spool(
        dir.path(),
        &MemoryEntry::new(MemoryEntryType::Note, "good one".to_string()),
    )
    .unwrap();

    // Hand-corrupt the spool with a line that isn't valid JSON, then add a
    // second good entry after it.
    let path = spool_path(dir.path());
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    use std::io::Write;
    writeln!(file, "not valid json").unwrap();
    drop(file);
    append_to_spool(
        dir.path(),
        &MemoryEntry::new(MemoryEntryType::Note, "good two".to_string()),
    )
    .unwrap();

    let pending = read_pending(dir.path()).unwrap();
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].content, "good one");
    assert_eq!(pending[1].content, "good two");

    let mut sunk = Vec::new();
    let outcome = drain_spool(dir.path(), &mut |entry| {
        sunk.push(entry.content.clone());
        Ok(())
    })
    .unwrap();

    assert_eq!(outcome.drained, 2);
    assert_eq!(outcome.skipped_malformed, 1);
    assert_eq!(sunk, vec!["good one", "good two"]);
}

#[test]
fn blank_lines_are_ignored_not_counted_malformed() {
    let dir = TempDir::new().unwrap();
    let path = spool_path(dir.path());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let entry = MemoryEntry::new(MemoryEntryType::Note, "surrounded by blanks".to_string());
    let line = serde_json::to_string(&entry).unwrap();
    fs::write(&path, format!("\n{line}\n\n")).unwrap();

    let pending = read_pending(dir.path()).unwrap();
    assert_eq!(pending.len(), 1);

    let outcome = drain_spool(dir.path(), &mut |_| Ok(())).unwrap();
    assert_eq!(outcome.drained, 1);
    assert_eq!(outcome.skipped_malformed, 0);
}

#[test]
fn drain_truncates_on_success() {
    let dir = TempDir::new().unwrap();
    append_to_spool(
        dir.path(),
        &MemoryEntry::new(MemoryEntryType::Note, "one".to_string()),
    )
    .unwrap();

    let outcome = drain_spool(dir.path(), &mut |_| Ok(())).unwrap();
    assert_eq!(outcome.drained, 1);

    let pending_after = read_pending(dir.path()).unwrap();
    assert!(pending_after.is_empty());
    let raw = fs::read_to_string(spool_path(dir.path())).unwrap();
    assert!(raw.is_empty());
}

#[test]
fn drain_does_not_truncate_when_sink_errors() {
    let dir = TempDir::new().unwrap();
    append_to_spool(
        dir.path(),
        &MemoryEntry::new(MemoryEntryType::Note, "one".to_string()),
    )
    .unwrap();
    append_to_spool(
        dir.path(),
        &MemoryEntry::new(MemoryEntryType::Note, "two".to_string()),
    )
    .unwrap();

    let result = drain_spool(dir.path(), &mut |entry| {
        if entry.content == "two" {
            anyhow::bail!("sink refused entry");
        }
        Ok(())
    });

    assert!(result.is_err());

    // Nothing was truncated - both entries, including the one the sink
    // already accepted, redeliver on the next pass.
    let pending = read_pending(dir.path()).unwrap();
    assert_eq!(pending.len(), 2);
}

#[test]
fn size_cap_refuses_an_append() {
    let dir = TempDir::new().unwrap();
    let path = spool_path(dir.path());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    // Pre-fill the spool past the cap without going through append_to_spool.
    fs::write(&path, vec![b'x'; SPOOL_MAX_BYTES as usize]).unwrap();

    let result = append_to_spool(
        dir.path(),
        &MemoryEntry::new(MemoryEntryType::Note, "overflow".to_string()),
    );

    assert!(result.is_err());
    let message = result.unwrap_err().to_string();
    assert!(message.contains("cap"));
    assert!(message.contains(&path.display().to_string()));
}

#[test]
fn entry_with_newlines_and_quotes_round_trips_intact() {
    let dir = TempDir::new().unwrap();
    let content = "line one\nline \"two\" has quotes\nline three".to_string();
    let entry = MemoryEntry::with_context(
        MemoryEntryType::Decision,
        content.clone(),
        "context with \"quotes\" and\nnewlines too".to_string(),
    );

    append_to_spool(dir.path(), &entry).unwrap();
    let pending = read_pending(dir.path()).unwrap();

    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].content, content);
    assert_eq!(
        pending[0].context.as_deref(),
        Some("context with \"quotes\" and\nnewlines too")
    );

    // The spool itself must still be exactly one line per entry.
    let raw = fs::read_to_string(spool_path(dir.path())).unwrap();
    assert_eq!(raw.lines().count(), 1);
}

#[test]
fn missing_spool_is_cheap_and_creates_nothing() {
    let dir = TempDir::new().unwrap();

    assert!(read_pending(dir.path()).unwrap().is_empty());
    assert_eq!(
        drain_spool(dir.path(), &mut |_| Ok(())).unwrap(),
        DrainOutcome::default()
    );
    assert!(!spool_path(dir.path()).exists());
    assert!(!dir.path().join(".loom").exists());
}

#[test]
fn drain_into_journal_moves_a_spooled_entry_and_empties_the_spool() {
    let worktree = TempDir::new().unwrap();
    let work_dir = TempDir::new().unwrap();
    let stage_id = "stage-under-test";

    append_to_spool(
        worktree.path(),
        &MemoryEntry::new(MemoryEntryType::Note, "worth remembering".to_string()),
    )
    .unwrap();

    let outcome = drain_into_journal(work_dir.path(), stage_id, worktree.path()).unwrap();
    assert_eq!(outcome.drained, 1);
    assert_eq!(outcome.skipped_malformed, 0);

    let journal_path = work_dir
        .path()
        .join("memory")
        .join(format!("{stage_id}.md"));
    let journal = fs::read_to_string(&journal_path).unwrap();
    assert!(journal.contains("worth remembering"));

    assert!(read_pending(worktree.path()).unwrap().is_empty());
}

#[test]
fn drain_into_journal_skips_an_invalid_entry_but_lands_the_valid_one() {
    let worktree = TempDir::new().unwrap();
    let work_dir = TempDir::new().unwrap();
    let stage_id = "stage-under-test";

    // Fails validate_content: over the 2000-char cap.
    append_to_spool(
        worktree.path(),
        &MemoryEntry::new(MemoryEntryType::Note, "x".repeat(2001)),
    )
    .unwrap();
    append_to_spool(
        worktree.path(),
        &MemoryEntry::new(MemoryEntryType::Note, "a valid entry".to_string()),
    )
    .unwrap();

    let outcome = drain_into_journal(work_dir.path(), stage_id, worktree.path()).unwrap();
    assert_eq!(outcome.drained, 1);
    assert_eq!(outcome.skipped_malformed, 1);

    let journal_path = work_dir
        .path()
        .join("memory")
        .join(format!("{stage_id}.md"));
    let journal = fs::read_to_string(&journal_path).unwrap();
    assert!(journal.contains("a valid entry"));
    assert!(!journal.contains(&"x".repeat(2001)));

    assert!(read_pending(worktree.path()).unwrap().is_empty());
}

#[test]
fn drain_into_journal_on_a_worktree_with_no_spool_is_a_no_op() {
    let worktree = TempDir::new().unwrap();
    let work_dir = TempDir::new().unwrap();

    let outcome = drain_into_journal(work_dir.path(), "no-spool-stage", worktree.path()).unwrap();
    assert_eq!(outcome, DrainOutcome::default());

    assert!(!work_dir.path().join("memory").exists());
    assert!(!worktree.path().join(".loom").exists());
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
    let (outside, victim) = victim_outside("memory-spool.jsonl");
    std::os::unix::fs::symlink(outside.path(), worktree.path().join(".loom")).unwrap();

    assert!(drain_spool(worktree.path(), &mut |_| Ok(())).is_err());
    assert_eq!(fs::read_to_string(&victim).unwrap(), VICTIM);
}

#[test]
fn drain_refuses_a_spool_hard_linked_to_an_outside_file() {
    let worktree = worktree_with_loom_dir();
    let (_outside, victim) = victim_outside("victim.txt");
    fs::hard_link(&victim, spool_path(worktree.path())).unwrap();

    assert!(drain_spool(worktree.path(), &mut |_| Ok(())).is_err());
    assert_eq!(fs::read_to_string(&victim).unwrap(), VICTIM);
}

#[test]
fn drain_refuses_a_fifo_spool_instead_of_blocking_on_it() {
    let worktree = worktree_with_loom_dir();
    nix::unistd::mkfifo(&spool_path(worktree.path()), nix::sys::stat::Mode::S_IRWXU).unwrap();

    assert!(drain_spool(worktree.path(), &mut |_| Ok(())).is_err());
}
