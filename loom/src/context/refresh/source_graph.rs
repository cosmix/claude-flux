//! Building and persisting the semantic (source-graph) layer.
//!
//! [`reconcile_source_graph`] is the driver: it walks the repository's
//! tracked files, runs the extractor registry over whatever changed, and
//! persists the resulting [`GraphLayer`] through [`GraphStore`]. Nothing here
//! ever silently drops a file — an unreadable file, an oversized one, and a
//! parse error all keep a reported entry (see [`unreadable_entry`] and
//! `crate::context::extract::extract_file`).
//!
//! ## Known gaps
//!
//! An overlay cannot express a deletion: [`GraphStore::resolved`] computes
//! `overlay ∪ base`, so a file the stage deleted keeps its base entry, and a
//! view over the resolved graph (e.g. `loom map --outline <deleted-file>`)
//! still prints the old outline. Fixing this needs a tombstone concept in
//! `graph_store.rs`, which this module does not own.

use anyhow::Result;
use chrono::Utc;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::{BoxedExtractor, ReconcileInputs, ScopeLayers};
use crate::context::extract::{self, extract_file};
use crate::context::graph_store::{FileEntry, GraphLayer, GraphStore};
use crate::context::schema::Freshness;
use crate::context::source_graph::{body_hash, FileCoverage};
use crate::context::store::ContextStore;
use crate::git::runner::run_git_checked;

/// Which slice of the source graph to build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceGraphScope {
    /// Rebuild the stage's overlay from the working tree.
    Overlay { plan: String, stage: String },
    /// Publish an immutable base layer for a clean revision.
    Base { revision: String },
}

/// What [`reconcile_source_graph`] actually did, over the layer as walked and
/// built by THIS call — not necessarily what ended up on disk. A `Base`
/// republish of an already-published revision, for instance, still reports
/// the full counts of what was (re)built even though nothing new was written;
/// a refused publish (dirty tree) or a listing failure reports all zeros.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceGraphOutcome {
    pub files_extracted: usize,
    pub nodes: usize,
    pub edges: usize,
    pub freshness: Freshness,
}

/// Roots to never descend into: caches, worktrees, and build/dependency output.
/// `.loom` covers the current state root (`.loom/work`) and loom's own cache;
/// `.work` stays listed for a legacy-layout project that has not moved.
const EXCLUDED_ROOTS: &[&str] = &[
    ".loom",
    ".work",
    ".worktrees",
    "target",
    "node_modules",
    ".git",
];

/// Walk tracked files, extract or reuse each one, and persist the layer — a missing git repository is data, not a crash, see [`Freshness::never_built`].
pub fn reconcile_source_graph(
    store: &ContextStore,
    graph_store: &GraphStore,
    project_root: &Path,
    scope: SourceGraphScope,
) -> Result<SourceGraphOutcome> {
    let extractors: Vec<BoxedExtractor> = extract::registry();

    if let SourceGraphScope::Base { revision } = &scope {
        if let Some(reason) = dirty_tree_reason(project_root, revision) {
            return Ok(degraded_outcome(store, reason));
        }
    }

    let (files, revision, previous, base) =
        match gather_reconcile_inputs(&scope, graph_store, project_root)? {
            Ok(inputs) => inputs,
            Err(detail) => return Ok(degraded_outcome(store, detail)),
        };

    let layer = build_layer(
        project_root,
        &files,
        revision.clone(),
        previous.as_ref(),
        base.as_ref(),
        &extractors,
    );
    let files_extracted = layer.files.len();
    let (nodes, edges) = (layer.nodes().count(), layer.edges().count());

    persist_layer(
        graph_store,
        &scope,
        &layer,
        previous.as_ref(),
        base.as_ref(),
    )?;
    let freshness = persist_semantic_freshness(store, revision)?;

    Ok(SourceGraphOutcome {
        files_extracted,
        nodes,
        edges,
        freshness,
    })
}

/// A zero-count outcome carrying `detail` as a stale, never-built
/// [`Freshness`], with that staleness persisted immediately — a degraded
/// return must not leave `state.json` still claiming whatever it claimed
/// before. `mark_semantic_stale`'s own failure is swallowed; see its doc.
fn degraded_outcome(store: &ContextStore, detail: String) -> SourceGraphOutcome {
    let _ = mark_semantic_stale(store, &detail);
    SourceGraphOutcome {
        files_extracted: 0,
        nodes: 0,
        edges: 0,
        freshness: Freshness::never_built(detail),
    }
}

/// `None` when the TRACKED working tree is clean; `Some(reason)` when tracked
/// files have uncommitted changes, or cleanliness could not be verified —
/// either way `Base` must not publish. Untracked files are excluded on
/// purpose, for the same reason `src/git/merge/probe.rs:119-127`'s
/// `require_clean_repository` excludes them: this only reads tracked bytes,
/// so an untracked scratch file cannot poison the layer, and the production
/// caller (`MergeLifecycle::reconcile_base`) runs against the main repo root
/// right after a merge, where untracked files are the norm.
pub(super) fn dirty_tree_reason(project_root: &Path, revision: &str) -> Option<String> {
    let args = &["status", "--porcelain=v1", "--untracked-files=no"];
    match run_git_checked(args, project_root) {
        Ok(status) if status.is_empty() => None,
        Ok(_) => Some(format!(
            "working tree is dirty; base layer not published for {revision}"
        )),
        Err(error) => Some(format!(
            "could not verify working tree cleanliness for {revision}: {error}"
        )),
    }
}

