//! Knowledge map operations for curated codebase knowledge.
//!
//! Design principle: Claude Code already has Glob, Grep, Read, LSP tools.
//! We curate high-level knowledge that helps agents know WHERE to look,
//! not raw indexing.
//!
//! Knowledge is tiered: a generated `INDEX.md` (tier 0) points at the seven
//! curated summary files (tier 1), which link to per-category topic files
//! (tier 2, e.g. `architecture/merge-flow.md`). Directories created before the
//! hierarchy existed stay flat and keep working — see [`types::KnowledgeLayout`].

pub mod catalog;
pub mod chunker;
pub mod dir;
pub(crate) mod frontmatter;
pub mod index;
mod scaffold;
pub mod splice;
pub mod templates;
pub mod types;

#[cfg(test)]
mod tests;

// Re-export commonly used types
pub use dir::KnowledgeDir;
pub use index::TopicEntry;
pub use splice::SectionOutcome;
pub use types::{KnowledgeFile, KnowledgeLayout, KnowledgeTarget, INDEX_FILENAME};
