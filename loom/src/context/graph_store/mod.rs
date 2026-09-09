//! Layered persistence for the derived source graph.
//!
//! ## Why layers exist
//!
//! Parallel stages run in separate worktrees off one repository. If they shared
//! a mutable graph, a stage would see half of a sibling's edits — worse than
//! seeing none, because there is no way to tell which half. So the graph is
//! split in two:
//!
//! - a **base layer**, keyed by the source revision it was built from, written
//!   once by the host and thereafter immutable. `.loom/cache/context-v1/graph/base/`
//!   under the canonical main project root, shared by every worktree;
//! - a per-stage **overlay**, under `.loom/work/context/<plan>/<stage>/`, holding
//!   only the files that stage changed.
//!
//! A read is `overlay ∪ (base − overlay's files)`: an overlay entry shadows the
//! base entry for the same path wholesale, never merges with it. Partial merges
//! are what produce a graph that describes no revision that ever existed.
//!
//! Nothing here builds a graph — `crate::context::refresh` does that — and
//! nothing here decides *when* to write one; that is
//! `crate::context::refresh::source_graph::reconcile_source_graph`'s job. This
//! module owns only the layout, the layering rule, and canonical serialization.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::context::source_graph::{FileCoverage, SourceEdge, SourceNode};
use crate::context::store::canonical_json;

/// Graph directory, relative to the context cache root.
pub const GRAPH_RELATIVE_DIR: &str = "graph";
/// Immutable per-revision base layers, relative to [`GRAPH_RELATIVE_DIR`].
pub const BASE_RELATIVE_DIR: &str = "base";
/// Overlay root inside `.loom/work/`.
pub const OVERLAY_RELATIVE_DIR: &str = "context";
/// File name of a persisted layer.
pub const LAYER_FILE: &str = "graph.json";

/// One file's contribution to a layer.
///
/// Stored per-file rather than as two flat lists so an overlay can shadow
/// exactly the files a stage touched, and so a single changed file can be
/// re-extracted without rebuilding the layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    /// `sha256:<hex>` over the file's exact bytes, for incremental rebuilds.
    pub content_hash: String,
    #[serde(default)]
    pub nodes: Vec<SourceNode>,
    #[serde(default)]
    pub edges: Vec<SourceEdge>,
    pub coverage: FileCoverage,
}

impl Default for FileEntry {
    /// An entry nothing has been extracted into yet. The coverage says so
    /// rather than claiming `Full`, so a half-built layer can never read as
    /// complete.
    fn default() -> Self {
        FileEntry {
            content_hash: String::new(),
            nodes: Vec::new(),
            edges: Vec::new(),
            coverage: FileCoverage::LexicalOnly {
                detail: "not extracted".to_string(),
            },
        }
    }
}

/// One persisted layer: base or overlay.
///
/// `files` is a `BTreeMap` and every collection inside is sorted, so two runs
/// over identical bytes serialize byte-identically (see [`canonical_json`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphLayer {
    /// Source revision this layer describes: a git commit for a base layer,
    /// and the base revision the overlay was cut from for an overlay.
    #[serde(default)]
    pub revision: String,
    /// Snapshot generation identifier; empty means unknown until
    /// `source-graph-snapshot` fills it.
    #[serde(default)]
    pub generation: String,
    /// When this layer was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub built_at: Option<DateTime<Utc>>,
    /// Extraction results keyed by project-relative, forward-slashed path.
    #[serde(default)]
    pub files: BTreeMap<String, FileEntry>,
    /// Path to the git blob object id whose bytes produced its
    /// [`FileEntry::content_hash`].
    #[serde(default)]
    pub blob_index: BTreeMap<String, String>,
}

impl GraphLayer {
    /// Every node in this layer, in path order.
    pub fn nodes(&self) -> impl Iterator<Item = &SourceNode> {
        self.files.values().flat_map(|entry| entry.nodes.iter())
    }

