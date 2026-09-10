//! Knowledge directory manager.

use super::index::{self, TopicEntry};
use super::scaffold;
use super::splice::{self, SectionOutcome};
use super::templates;
use super::types::{KnowledgeFile, KnowledgeLayout, KnowledgeTarget, INDEX_FILENAME};
use anyhow::{Context, Result};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Manager for the doc/loom/knowledge/ directory
pub struct KnowledgeDir {
    root: PathBuf,
}

impl KnowledgeDir {
    /// Create a new KnowledgeDir from the project root directory
    pub fn new<P: AsRef<Path>>(project_root: P) -> Self {
        Self {
            root: project_root.as_ref().join("doc/loom/knowledge"),
        }
    }

    /// Wrap an existing knowledge directory (the path [`KnowledgeDir::root`]
    /// returns), rather than deriving it from a project root. Callers that have
    /// already resolved the tree — `loom knowledge sync`, retrieval — must not
    /// have to re-derive `doc/loom/knowledge` and risk disagreeing about it.
    pub fn from_root<P: Into<PathBuf>>(knowledge_root: P) -> Self {
        Self {
            root: knowledge_root.into(),
        }
    }

    /// Get the knowledge directory path
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Check if the knowledge directory exists
    pub fn exists(&self) -> bool {
        self.root.exists()
    }

    /// Check if the knowledge directory has any meaningful content
    ///
    /// Returns true if at least one knowledge file exists and has content
    /// beyond the default placeholder text.
    pub fn has_content(&self) -> bool {
        if !self.exists() {
            return false;
        }

        for file_type in KnowledgeFile::all() {
            let path = self.file_path(*file_type);
            if path.exists() {
                if let Ok(content) = fs::read_to_string(&path) {
                    // Check if content has more than just the default template
                    // by looking for ## headers added by agents
                    if content.lines().any(|line| {
                        line.starts_with("## ")
                            && !line.contains("(Add ")
                            && !line.contains("append-only")
                    }) {
                        return true;
                    }
                }
            }
        }

        // A tier-2 topic file always counts as content, even if every tier-1
        // file is still at its default scaffold.
        matches!(self.list_topics(), Ok(topics) if !topics.is_empty())
    }

