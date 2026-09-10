//! Type definitions for memory journal entries.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Entry type in the memory journal
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryEntryType {
    /// General observations and context
    Note,
    /// Choices made with rationale
    Decision,
    /// Open questions for future investigation
    Question,
    /// File changes made during implementation
    Change,
    /// Processing outcome for a previously captured memory event
    Receipt,
}

impl MemoryEntryType {
    /// Get a display name for this entry type
    pub fn display_name(&self) -> &'static str {
        match self {
            MemoryEntryType::Note => "Note",
            MemoryEntryType::Decision => "Decision",
            MemoryEntryType::Question => "Question",
            MemoryEntryType::Change => "Change",
            MemoryEntryType::Receipt => "Receipt",
        }
    }

    /// Get the emoji used for this entry type.
    pub fn emoji(&self) -> &'static str {
        match self {
            MemoryEntryType::Note => "📝",
            MemoryEntryType::Decision => "✅",
            MemoryEntryType::Question => "❓",
            MemoryEntryType::Change => "🔧",
            MemoryEntryType::Receipt => "🧾",
        }
    }

    /// Get all entry types
    pub fn all() -> &'static [MemoryEntryType] {
        &[
            MemoryEntryType::Note,
            MemoryEntryType::Decision,
            MemoryEntryType::Question,
            MemoryEntryType::Change,
            MemoryEntryType::Receipt,
        ]
    }
}

impl std::fmt::Display for MemoryEntryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryEntryType::Note => write!(f, "note"),
            MemoryEntryType::Decision => write!(f, "decision"),
            MemoryEntryType::Question => write!(f, "question"),
            MemoryEntryType::Change => write!(f, "change"),
            MemoryEntryType::Receipt => write!(f, "receipt"),
        }
    }
}

impl std::str::FromStr for MemoryEntryType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "note" | "notes" => Ok(MemoryEntryType::Note),
            "decision" | "decisions" => Ok(MemoryEntryType::Decision),
            "question" | "questions" => Ok(MemoryEntryType::Question),
            "change" | "changes" => Ok(MemoryEntryType::Change),
            "receipt" | "receipts" => Ok(MemoryEntryType::Receipt),
            _ => anyhow::bail!(
                "Invalid entry type: {s}. Use: note, decision, question, change, receipt"
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReceiptOutcome {
    Promoted,
    Merged,
    Discarded,
    Deferred,
}

impl std::fmt::Display for ReceiptOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let outcome = match self {
            ReceiptOutcome::Promoted => "promoted",
            ReceiptOutcome::Merged => "merged",
            ReceiptOutcome::Discarded => "discarded",
            ReceiptOutcome::Deferred => "deferred",
        };
        f.write_str(outcome)
    }
}

impl std::str::FromStr for ReceiptOutcome {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "promoted" => Ok(Self::Promoted),
            "merged" => Ok(Self::Merged),
            "discarded" => Ok(Self::Discarded),
            "deferred" => Ok(Self::Deferred),
            _ => anyhow::bail!("Invalid receipt outcome: {s}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    /// The `MemoryEntry::id` this receipt settles.
    pub event_id: String,
    pub outcome: ReceiptOutcome,
    /// Knowledge target the event was promoted or merged into (`architecture/context-retrieval.md#required-items`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// A single memory entry in the journal
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// Assigned once at capture: `uuid::Uuid::new_v4().simple()` (32 lowercase hex chars).
    pub id: String,
    /// When the entry was recorded
    pub timestamp: DateTime<Utc>,
    /// Type of entry
    pub entry_type: MemoryEntryType,
    /// The content of the entry
    pub content: String,
    /// Optional additional context or rationale (for decisions)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// `LOOM_SESSION_ID` at capture, when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Paths, `path:line` spans, or symbols the entry rests on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    /// Present exactly when `entry_type == Receipt`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Receipt>,
}

impl MemoryEntry {
    /// Create a new memory entry
    pub fn new(entry_type: MemoryEntryType, content: String) -> Self {
        Self {
            id: Uuid::new_v4().simple().to_string(),
            timestamp: Utc::now(),
            entry_type,
            content,
            context: None,
            session: std::env::var("LOOM_SESSION_ID").ok(),
            evidence: Vec::new(),
            receipt: None,
        }
    }

    /// Create a new memory entry with context
    pub fn with_context(entry_type: MemoryEntryType, content: String, context: String) -> Self {
        Self::new(entry_type, content).with_context_value(context)
    }

    /// Create an entry recording how an earlier memory event was handled.
    pub fn receipt(receipt: Receipt, reason: String) -> Self {
        let mut entry = Self::new(MemoryEntryType::Receipt, reason);
        entry.receipt = Some(receipt);
        entry
    }

    /// Attach evidence references to this entry.
    pub fn with_evidence(mut self, evidence: Vec<String>) -> Self {
        self.evidence = evidence;
        self
    }

    fn with_context_value(mut self, context: String) -> Self {
        self.context = Some(context);
        self
    }
}

/// Memory journal for a session
#[derive(Debug, Clone, Default)]
pub struct MemoryJournal {
    /// Stage ID this journal belongs to
    pub stage_id: String,
    /// All entries in the journal
    pub entries: Vec<MemoryEntry>,
    /// Summary of the journal (generated at context threshold)
    pub summary: Option<String>,
}

/// Builder for parsing memory entries
pub(crate) struct EntryBuilder {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub entry_type: MemoryEntryType,
    pub content: String,
    pub context: Option<String>,
    pub session: Option<String>,
    pub evidence: Vec<String>,
    pub receipt: Option<Receipt>,
    pub in_metadata: bool,
    pub valid: bool,
}

impl EntryBuilder {
    pub fn build(self) -> Option<MemoryEntry> {
        let receipt_is_valid =
            (self.entry_type == MemoryEntryType::Receipt) == self.receipt.is_some();
        if self.content.trim().is_empty() || !self.valid || !receipt_is_valid {
            return None;
        }

        Some(MemoryEntry {
            id: self.id,
            timestamp: self.timestamp,
            entry_type: self.entry_type,
            content: self.content.trim().to_string(),
            context: self.context,
            session: self.session,
            evidence: self.evidence,
            receipt: self.receipt,
        })
    }
}