/// `git rev-parse HEAD` for `project_root`, or `None` when this is not a git
/// repository, has no HEAD (e.g. no commits yet), or `git` itself is
/// unavailable. The second of the two places `context` reaches `crate::git`
/// (the first is [`dirty_tree_reason`] above) — deliberately kept to these
/// two call sites rather than importing `crate::git` a third time elsewhere,
/// see `doc/loom/knowledge/architecture/source-graph.md`'s "Building and
/// Persisting" section.
pub(crate) fn head_revision(project_root: &Path) -> Option<String> {
    run_git_checked(&["rev-parse", "HEAD"], project_root).ok()
}

/// List tracked files, dropping cache/build roots and paths git lists that no longer exist on disk.
///
/// Asks for `-z` (NUL-terminated, unquoted) output rather than the default
/// newline-terminated form. With `core.quotePath` on (git's default), any
/// tracked path containing a non-ASCII or otherwise "unusual" byte is
/// C-quoted on the newline form — e.g. `"loom/src/caf\303\251.rs"` — so a
/// split on `\n` would leave the quoted literal in `line`,
/// `project_root.join(line).exists()` would never resolve it, and the file
/// would be silently dropped from the graph, in violation of this module's
/// "nothing here ever silently drops a file" contract (see the module doc
/// comment above). `-z` sidesteps quoting entirely and, as a bonus, also
/// handles paths containing embedded spaces or newlines correctly. Do not
/// "simplify" this back to the newline form.
fn tracked_source_files(project_root: &Path) -> Result<Vec<String>> {
    let output = run_git_checked(&["ls-files", "-z"], project_root)?;

    let mut files: Vec<String> = output
        .split('\0')
        .filter(|line| !line.is_empty())
        .filter(|line| {
            let first_component = line.split('/').next().unwrap_or(line);
            !EXCLUDED_ROOTS.contains(&first_component)
        })
        .filter(|line| project_root.join(line).exists())
        .map(|line| line.to_string())
        .collect();
    files.sort();
    Ok(files)
}

/// Resolve the revision to build against, plus the reuse sources for
/// incremental extraction: `previous` (the stage's own overlay, `Overlay`
/// scope only) and `base` (the published base at that revision, if any —
/// for `Base` scope this IS the reuse source, loaded once here rather than
/// again inside `persist_layer`).
///
/// `Ok(Err(detail))` means the revision itself could not be resolved (a soft
/// failure the caller degrades to); `Err(_)` is a hard I/O failure on the
/// graph store, which the caller propagates.
fn resolve_scope_layers(
    scope: &SourceGraphScope,
    graph_store: &GraphStore,
    project_root: &Path,
) -> Result<Result<ScopeLayers, String>> {
    match scope {
        SourceGraphScope::Overlay { plan, stage } => {
            let head = match run_git_checked(&["rev-parse", "HEAD"], project_root) {
                Ok(head) => head,
                Err(error) => return Ok(Err(format!("cannot resolve HEAD: {error}"))),
            };
            let overlay = graph_store.load_overlay(plan, stage)?;
            let base = graph_store.load_base(&head)?;
            Ok(Ok((head, overlay, base)))
        }
        SourceGraphScope::Base { revision } => {
            let base = graph_store.load_base(revision)?;
            Ok(Ok((revision.clone(), None, base)))
        }
    }
}

/// Gathers the tracked-file list and the revision/reuse-source triple from
/// [`resolve_scope_layers`], so the caller reads as one "gather inputs, then
/// build" step. `Ok(Err(detail))` is a soft failure to degrade to; `Err(_)`
/// is a hard I/O failure on the graph store, propagated as-is.
fn gather_reconcile_inputs(
    scope: &SourceGraphScope,
    graph_store: &GraphStore,
    project_root: &Path,
) -> Result<Result<ReconcileInputs, String>> {
    let files = match tracked_source_files(project_root) {
        Ok(files) => files,
        Err(error) => return Ok(Err(format!("failed to list tracked source files: {error}"))),
    };

    let (revision, previous, base) = match resolve_scope_layers(scope, graph_store, project_root)? {
        Ok(resolved) => resolved,
        Err(detail) => return Ok(Err(detail)),
    };
    Ok(Ok((files, revision, previous, base)))
}

