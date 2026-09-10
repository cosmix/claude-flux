//! Recording processing receipts for captured memory events.

use anyhow::{bail, Result};

use crate::fs::memory::{MemoryEntry, MemoryEntryType, Receipt, ReceiptOutcome};

use super::pending::staged_entries;
use super::record::record;
use super::work_dir::readonly_work_dir;

/// Settle a memory event by recording a receipt through the normal write path.
pub fn resolve(
    event_id: String,
    outcome: String,
    target: Option<String>,
    reason: Option<String>,
    stage_id: Option<String>,
) -> Result<()> {
    ensure_event_exists(&event_id)?;
    let outcome: ReceiptOutcome = outcome.parse()?;
    let (target, reason) = resolution_details(outcome, target, reason)?;
    let entry = MemoryEntry::receipt(
        Receipt {
            event_id: event_id.clone(),
            outcome,
            target,
        },
        reason,
    );
    let receipt_id = entry.id.clone();

    record(entry, stage_id)?;
    println!("✓ Resolved {event_id} ({outcome}) as receipt {receipt_id}");
    Ok(())
}

fn ensure_event_exists(event_id: &str) -> Result<()> {
    let Some(work_dir) = readonly_work_dir() else {
        bail!("Unknown memory event id: {event_id}");
    };
    let (entries, _) = staged_entries(&work_dir)?;
    if entries.iter().any(|staged| {
        staged.entry.id == event_id && staged.entry.entry_type != MemoryEntryType::Receipt
    }) {
        return Ok(());
    }
    bail!("Unknown memory event id: {event_id}")
}

fn resolution_details(
    outcome: ReceiptOutcome,
    target: Option<String>,
    reason: Option<String>,
) -> Result<(Option<String>, String)> {
    match outcome {
        ReceiptOutcome::Promoted | ReceiptOutcome::Merged => {
            let target = target
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| anyhow::anyhow!("--target is required for outcome {outcome}"))?;
            let reason = reason.unwrap_or_else(|| format!("{outcome} into {target}"));
            Ok((Some(target), reason))
        }
        ReceiptOutcome::Discarded | ReceiptOutcome::Deferred => {
            let reason = reason
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| anyhow::anyhow!("--reason is required for outcome {outcome}"))?;
            Ok((target, reason))
        }
    }
}
