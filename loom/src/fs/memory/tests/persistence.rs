use super::*;
use crate::fs::memory::parser::{format_entry, parse_journal};
use crate::fs::memory::types::{MemoryEntry, MemoryEntryType};
use chrono::{TimeZone, Utc};

#[test]
fn evidence_with_a_backtick_is_rejected_before_it_can_orphan_an_entry() {
    let err = validate_evidence(&["`MemoryEntry`".to_string()]).unwrap_err();
    assert!(err.to_string().contains('`'));

    let mut entry = MemoryEntry::new(MemoryEntryType::Note, "safe entry".to_string())
        .with_evidence(vec!["src/lib.rs:12".to_string()]);
    entry.timestamp = Utc.with_ymd_and_hms(2026, 9, 10, 14, 3, 22).unwrap();
    validate_evidence(&entry.evidence).unwrap();

    let rendered = format_entry(&entry);
    let journal = parse_journal(&rendered, "round-trip").unwrap();
    assert_eq!(journal.entries, vec![entry]);
}