/// Extract or reuse an entry for every tracked file. Reuse checks `previous`
/// (the stage's own overlay) before falling back to `base`: the base holds
/// every file the stage has not itself touched, so skipping it here meant
/// almost every file was re-parsed on every incremental run. An unreadable
/// file gets a reported entry instead of vanishing from the layer.
fn build_layer(
    project_root: &Path,
    files: &[String],
    revision: String,
    previous: Option<&GraphLayer>,
    base: Option<&GraphLayer>,
    extractors: &[BoxedExtractor],
) -> GraphLayer {
    let mut entries: BTreeMap<String, FileEntry> = BTreeMap::new();

    for path in files {
        let bytes = match fs::read(project_root.join(path)) {
            Ok(bytes) => bytes,
            Err(error) => {
                entries.insert(path.clone(), unreadable_entry(error));
                continue;
            }
        };
        let hash = body_hash(&bytes);

        let reused = previous
            .and_then(|layer| layer.files.get(path))
            .or_else(|| base.and_then(|layer| layer.files.get(path)))
            .filter(|entry| entry.content_hash == hash)
            .filter(|entry| parser_version_matches(entry, extractors, Path::new(path)))
            .cloned();

        let entry = reused.unwrap_or_else(|| {
            let extraction = extract_file(extractors, Path::new(path), &bytes);
            FileEntry {
                content_hash: hash,
                nodes: extraction.nodes,
                edges: extraction.edges,
                coverage: extraction.coverage,
            }
        });

        entries.insert(path.clone(), entry);
    }

    GraphLayer {
        revision,
        generation: String::new(),
        built_at: Some(Utc::now()),
        files: entries,
        blob_index: BTreeMap::new(),
    }
}

/// A placeholder entry for a file `fs::read` could not open — reported, not
/// dropped. The empty `content_hash` can never equal a real `body_hash`
/// output, which guarantees the file is retried on the very next run.
fn unreadable_entry(error: std::io::Error) -> FileEntry {
    FileEntry {
        content_hash: String::new(),
        nodes: Vec::new(),
        edges: Vec::new(),
        coverage: FileCoverage::LexicalOnly {
            detail: format!("unreadable: {error}"),
        },
    }
}

/// Whether a cached entry's nodes still match the extractor build that would
/// claim `path` today — an entry with no nodes (e.g. an unreadable-file
/// placeholder) is always considered current. When no extractor claims
/// `path` today, a node already stamped [`extract::lexical::LEXICAL_PARSER_VERSION`]
/// took the lexical fallback and would again, so it stays current; any other
/// version came from an extractor that is no longer registered and is stale.
/// A grammar or query bump changes `parser_version`, invalidating every
/// unchanged file's stale nodes.
fn parser_version_matches(entry: &FileEntry, extractors: &[BoxedExtractor], path: &Path) -> bool {
    let Some(node) = entry.nodes.first() else {
        return true;
    };
    match extractors.iter().find(|extractor| extractor.supports(path)) {
        Some(extractor) => extractor.cache_identity().to_parser_version() == node.parser_version,
        None => node.parser_version == extract::lexical::LEXICAL_PARSER_VERSION,
    }
}

/// Persist `layer`: an overlay is pruned to a delta against `base` and
/// skipped when unchanged (by revision AND contents, so a moved HEAD always
/// re-stamps even with byte-identical files); a base republish is a no-op,
/// not an error — a published base layer is immutable.
fn persist_layer(
    graph_store: &GraphStore,
    scope: &SourceGraphScope,
    layer: &GraphLayer,
    previous: Option<&GraphLayer>,
    base: Option<&GraphLayer>,
) -> Result<()> {
    match scope {
        SourceGraphScope::Overlay { plan, stage } => {
            let mut overlay = layer.clone();
            if let Some(base) = base {
                overlay
                    .files
                    .retain(|path, entry| base.files.get(path) != Some(&*entry));
            }

            let unchanged = previous.is_some_and(|previous| {
                previous.revision == overlay.revision && previous.files == overlay.files
            });
            if unchanged {
                return Ok(());
            }
            graph_store.save_overlay(plan, stage, &overlay)
        }
        SourceGraphScope::Base { revision } => {
            graph_store.publish_base(revision, layer).map(|_| ())
        }
    }
}

/// Stamp a fresh, non-stale [`Freshness`] for `revision` and persist it as the store's state.
fn persist_semantic_freshness(store: &ContextStore, revision: String) -> Result<Freshness> {
    let freshness = Freshness {
        revision,
        computed_at: Some(Utc::now()),
        ..Default::default()
    };
    store.update_state(|state| state.semantic = freshness.clone())?;
    Ok(freshness)
}

/// Mark the semantic layer stale with `reason`. `ContextStore::update_state`
/// degrades a missing, unreadable, or malformed `state.json` to
/// [`crate::context::store::StoreState::default`] rather than erroring;
/// callers log and continue rather than propagate the `Result` — both
/// deliberate, so a corrupt or unwritable cache can never block a merge.
pub fn mark_semantic_stale(store: &ContextStore, reason: &str) -> Result<()> {
    store.update_state(|state| {
        state.semantic.stale = true;
        state.semantic.detail = Some(reason.to_string());
    })
}

#[cfg(test)]
#[path = "tests_source_graph.rs"]
mod tests;