    /// Every edge in this layer, in path order.
    pub fn edges(&self) -> impl Iterator<Item = &SourceEdge> {
        self.files.values().flat_map(|entry| entry.edges.iter())
    }
}

/// A base layer with an overlay applied — the view every reader sees.
///
/// Built by [`GraphStore::resolved`]. Holds owned data because the two layers
/// it draws from have different lifetimes and a reader should not have to care
/// which layer an entry came from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedGraph {
    /// Revision of the base layer underneath, empty when there is none.
    pub base_revision: String,
    /// Paths the overlay shadowed. Non-empty means this view is stage-local.
    pub overlaid: BTreeSet<String>,
    pub files: BTreeMap<String, FileEntry>,
}

impl ResolvedGraph {
    pub fn nodes(&self) -> impl Iterator<Item = &SourceNode> {
        self.files.values().flat_map(|entry| entry.nodes.iter())
    }

    pub fn edges(&self) -> impl Iterator<Item = &SourceEdge> {
        self.files.values().flat_map(|entry| entry.edges.iter())
    }

    /// Look up a node by its [`SourceNode::id`].
    pub fn node(&self, id: &str) -> Option<&SourceNode> {
        self.nodes().find(|node| node.id == id)
    }

    /// Total node count, for coverage reporting.
    pub fn node_count(&self) -> usize {
        self.files.values().map(|entry| entry.nodes.len()).sum()
    }

    /// Total edge count, for coverage reporting.
    pub fn edge_count(&self) -> usize {
        self.files.values().map(|entry| entry.edges.len()).sum()
    }
}

/// Resolves layer paths and reads/writes layers. Holds no graph state itself.
#[derive(Debug, Clone)]
pub struct GraphStore {
    /// `<main project root>/.loom/cache/context-v1/graph`.
    graph_root: PathBuf,
    /// `<.loom/work>/context`.
    overlay_root: PathBuf,
}

impl GraphStore {
    /// Build from the two roots directly. `context_cache_root` is
    /// [`crate::context::store::ContextStore::root`]; `work_root` is
    /// [`crate::fs::work_dir::WorkDir::root`].
    pub fn new(context_cache_root: &Path, work_root: &Path) -> Self {
        GraphStore {
            graph_root: context_cache_root.join(GRAPH_RELATIVE_DIR),
            overlay_root: work_root.join(OVERLAY_RELATIVE_DIR),
        }
    }

    /// Directory holding immutable per-revision base layers.
    pub fn base_dir(&self) -> PathBuf {
        self.graph_root.join(BASE_RELATIVE_DIR)
    }

    /// Path of the base layer for `revision`.
    ///
    /// One file per revision, so a stage that started against an older base
    /// keeps reading a consistent snapshot after the host publishes a newer one.
    pub fn base_path(&self, revision: &str) -> PathBuf {
        self.base_dir().join(format!("{revision}.json"))
    }

    /// Directory of a stage's overlay: `.loom/work/context/<plan>/<stage>/`.
    pub fn overlay_dir(&self, plan: &str, stage: &str) -> PathBuf {
        self.overlay_root.join(plan).join(stage)
    }

    /// Path of a stage's overlay layer.
    pub fn overlay_path(&self, plan: &str, stage: &str) -> PathBuf {
        self.overlay_dir(plan, stage).join(LAYER_FILE)
    }

    /// Read the base layer for `revision`, or `None` when it was never built.
    pub fn load_base(&self, revision: &str) -> Result<Option<GraphLayer>> {
        read_layer(&self.base_path(revision))
    }

