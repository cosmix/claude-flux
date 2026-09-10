use anyhow::Result;
use chrono::Utc;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

use super::{elapsed_ms, Enumeration, SourceGraphCounters, SourceGraphScope, WorkingTree};
use crate::context::extract::{self, extract_file};
use crate::context::graph_store::{FileEntry, GraphLayer, GraphStore};
use crate::context::refresh::BoxedExtractor;
use crate::context::source_graph::{body_hash, FileCoverage};
use crate::context::store::canonical_json;
use crate::fs::safe_read::read_bounded;
use crate::git::runner::run_git_checked;

pub(super) struct LayerBuild {
    pub layer: GraphLayer,
    pub counters: SourceGraphCounters,
}

/// Everything a per-file reuse-or-parse decision needs, bundled so the loop
/// body in [`build_layer`] can pass it in one argument.
struct FileContext<'a> {
    project_root: &'a Path,
    scope: &'a SourceGraphScope,
    tree: &'a WorkingTree,
    enumeration: &'a Enumeration,
    previous: Option<&'a GraphLayer>,
    base: Option<&'a GraphLayer>,
    extractors: &'a [BoxedExtractor],
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_layer(
    project_root: &Path,
    scope: &SourceGraphScope,
    enumeration: &Enumeration,
    tree: &WorkingTree,
    revision: String,
    previous: Option<&GraphLayer>,
    base: Option<&GraphLayer>,
    extractors: &[BoxedExtractor],
) -> LayerBuild {
    let mut counters = SourceGraphCounters {
        files_untracked: enumeration.untracked.len(),
        ..Default::default()
    };
    let deleted = deleted_paths(project_root, scope, enumeration, tree, base);
    let active = active_paths(scope, enumeration, &deleted);
    counters.files_enumerated = active.len().saturating_add(deleted.len());
    counters.files_deleted = deleted.len();

    let mut files = BTreeMap::new();
    for path in deleted {
        files.insert(path, FileEntry::tombstone());
    }
    let ctx = FileContext {
        project_root,
        scope,
        tree,
        enumeration,
        previous,
        base,
        extractors,
    };
    for path in active {
        let entry = resolve_file_entry(&ctx, &path, &mut counters);
        files.insert(path, entry);
    }

    LayerBuild {
        layer: assemble_layer(scope, tree, revision, enumeration, files),
        counters,
    }
}

/// Wrap the resolved per-file entries into a [`GraphLayer`], deriving
/// `generation` from the scope (an overlay tracks its tree's generation, a
/// base layer has none) and copying the enumeration's blob index.
fn assemble_layer(
    scope: &SourceGraphScope,
    tree: &WorkingTree,
    revision: String,
    enumeration: &Enumeration,
    files: BTreeMap<String, FileEntry>,
) -> GraphLayer {
    GraphLayer {
        revision,
        generation: match scope {
            SourceGraphScope::Overlay { .. } => tree.generation.clone(),
            SourceGraphScope::Base { .. } => String::new(),
        },
        built_at: Some(Utc::now()),
        files,
        blob_index: enumeration.known.clone(),
    }
}

/// Decide, for one path, whether the previous or base layer's entry can be
/// reused (by blob oid, then by content hash) or whether it must be read and
/// re-parsed - accounting the outcome into `counters` either way.
fn resolve_file_entry(
    ctx: &FileContext,
    path: &str,
    counters: &mut SourceGraphCounters,
) -> FileEntry {
    let current_oid = ctx.enumeration.known.get(path);
    let is_dirty = ctx.tree.dirty.contains_key(path);
    if let Some(entry) = reuse_by_oid(
        path,
        current_oid,
        is_dirty,
        ctx.previous,
        ctx.base,
        ctx.extractors,
    ) {
        counters.files_reused += 1;
        return entry;
    }

    let hash_started = Instant::now();
    let bytes = read_bytes(ctx.project_root, ctx.scope, ctx.tree, path);
    let bytes = match bytes {
        Ok(bytes) => bytes,
        Err(error) => {
            counters.hash_ms = counters.hash_ms.saturating_add(elapsed_ms(hash_started));
            return unreadable_entry(error);
        }
    };
    let hash = body_hash(&bytes);
    counters.hash_ms = counters.hash_ms.saturating_add(elapsed_ms(hash_started));
    counters.files_hashed += 1;
    if let Some(entry) = reuse_by_hash(path, &hash, ctx.previous, ctx.base, ctx.extractors) {
        counters.files_reused += 1;
        return entry;
    }

    let parse_started = Instant::now();
    let extraction = extract_file(ctx.extractors, Path::new(path), &bytes);
    counters.parse_ms = counters.parse_ms.saturating_add(elapsed_ms(parse_started));
    counters.files_parsed += 1;
    FileEntry {
        content_hash: hash,
        nodes: extraction.nodes,
        edges: extraction.edges,
        coverage: extraction.coverage,
    }
}

fn active_paths(
    scope: &SourceGraphScope,
    enumeration: &Enumeration,
    deleted: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut active = enumeration
        .known
        .keys()
        .filter(|path| !deleted.contains(*path))
        .cloned()
        .collect::<BTreeSet<_>>();
    if matches!(scope, SourceGraphScope::Overlay { .. }) {
        active.extend(enumeration.untracked.iter().cloned());
    }
    active
}

