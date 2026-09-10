//! Building and persisting source-graph snapshots.

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Instant;

use crate::context::extract;
use crate::context::graph_store::{GraphLayer, GraphStore};
use crate::context::local_overlay::local_overlay_key;
use crate::context::schema::Freshness;
use crate::context::store::ContextStore;

mod enumerate;
mod generation;
mod layer;

use enumerate::enumerate;
pub(crate) use enumerate::Enumeration;
pub(crate) use generation::{clean_generation, working_tree, WorkingTree};
#[cfg(test)]
pub(super) use layer::parser_version_matches;
use layer::{build_layer, persist_layer};

/// Which source-graph snapshot to build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceGraphScope {
    /// Rebuild a stage's overlay from the working tree.
    Overlay { plan: String, stage: String },
    /// Publish an immutable base layer from committed `HEAD` content.
    Base { revision: String },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceGraphCounters {
    pub files_enumerated: usize,
    pub files_hashed: usize,
    pub files_parsed: usize,
    pub files_reused: usize,
    pub files_deleted: usize,
    pub files_untracked: usize,
    pub bytes_serialized: u64,
    pub enumerate_ms: u64,
    pub hash_ms: u64,
    pub parse_ms: u64,
    pub persist_ms: u64,
}

impl SourceGraphCounters {
    pub(crate) fn accumulate(&mut self, other: Self) {
        self.files_enumerated = self.files_enumerated.saturating_add(other.files_enumerated);
        self.files_hashed = self.files_hashed.saturating_add(other.files_hashed);
        self.files_parsed = self.files_parsed.saturating_add(other.files_parsed);
        self.files_reused = self.files_reused.saturating_add(other.files_reused);
        self.files_deleted = self.files_deleted.saturating_add(other.files_deleted);
        self.files_untracked = self.files_untracked.saturating_add(other.files_untracked);
        self.bytes_serialized = self.bytes_serialized.saturating_add(other.bytes_serialized);
        self.enumerate_ms = self.enumerate_ms.saturating_add(other.enumerate_ms);
        self.hash_ms = self.hash_ms.saturating_add(other.hash_ms);
        self.parse_ms = self.parse_ms.saturating_add(other.parse_ms);
        self.persist_ms = self.persist_ms.saturating_add(other.persist_ms);
    }
}

/// What one source-graph reconcile did.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceGraphOutcome {
    pub nodes: usize,
    pub edges: usize,
    pub freshness: Freshness,
    pub counters: SourceGraphCounters,
}

pub(super) const EXCLUDED_ROOTS: &[&str] = &[
    ".loom",
    ".work",
    ".worktrees",
    "target",
    "node_modules",
    ".git",
];

pub(super) fn excluded(path: &str) -> bool {
    let first = path.split('/').next().unwrap_or(path);
    EXCLUDED_ROOTS.contains(&first)
}

/// Enumerate, build, persist, and stamp one honest source-graph snapshot.
pub fn reconcile_source_graph(
    store: &ContextStore,
    graph_store: &GraphStore,
    project_root: &Path,
    scope: SourceGraphScope,
) -> Result<SourceGraphOutcome> {
    let tree = match working_tree(project_root) {
        Ok(tree) => tree,
        Err(error) => {
            return Ok(degraded_outcome(
                store,
                format!("failed to inspect the working tree: {error:#}"),
            ));
        }
    };
    reconcile_with_working_tree(store, graph_store, project_root, scope, &tree)
}

pub(super) fn reconcile_with_working_tree(
    store: &ContextStore,
    graph_store: &GraphStore,
    project_root: &Path,
    scope: SourceGraphScope,
    tree: &WorkingTree,
) -> Result<SourceGraphOutcome> {
    let enumerate_started = Instant::now();
    let enumeration = match enumerate(project_root, &scope) {
        Ok(enumeration) => enumeration,
        Err(error) => {
            return Ok(degraded_outcome(
                store,
                format!("failed to enumerate source files: {error:#}"),
            ));
        }
    };
    let enumerate_ms = elapsed_ms(enumerate_started);
    let (previous, base) = resolve_scope_layers(&scope, graph_store, project_root, &tree.head)?;
    let revision = scope_revision(&scope, tree);
    let mut built = build_layer(
        project_root,
        &scope,
        &enumeration,
        tree,
        revision.clone(),
        previous.as_ref(),
        base.as_ref(),
        &extract::registry(),
    );
    built.counters.enumerate_ms = enumerate_ms;

    persist_and_stamp(
        store,
        graph_store,
        &scope,
        previous.as_ref(),
        base.as_ref(),
        built,
        revision,
    )
}

fn scope_revision(scope: &SourceGraphScope, tree: &WorkingTree) -> String {
    match scope {
        SourceGraphScope::Overlay { .. } => tree.head.clone(),
        SourceGraphScope::Base { revision } => revision.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn persist_and_stamp(
    store: &ContextStore,
    graph_store: &GraphStore,
    scope: &SourceGraphScope,
    previous: Option<&GraphLayer>,
    base: Option<&GraphLayer>,
    mut built: layer::LayerBuild,
    revision: String,
) -> Result<SourceGraphOutcome> {
    let (nodes, edges) = (built.layer.nodes().count(), built.layer.edges().count());

    let persist_started = Instant::now();
    built.counters.bytes_serialized =
        persist_layer(graph_store, scope, &built.layer, previous, base)?;
    built.counters.persist_ms = elapsed_ms(persist_started);
    let freshness = persist_semantic_freshness(store, revision)?;

    Ok(SourceGraphOutcome {
        nodes,
        edges,
        freshness,
        counters: built.counters,
    })
}

fn resolve_scope_layers(
    scope: &SourceGraphScope,
    graph_store: &GraphStore,
    project_root: &Path,
    head: &str,
) -> Result<(Option<GraphLayer>, Option<GraphLayer>)> {
    match scope {
        SourceGraphScope::Overlay { plan, stage } => Ok((
            graph_store.load_overlay(plan, stage)?,
            graph_store.load_base(head)?,
        )),
        SourceGraphScope::Base { .. } => {
            let (plan, stage) = local_overlay_key(project_root);
            let local = graph_store.load_overlay(&plan, &stage)?;
            let newest = graph_store.load_newest_base()?;
            Ok(match local {
                Some(local) => (Some(local), newest),
                None => (newest, None),
            })
        }
    }
}

fn degraded_outcome(store: &ContextStore, detail: String) -> SourceGraphOutcome {
    let _ = mark_semantic_stale(store, &detail);
    SourceGraphOutcome {
        nodes: 0,
        edges: 0,
        freshness: Freshness::never_built(detail),
        counters: SourceGraphCounters::default(),
    }
}

pub(crate) fn head_revision(project_root: &Path) -> Option<String> {
    working_tree(project_root).ok().map(|tree| tree.head)
}

fn persist_semantic_freshness(store: &ContextStore, revision: String) -> Result<Freshness> {
    let freshness = Freshness {
        revision,
        computed_at: Some(Utc::now()),
        ..Default::default()
    };
    store.update_state(|state| state.semantic = freshness.clone())?;
    Ok(freshness)
}

pub fn mark_semantic_stale(store: &ContextStore, reason: &str) -> Result<()> {
    store.update_state(|state| {
        state.semantic.stale = true;
        state.semantic.detail = Some(reason.to_string());
    })
}

pub(super) fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "tests_source_graph/mod.rs"]
mod tests;
