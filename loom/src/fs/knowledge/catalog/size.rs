//! These are the mechanical form of CLAUDE.md Rule 12's tier-1 size discipline,
//! reported and never repaired.

use super::CatalogIssue;
use crate::fs::knowledge::chunker::KnowledgeChunk;
use crate::fs::knowledge::types::INDEX_FILENAME;
use std::fs;
use std::path::Path;

/// Maximum tier-1 section lines. CLAUDE.md Rule 12's spill threshold is the
/// point at which a tier-1 section is supposed to move to a tier-2 topic file.
const MAX_TIER_ONE_SECTION_LINES: usize = 40;
/// Maximum line count for a tier-1 summary file.
const MAX_TIER_ONE_FILE_LINES: usize = 250;
/// Maximum byte size for the generated tier-0 index. The index is read in
/// full at the start of every session, so this is the byte budget for
/// roughly 3k tokens — the ceiling we accept for that first read.
pub(crate) const MAX_INDEX_BYTES: u64 = 12_288;

/// Flag a tier-1 section whose line count (its `## ` heading line included,
/// trailing blank lines excluded) exceeds `MAX_TIER_ONE_SECTION_LINES`. A
/// no-op for tier-2 topic files and for the file's headingless preamble.
pub(super) fn oversized_section(
    relative_path: &Path,
    chunk: &KnowledgeChunk,
) -> Option<CatalogIssue> {
    let lines = chunk.body.lines().count();
    (is_tier_one(relative_path) && !chunk.heading.is_empty() && lines > MAX_TIER_ONE_SECTION_LINES)
        .then(|| CatalogIssue::OversizedSection {
            file: relative_path.to_path_buf(),
            heading: chunk.heading.clone(),
            lines,
        })
}

/// Flag a tier-1 file whose total line count exceeds
/// `MAX_TIER_ONE_FILE_LINES`. A no-op for tier-2 topic files.
pub(super) fn oversized_file(relative_path: &Path, content: &str) -> Option<CatalogIssue> {
    let lines = content.lines().count();
    (is_tier_one(relative_path) && lines > MAX_TIER_ONE_FILE_LINES).then(|| {
        CatalogIssue::OversizedFile {
            file: relative_path.to_path_buf(),
            lines,
        }
    })
}

/// Flag a generated `INDEX.md` whose byte size exceeds `MAX_INDEX_BYTES`.
/// `None` when the index has not been generated yet.
pub(super) fn oversized_index(root: &Path) -> Option<CatalogIssue> {
    let bytes = fs::metadata(root.join(INDEX_FILENAME)).ok()?.len();
    (bytes > MAX_INDEX_BYTES).then_some(CatalogIssue::OversizedIndex { bytes })
}

fn is_tier_one(relative_path: &Path) -> bool {
    relative_path.components().count() == 1
}