fn deleted_paths(
    project_root: &Path,
    scope: &SourceGraphScope,
    enumeration: &Enumeration,
    tree: &WorkingTree,
    base: Option<&GraphLayer>,
) -> BTreeSet<String> {
    if !matches!(scope, SourceGraphScope::Overlay { .. }) {
        return BTreeSet::new();
    }
    let mut deleted = enumeration.deleted.iter().cloned().collect::<BTreeSet<_>>();
    // A symlink or gitlink only becomes a tombstone when the base already
    // holds a regular-file entry for the same path - the same predicate
    // `persist_layer` applies when it prunes a tombstone the base never had -
    // so a repo-native symlink never inflates `files_deleted`.
    deleted.extend(
        enumeration
            .not_source
            .iter()
            .filter(|path| base.is_some_and(|base| base.files.contains_key(path.as_str())))
            .cloned(),
    );
    deleted.extend(
        tree.dirty
            .keys()
            .filter(|path| !project_root.join(path).exists())
            .cloned(),
    );
    deleted
}

fn reuse_by_oid(
    path: &str,
    current_oid: Option<&String>,
    dirty: bool,
    previous: Option<&GraphLayer>,
    base: Option<&GraphLayer>,
    extractors: &[BoxedExtractor],
) -> Option<FileEntry> {
    if dirty {
        return None;
    }
    let oid = current_oid?;
    [previous, base].into_iter().flatten().find_map(|layer| {
        (layer.blob_index.get(path) == Some(oid))
            .then(|| layer.files.get(path))
            .flatten()
            .filter(|entry| !entry.content_hash.is_empty())
            .filter(|entry| parser_version_matches(entry, extractors, Path::new(path)))
            .cloned()
    })
}

fn reuse_by_hash(
    path: &str,
    hash: &str,
    previous: Option<&GraphLayer>,
    base: Option<&GraphLayer>,
    extractors: &[BoxedExtractor],
) -> Option<FileEntry> {
    [previous, base].into_iter().flatten().find_map(|layer| {
        layer
            .files
            .get(path)
            .filter(|entry| entry.content_hash == hash)
            .filter(|entry| parser_version_matches(entry, extractors, Path::new(path)))
            .cloned()
    })
}

fn read_bytes(
    project_root: &Path,
    scope: &SourceGraphScope,
    tree: &WorkingTree,
    path: &str,
) -> std::result::Result<Vec<u8>, String> {
    if matches!(scope, SourceGraphScope::Base { .. }) && tree.dirty.contains_key(path) {
        let object = format!("HEAD:{path}");
        return run_git_checked(&["show", &object], project_root)
            .map(String::into_bytes)
            .map_err(|error| error.to_string());
    }
    // The working tree is a sandboxed stage's to write, so a symlink there -
    // tracked, untracked, or swapped in after enumeration - is refused at every
    // component instead of followed out of the repository; the refusal lands in
    // `unreadable_entry` like any other unreadable file. Unbounded, as before.
    read_bounded(project_root, Path::new(path), usize::MAX).map_err(|error| format!("{error:#}"))
}

fn unreadable_entry(error: String) -> FileEntry {
    FileEntry {
        content_hash: String::new(),
        nodes: Vec::new(),
        edges: Vec::new(),
        coverage: FileCoverage::LexicalOnly {
            detail: format!("unreadable: {error}"),
        },
    }
}

pub(in super::super) fn parser_version_matches(
    entry: &FileEntry,
    extractors: &[BoxedExtractor],
    path: &Path,
) -> bool {
    let Some(node) = entry.nodes.first() else {
        return true;
    };
    match extractors.iter().find(|extractor| extractor.supports(path)) {
        Some(extractor) => extractor.cache_identity().to_parser_version() == node.parser_version,
        None => node.parser_version == extract::lexical::LEXICAL_PARSER_VERSION,
    }
}

pub(super) fn persist_layer(
    graph_store: &GraphStore,
    scope: &SourceGraphScope,
    layer: &GraphLayer,
    previous: Option<&GraphLayer>,
    base: Option<&GraphLayer>,
) -> Result<u64> {
    match scope {
        SourceGraphScope::Overlay { plan, stage } => {
            let mut overlay = layer.clone();
            overlay.files.retain(|path, entry| match &entry.coverage {
                FileCoverage::Deleted => base.is_some_and(|base| base.files.contains_key(path)),
                _ => base.and_then(|base| base.files.get(path)) != Some(&*entry),
            });
            let unchanged = previous.is_some_and(|previous| {
                previous.revision == overlay.revision
                    && previous.generation == overlay.generation
                    && previous.files == overlay.files
                    && previous.blob_index == overlay.blob_index
            });
            if unchanged {
                return Ok(0);
            }
            let bytes = serialized_len(&overlay)?;
            graph_store.save_overlay(plan, stage, &overlay)?;
            Ok(bytes)
        }
        SourceGraphScope::Base { revision } => {
            let bytes = serialized_len(layer)?;
            graph_store
                .publish_base(revision, layer)
                .map(|written| if written { bytes } else { 0 })
        }
    }
}

fn serialized_len(layer: &GraphLayer) -> Result<u64> {
    Ok(u64::try_from(canonical_json(layer)?.len()).unwrap_or(u64::MAX))
}
