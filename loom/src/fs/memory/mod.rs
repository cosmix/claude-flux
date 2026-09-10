//! Per-session memory journal for continuous fact recording.
//!
//! Journals persist typed stage events for recitation, handoff, and review.

mod archive;
mod constants;
mod export;
mod parser;
mod persistence;
mod query;
mod spool;
mod storage;
mod types;

pub use types::{MemoryEntry, MemoryEntryType, MemoryJournal, Receipt, ReceiptOutcome};

pub use archive::archive_run_state;

pub use storage::{
    append_entry, create_journal, init_memory_dir, memory_dir, memory_file_path, read_journal,
    write_summary,
};

pub use query::{generate_summary, get_recent_entries, query_entries};

pub use export::{format_memory_for_handoff, format_memory_for_signal};

pub use persistence::{
    extract_key_notes, list_journals, preserve_for_crash, validate_content, validate_evidence,
};

pub use spool::{
    append_to_spool, drain_into_journal, drain_spool, read_pending, spool_path, DrainOutcome,
    SPOOL_MAX_BYTES, SPOOL_RELPATH,
};

#[cfg(test)]
#[path = "tests/archive.rs"]
mod archive_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use std::sync::Arc;
    use tempfile::TempDir;

    #[test]
    fn test_entry_type_display() {
        assert_eq!(MemoryEntryType::Note.to_string(), "note");
        assert_eq!(MemoryEntryType::Decision.to_string(), "decision");
        assert_eq!(MemoryEntryType::Question.to_string(), "question");
        assert_eq!(MemoryEntryType::Receipt.to_string(), "receipt");
    }

    #[test]
    fn test_entry_type_from_str() {
        assert_eq!(
            "note".parse::<MemoryEntryType>().unwrap(),
            MemoryEntryType::Note
        );
        assert_eq!(
            "DECISION".parse::<MemoryEntryType>().unwrap(),
            MemoryEntryType::Decision
        );
        assert_eq!(
            "questions".parse::<MemoryEntryType>().unwrap(),
            MemoryEntryType::Question
        );
        assert_eq!(
            "receipts".parse::<MemoryEntryType>().unwrap(),
            MemoryEntryType::Receipt
        );
        assert!("invalid".parse::<MemoryEntryType>().is_err());
    }

    #[test]
    fn test_create_and_read_journal() {
        let temp_dir = TempDir::new().unwrap();
        let work_dir = temp_dir.path();

        let stage_id = "test-stage";
        create_journal(work_dir, stage_id).unwrap();

        let journal = read_journal(work_dir, stage_id).unwrap();
        assert_eq!(journal.stage_id, stage_id);
        assert!(journal.entries.is_empty());
    }

    #[test]
    fn test_append_and_read_entries() {
        let temp_dir = TempDir::new().unwrap();
        let work_dir = temp_dir.path();

        let stage_id = "test-stage";
        create_journal(work_dir, stage_id).unwrap();

        let entry1 = MemoryEntry::new(MemoryEntryType::Note, "Found important pattern".to_string());
        append_entry(work_dir, stage_id, &entry1).unwrap();

        let entry2 = MemoryEntry::with_context(
            MemoryEntryType::Decision,
            "Use builder pattern for config".to_string(),
            "Provides better API ergonomics".to_string(),
        );
        append_entry(work_dir, stage_id, &entry2).unwrap();

        let entry3 = MemoryEntry::new(
            MemoryEntryType::Question,
            "Should we cache results?".to_string(),
        );
        append_entry(work_dir, stage_id, &entry3).unwrap();

        let journal = read_journal(work_dir, stage_id).unwrap();
        assert_eq!(journal.entries.len(), 3);

        assert_eq!(journal.entries[0].entry_type, MemoryEntryType::Note);
        assert!(journal.entries[0].content.contains("important pattern"));

        assert_eq!(journal.entries[1].entry_type, MemoryEntryType::Decision);
        assert!(journal.entries[1].context.is_some());

        assert_eq!(journal.entries[2].entry_type, MemoryEntryType::Question);
    }

    #[test]
    fn test_format_memory_for_signal() {
        let temp_dir = TempDir::new().unwrap();
        let work_dir = temp_dir.path();

        let stage_id = "test-stage";
        create_journal(work_dir, stage_id).unwrap();

        let entry1 = MemoryEntry::new(MemoryEntryType::Note, "Note 1".to_string());
        let entry2 = MemoryEntry::new(MemoryEntryType::Decision, "Decision 1".to_string());
        let entry3 = MemoryEntry::new(MemoryEntryType::Question, "Question 1".to_string());

        append_entry(work_dir, stage_id, &entry1).unwrap();
        append_entry(work_dir, stage_id, &entry2).unwrap();
        append_entry(work_dir, stage_id, &entry3).unwrap();

        let signal = format_memory_for_signal(work_dir, stage_id, 10).unwrap();
        assert!(signal.contains("### Notes"));
        assert!(signal.contains("Note 1"));
        assert!(signal.contains("### Decisions"));
        assert!(signal.contains("Decision 1"));
        assert!(signal.contains("### Open Questions"));
        assert!(signal.contains("Question 1"));
    }

    #[test]
    fn test_query_entries() {
        let temp_dir = TempDir::new().unwrap();
        let work_dir = temp_dir.path();

        let stage_id = "test-stage";
        create_journal(work_dir, stage_id).unwrap();

        append_entry(
            work_dir,
            stage_id,
            &MemoryEntry::new(MemoryEntryType::Note, "Authentication flow".to_string()),
        )
        .unwrap();
        append_entry(
            work_dir,
            stage_id,
            &MemoryEntry::new(MemoryEntryType::Note, "Database schema".to_string()),
        )
        .unwrap();
        append_entry(
            work_dir,
            stage_id,
            &MemoryEntry::new(MemoryEntryType::Decision, "Use JWT for auth".to_string()),
        )
        .unwrap();

        let journal = read_journal(work_dir, stage_id).unwrap();

        let results = query_entries(&journal, "auth");
        assert_eq!(results.len(), 2);

        let results = query_entries(&journal, "database");
        assert_eq!(results.len(), 1);

        let results = query_entries(&journal, "nonexistent");
        assert!(results.is_empty());
    }

    #[test]
    fn test_generate_summary() {
        let journal = MemoryJournal {
            stage_id: "test-stage".to_string(),
            entries: vec![
                MemoryEntry::new(MemoryEntryType::Note, "Note 1".to_string()),
                MemoryEntry::new(MemoryEntryType::Note, "Note 2".to_string()),
                MemoryEntry::new(MemoryEntryType::Decision, "Decision 1".to_string()),
                MemoryEntry::new(MemoryEntryType::Question, "Question 1".to_string()),
            ],
            summary: None,
        };

        let summary = generate_summary(&journal, 5);
        assert!(summary.contains("## Summary"));
        assert!(summary.contains("**Total entries**: 4"));
        assert!(summary.contains("**Notes**: 2"));
        assert!(summary.contains("**Decisions**: 1"));
        assert!(summary.contains("**Questions**: 1"));
        assert!(summary.contains("### Key Decisions"));
        assert!(summary.contains("### Open Questions"));
    }

    #[test]
    fn test_format_memory_for_handoff() {
        let temp_dir = TempDir::new().unwrap();
        let work_dir = temp_dir.path();

        let stage_id = "test-stage";
        create_journal(work_dir, stage_id).unwrap();

        append_entry(
            work_dir,
            stage_id,
            &MemoryEntry::new(MemoryEntryType::Note, "Important note".to_string()),
        )
        .unwrap();
        append_entry(
            work_dir,
            stage_id,
            &MemoryEntry::with_context(
                MemoryEntryType::Decision,
                "Key decision".to_string(),
                "Good rationale".to_string(),
            ),
        )
        .unwrap();

        let handoff = format_memory_for_handoff(work_dir, stage_id).unwrap();
        assert!(handoff.contains("## Stage Memory"));
        assert!(handoff.contains("### Decisions Made"));
        assert!(handoff.contains("Key decision"));
        assert!(handoff.contains("Good rationale"));
        assert!(handoff.contains("### Recent Notes"));
        assert!(handoff.contains("Important note"));
    }

    #[test]
    fn test_preserve_for_crash() {
        let temp_dir = TempDir::new().unwrap();
        let work_dir = temp_dir.path();

        let stage_id = "test-stage";
        create_journal(work_dir, stage_id).unwrap();
        append_entry(
            work_dir,
            stage_id,
            &MemoryEntry::new(MemoryEntryType::Note, "Important work".to_string()),
        )
        .unwrap();

        let preserved = preserve_for_crash(work_dir, stage_id).unwrap().unwrap();
        assert!(preserved.exists());
        assert!(preserved.to_string_lossy().contains("memory-test-stage.md"));

        let content = std::fs::read_to_string(&preserved).unwrap();
        assert!(content.contains("Important work"));
    }

    #[test]
    fn test_extract_key_notes() {
        let journal = MemoryJournal {
            stage_id: "test-stage".to_string(),
            entries: vec![
                MemoryEntry::new(MemoryEntryType::Note, "Just a note".to_string()),
                MemoryEntry::with_context(
                    MemoryEntryType::Decision,
                    "Use pattern X".to_string(),
                    "Because Y".to_string(),
                ),
                MemoryEntry::new(MemoryEntryType::Decision, "Another decision".to_string()),
            ],
            summary: None,
        };

        let key_notes = extract_key_notes(&journal);
        assert_eq!(key_notes.len(), 2);
        assert!(key_notes[0].contains("Use pattern X"));
        assert!(key_notes[0].contains("Because Y"));
        assert!(key_notes[1].contains("Another decision"));
    }

    #[test]
    fn test_validate_content() {
        assert!(validate_content("Valid content").is_ok());
        assert!(validate_content("").is_err());
        assert!(validate_content(&"a".repeat(2001)).is_err());
    }

    #[test]
    fn test_list_journals() {
        let temp_dir = TempDir::new().unwrap();
        let work_dir = temp_dir.path();

        create_journal(work_dir, "stage-1").unwrap();
        create_journal(work_dir, "stage-2").unwrap();

        let journals = list_journals(work_dir).unwrap();
        assert_eq!(journals.len(), 2);
        assert!(journals.contains(&"stage-1".to_string()));
        assert!(journals.contains(&"stage-2".to_string()));
    }

    #[test]
    fn an_entry_round_trips_its_id_full_timestamp_session_evidence_and_receipt() {
        let temp = TempDir::new().unwrap();
        let receipt = Receipt {
            event_id: "1".repeat(32),
            outcome: ReceiptOutcome::Promoted,
            target: Some("architecture/context-retrieval.md#required-items".to_string()),
        };
        let mut entry = MemoryEntry::receipt(receipt, "captured in knowledge".to_string())
            .with_evidence(vec![
                "src/lib.rs:12".to_string(),
                "important_symbol".to_string(),
            ]);
        entry.id = "a".repeat(32);
        entry.timestamp = Utc.with_ymd_and_hms(2026, 9, 10, 14, 3, 22).unwrap();
        entry.session = Some("session-42".to_string());

        append_entry(temp.path(), "round-trip", &entry).unwrap();
        let parsed = read_journal(temp.path(), "round-trip").unwrap();

        assert_eq!(parsed.entries, vec![entry]);
    }

    #[test]
    fn a_journal_read_the_next_day_keeps_yesterdays_date() {
        let temp = TempDir::new().unwrap();
        let mut entry = MemoryEntry::new(MemoryEntryType::Note, "yesterday".to_string());
        entry.timestamp = Utc.with_ymd_and_hms(2026, 9, 9, 23, 59, 58).unwrap();
        append_entry(temp.path(), "multi-day", &entry).unwrap();

        let parsed = read_journal(temp.path(), "multi-day").unwrap();

        assert_eq!(
            parsed.entries[0].timestamp.date_naive(),
            entry.timestamp.date_naive()
        );
    }

    #[test]
    fn two_concurrent_appends_never_interleave() {
        let temp = Arc::new(TempDir::new().unwrap());
        let threads: Vec<_> = (0..2)
            .map(|thread| {
                let temp = Arc::clone(&temp);
                std::thread::spawn(move || {
                    for index in 0..50 {
                        let content = format!("{thread}:{index}:{}", "x".repeat(3000));
                        let entry = MemoryEntry::new(MemoryEntryType::Note, content);
                        append_entry(temp.path(), "concurrent", &entry).unwrap();
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }

        let journal = read_journal(temp.path(), "concurrent").unwrap();
        assert_eq!(journal.entries.len(), 100);
    }

    #[test]
    fn receipts_are_excluded_from_signal_and_handoff_exports() {
        let temp = TempDir::new().unwrap();
        let note = MemoryEntry::new(MemoryEntryType::Note, "keep me".to_string());
        let receipt = MemoryEntry::receipt(
            Receipt {
                event_id: note.id.clone(),
                outcome: ReceiptOutcome::Discarded,
                target: None,
            },
            "hide me".to_string(),
        );
        append_entry(temp.path(), "exports", &note).unwrap();
        append_entry(temp.path(), "exports", &receipt).unwrap();

        let signal = format_memory_for_signal(temp.path(), "exports", 10).unwrap();
        let handoff = format_memory_for_handoff(temp.path(), "exports").unwrap();

        assert!(signal.contains("keep me") && !signal.contains("hide me"));
        assert!(handoff.contains("keep me") && !handoff.contains("hide me"));
    }
}