    /// Initialize the knowledge directory with default files.
    ///
    /// A directory created here starts **hierarchical**: `INDEX.md` is written
    /// so new projects get the tiered layout from `loom init` onward. An
    /// existing directory is never given an index — a flat knowledge dir that
    /// predates the hierarchy stays flat until the user opts in with `loom
    /// knowledge sync`.
    pub fn initialize(&self) -> Result<()> {
        let fresh = !self.root.exists();
        if fresh {
            fs::create_dir_all(&self.root).context("Failed to create knowledge directory")?;
        }

        // create_new atomically fails if file exists, preventing TOCTOU race
        for file_type in KnowledgeFile::all() {
            let path = self.file_path(*file_type);
            let content = templates::default_content(*file_type);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    file.write_all(content.as_bytes())
                        .with_context(|| format!("Failed to write {}", file_type.filename()))?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // File already exists, skip (idempotent)
                }
                Err(e) => {
                    return Err(anyhow::anyhow!(e))
                        .with_context(|| format!("Failed to create {}", file_type.filename()));
                }
            }
        }

        if fresh {
            index::write_index(&self.root)?;
        }

        Ok(())
    }

    /// Get the path to a specific knowledge file
    pub fn file_path(&self, file_type: KnowledgeFile) -> PathBuf {
        self.root.join(file_type.filename())
    }

    /// Read a knowledge file
    pub fn read(&self, file_type: KnowledgeFile) -> Result<String> {
        let path = self.file_path(file_type);
        fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", file_type.filename()))
    }

    /// Append content to a knowledge file (knowledge files are append-only)
    pub fn append(&self, file_type: KnowledgeFile, content: &str) -> Result<()> {
        self.append_target(&KnowledgeTarget::Tier1(file_type), content)
    }

    /// Replace a section in a knowledge file identified by its heading, at
    /// whatever ATX level (`##` through `######`) it is written.
    ///
    /// Finds the first heading line matching `heading` and replaces everything
    /// between it and the next heading of the same or shallower level (or EOF)
    /// with the new content. If no heading matches, appends a new `##
    /// <heading>` section instead — see [`SectionOutcome`].
    pub fn replace_section(
        &self,
        file_type: KnowledgeFile,
        heading: &str,
        content: &str,
    ) -> Result<SectionOutcome> {
        self.replace_section_target(&KnowledgeTarget::Tier1(file_type), heading, content)
    }

    /// Layout of this knowledge directory: `Hierarchical` iff `INDEX.md` exists.
    pub fn layout(&self) -> KnowledgeLayout {
        if self.index_path().exists() {
            KnowledgeLayout::Hierarchical
        } else {
            KnowledgeLayout::Legacy
        }
    }

    /// Path to the generated `INDEX.md`.
    pub fn index_path(&self) -> PathBuf {
        self.root.join(INDEX_FILENAME)
    }

    /// Read the generated `INDEX.md`.
    pub fn read_index(&self) -> Result<String> {
        fs::read_to_string(self.index_path())
            .with_context(|| format!("Failed to read {INDEX_FILENAME}"))
    }

    /// Regenerate and crash-atomically write `INDEX.md`.
    pub fn write_index(&self) -> Result<()> {
        index::write_index(&self.root)
    }

    /// Path of `target` relative to the project root.
    pub fn target_path(&self, target: &KnowledgeTarget) -> PathBuf {
        self.root.join(target.relative_path())
    }

    /// Read a tier-1 or tier-2 knowledge target.
    pub fn read_target(&self, target: &KnowledgeTarget) -> Result<String> {
        let path = self.target_path(target);
        fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", target.display_name()))
    }

    /// Append content to a tier-1 file or tier-2 topic (both are append-only).
    ///
    /// A topic file that does not exist yet is scaffolded with a `# Title` /
    /// `> blurb` header derived from its slug before the content is appended
    /// — UNLESS the content already carries its own `# ` title, in which case
    /// it is written verbatim and the generic scaffold is skipped entirely
    /// (tier-1 files always keep their curated template regardless). An
    /// EXISTING topic file whose header is still that generic stub is healed
    /// in place: content carrying its own `# ` title replaces the stub header
    /// instead of leaving both a generic and a real title in the file.
    pub fn append_target(&self, target: &KnowledgeTarget, content: &str) -> Result<()> {
        let path = self.target_path(target);
        let default = templates::default_scaffold(target);
        let content_owned = content.to_string();
        let topic = scaffold::topic_category_and_slug(target);

        crate::fs::locking::locked_read_modify_write(&path, |existing| {
            if existing.is_empty() {
                return scaffold::new_topic_content(&default, &content_owned, topic.is_some());
            }
            if let Some((category, slug)) = topic {
                if scaffold::has_own_title(&content_owned) {
                    if let Some(rest) = scaffold::strip_stub_header(&existing, category, slug) {
                        return scaffold::heal_stub_header(&content_owned, &rest);
                    }
                }
            }
            scaffold::append_below(&existing, &content_owned)
        })
        .with_context(|| format!("Failed to append to {}", target.display_name()))?;

        self.refresh_index_if_hierarchical();
        Ok(())
    }

    /// Replace a `#{2,6} <heading>` section in a tier-1 file or tier-2 topic,
    /// at whatever level the heading is actually written, appending a new
    /// `## <heading>` section if no heading matches. See
    /// [`Self::replace_section`] for the exact splicing rules and
    /// [`SectionOutcome`] for what gets reported back.
    ///
    /// A tier-2 topic that does not exist yet is still seeded with
    /// `templates::default_scaffold`'s generic `# Title` / stub blurb here —
    /// unlike [`Self::append_target`]'s BUG-3 fix, there is no caller-supplied
    /// title to prefer, since a section `content` is a body, not a whole file.
    pub fn replace_section_target(
        &self,
        target: &KnowledgeTarget,
        heading: &str,
        content: &str,
    ) -> Result<SectionOutcome> {
        let path = self.target_path(target);
        let default = templates::default_scaffold(target);
        let heading_owned = heading.to_string();
        let content_owned = content.to_string();
        let mut outcome = None;

        crate::fs::locking::locked_read_modify_write(&path, |existing| {
            let base = if existing.is_empty() {
                default
            } else {
                existing
            };
            let (result, found) = splice::splice_section(base, &heading_owned, &content_owned);
            outcome = Some(found);
            result
        })
        .with_context(|| {
            format!(
                "Failed to replace section '{heading}' in {}",
                target.display_name()
            )
        })?;

        self.refresh_index_if_hierarchical();
        Ok(outcome.expect("locked_read_modify_write always invokes its closure"))
    }

    /// List every tier-2 topic file under this knowledge directory.
    pub fn list_topics(&self) -> Result<Vec<TopicEntry>> {
        index::scan_topics(&self.root)
    }

    /// Regenerate `INDEX.md` when the directory is already hierarchical.
    ///
    /// Called AFTER `locked_read_modify_write` has returned and released its
    /// lock — never from inside that closure or nested within the locked
    /// call. `write_index` takes the same parent-directory lock a tier-1
    /// write just held (`INDEX.md` and the tier-1 files share `root`), so
    /// calling it while that lock is still held would deadlock: `flock` is
    /// per-open-file-description, so a second exclusive lock request on the
    /// same directory from this thread blocks forever waiting on the first,
    /// which never releases because it's waiting on us.
    ///
    /// A refresh failure is reported but NOT propagated: the content write has
    /// already committed, so returning an error here would make a successful
    /// `loom knowledge update` exit non-zero, and an agent's natural response —
    /// re-running the command — would append the same block twice. The index is
    /// derived state; the next knowledge write, or `loom knowledge sync`,
    /// rebuilds it.
    pub(crate) fn refresh_index_if_hierarchical(&self) {
        if self.layout() == KnowledgeLayout::Hierarchical {
            if let Err(e) = self.write_index() {
                eprintln!("warning: failed to refresh {INDEX_FILENAME}: {e:#}");
                eprintln!("         the content was written; the next knowledge write, or `loom knowledge sync`, rebuilds the index");
            }
        }
    }
}

#[cfg(test)]
#[path = "tests_dir.rs"]
mod tests;

#[cfg(test)]
#[path = "tests_dir_bugfixes.rs"]
mod tests_bugfixes;
