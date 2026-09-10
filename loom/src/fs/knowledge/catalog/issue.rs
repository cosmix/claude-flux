use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A problem found in the knowledge base. Reported, never repaired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CatalogIssue {
    DuplicateHeading {
        file: PathBuf,
        heading: String,
        occurrences: usize,
    },
    GenericBlurb {
        file: PathBuf,
        blurb: String,
    },
    BrokenLink {
        file: PathBuf,
        target: String,
    },
    MissingSourceRef {
        file: PathBuf,
        source_path: String,
    },
    EvidenceChanged {
        file: PathBuf,
        source_path: String,
        verified: String,
    },
    UnverifiableReference {
        file: PathBuf,
        source_path: String,
        kind: String,
    },
    OversizedSection {
        file: PathBuf,
        heading: String,
        lines: usize,
    },
    OversizedFile {
        file: PathBuf,
        lines: usize,
    },
    OversizedIndex {
        bytes: u64,
    },
}

impl CatalogIssue {
    /// Review prompts and explanatory notes do not make `check --strict` fail.
    pub fn is_review_only(&self) -> bool {
        matches!(
            self,
            Self::EvidenceChanged { .. } | Self::UnverifiableReference { .. }
        )
    }
}