    /// Publish a base layer for `revision`.
    ///
    /// A published base layer is immutable: if one already exists for this
    /// revision, this is a no-op returning `false`, because two builds of the
    /// same revision must agree and rewriting would invalidate every overlay
    /// cut from it — and, deliberately, that early-return path never prunes:
    /// a rapid sequence of `publish_base` calls for an already-published
    /// revision must not each pay a GC pass for nothing.
    ///
    /// A NEWLY written base, on the other hand, triggers
    /// `Self::prune_after_publish` (`graph_store/prune.rs`) so publishing
    /// is the one place base-layer retention is enforced (see
    /// `doc/PROPOSAL-retrieval-precision.md` §A.14) — `graph/base/`
    /// otherwise accretes one file per published commit forever.
    pub fn publish_base(&self, revision: &str, layer: &GraphLayer) -> Result<bool> {
        let path = self.base_path(revision);
        if path.exists() {
            return Ok(false);
        }
        write_layer(&path, layer)?;
        self.prune_after_publish(revision);
        Ok(true)
    }

    /// Read a stage's overlay, or `None` when it has none.
    pub fn load_overlay(&self, plan: &str, stage: &str) -> Result<Option<GraphLayer>> {
        read_layer(&self.overlay_path(plan, stage))
    }

    /// Write a stage's overlay, replacing any previous one.
    ///
    /// Overlays are mutable — a stage rewrites its own as it edits — but they
    /// are private to that stage, so no other reader can observe a torn write.
    pub fn save_overlay(&self, plan: &str, stage: &str, layer: &GraphLayer) -> Result<()> {
        write_layer(&self.overlay_path(plan, stage), layer)
    }

    /// Delete a stage's overlay layer file. Idempotent.
    ///
    /// Called after the stage's work is merged and folded into a new base
    /// layer, at which point the overlay layer describes a revision nobody
    /// reads. Removes only [`LAYER_FILE`] — never the overlay directory — because
    /// that directory is a shared namespace, not this module's alone:
    /// `crate::commands::context::record_edit` keeps `dirty-paths.json` there,
    /// and `crate::context::delivery` keeps `session-retrieval/*.json` there,
    /// and both outlive the graph layer and are read by other stages after
    /// this one merges. A `remove_dir_all` here would delete those out from
    /// under their owners on a schedule this module does not control; `.loom/work/`
    /// is removed wholesale when the plan finishes, so the leftover directory
    /// does not accumulate across plans.
    pub fn discard_overlay(&self, plan: &str, stage: &str) -> Result<()> {
        let path = self.overlay_path(plan, stage);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error)
                .with_context(|| format!("Failed to remove overlay layer: {}", path.display())),
        }
    }

    /// The view a reader sees: base at `revision` with `plan`/`stage`'s overlay
    /// applied on top.
    ///
    /// Pass `None` for the stage to read the base layer alone. A missing base
    /// is not an error — an overlay-only view is exactly what a stage sees
    /// before the host has ever published a base.
    pub fn resolved(&self, revision: &str, stage: Option<(&str, &str)>) -> Result<ResolvedGraph> {
        let base = self.load_base(revision)?.unwrap_or_default();
        let mut resolved = ResolvedGraph {
            base_revision: base.revision.clone(),
            overlaid: BTreeSet::new(),
            files: base.files,
        };

        if let Some((plan, stage)) = stage {
            if let Some(overlay) = self.load_overlay(plan, stage)? {
                for (path, entry) in overlay.files {
                    // Wholesale replacement, never a merge: an overlay entry is
                    // the complete truth for that file in this stage.
                    resolved.files.insert(path.clone(), entry);
                    resolved.overlaid.insert(path);
                }
            }
        }

        Ok(resolved)
    }
}

/// Read one layer file, treating absence as "never written".
fn read_layer(path: &Path) -> Result<Option<GraphLayer>> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to read source graph: {}", path.display()));
        }
    };

    serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse source graph: {}", path.display()))
        .map(Some)
}

/// Write one layer file with a locked, crash-atomic replacement.
fn write_layer(path: &Path, layer: &GraphLayer) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create source graph directory: {}",
                parent.display()
            )
        })?;
    }
    let content = canonical_json(layer)?;
    crate::fs::locking::locked_write(path, &content)
        .with_context(|| format!("Failed to write source graph: {}", path.display()))
}

mod prune;

#[cfg(test)]
mod tests;
