//! One policy-driven decision path for ensuring source-graph snapshots.

use anyhow::{Context, Result};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

use super::source_graph::{
    clean_generation, reconcile_with_working_tree, working_tree, WorkingTree,
};
use super::{SourceGraphCounters, SourceGraphOutcome, SourceGraphScope};
use crate::context::extract;
use crate::context::graph_store::{GraphLayer, GraphStore};
use crate::context::local_overlay::{local_overlay_key, LOCAL_PLAN_KEY};
use crate::context::source_graph::FileCoverage;
use crate::context::store::ContextStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotPolicy {
    /// Base for HEAD present, plus the `_local` overlay current when the tree is dirty.
    LocalCurrent,
    /// The named stage overlay current (a stage worktree; no base publish).
    StageOverlay { plan: String, stage: String },
    /// Base for HEAD present; the working tree is not consulted beyond HEAD.
    BaseOnly,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotAction {
    Reused,
    Updated,
    Rebuilt,
    Unavailable,
}
impl SnapshotAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reused => "reused",
            Self::Updated => "updated",
            Self::Rebuilt => "rebuilt",
            Self::Unavailable => "unavailable",
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotOutcome {
    pub action: SnapshotAction,
    pub reason: String,
    pub revision: String,
    pub generation: String,
    pub overlay: Option<(String, String)>,
    pub counters: SourceGraphCounters,
    pub elapsed: Duration,
}
impl SnapshotOutcome {
    /// Render the one advisory line shared by every source-graph entry point.
    pub fn describe(&self) -> String {
        if self.action == SnapshotAction::Unavailable {
            return format!("source graph: unavailable ({})", self.reason);
        }

        let target = self.target_description();
        if self.action == SnapshotAction::Reused {
            let state = if self.overlay.is_some() {
                "generation current"
            } else if self.generation == clean_generation(&self.revision) {
                "tree clean"
            } else {
                "tree dirty"
            };
            return format!("source graph: reused {target} ({state})");
        }

        format!(
            "source graph: {} {target} ({}; {})",
            self.action.as_str(),
            describe_counters(&self.counters),
            describe_elapsed(self.elapsed)
        )
    }

    fn target_description(&self) -> String {
        match &self.overlay {
            Some((plan, stage)) if plan == LOCAL_PLAN_KEY => {
                format!("local overlay {plan}/{stage}")
            }
            Some((plan, stage)) => format!("stage overlay {plan}/{stage}"),
            None => format!("base {}", super::short_revision(&self.revision)),
        }
    }
}

/// Ensure the policy-selected graph layer without making callers repeat its decision tree.
pub fn ensure_snapshot(
    store: &ContextStore,
    graph_store: &GraphStore,
    project_root: &Path,
    policy: SnapshotPolicy,
) -> Result<SnapshotOutcome> {
    let started = Instant::now();
    let tree = match working_tree(project_root) {
        Ok(tree) => tree,
        Err(error) => {
            let reason = format!("failed to inspect the working tree: {error}");
            let _ = super::mark_semantic_stale(store, &reason);
            return Ok(unavailable(reason, started.elapsed()));
        }
    };

    let mut outcome = match policy {
        SnapshotPolicy::BaseOnly => ensure_base_only(store, graph_store, project_root, &tree)?,
        SnapshotPolicy::LocalCurrent => {
            ensure_local_current(store, graph_store, project_root, &tree)?
        }
        SnapshotPolicy::StageOverlay { plan, stage } => {
            ensure_stage_overlay(store, graph_store, project_root, &tree, plan, stage)?
        }
    };
    outcome.elapsed = started.elapsed();
    Ok(outcome)
}

fn ensure_base_only(
    store: &ContextStore,
    graph_store: &GraphStore,
    project_root: &Path,
    tree: &WorkingTree,
) -> Result<SnapshotOutcome> {
    match ensure_base(store, graph_store, project_root, tree)? {
        Some(outcome) => from_reconcile(tree, None, outcome, "base refreshed"),
        None => Ok(reused(
            tree,
            None,
            format!(
                "base for {} present; {}",
                super::short_revision(&tree.head),
                tree_state(tree)
            ),
        )),
    }
}

fn ensure_local_current(
    store: &ContextStore,
    graph_store: &GraphStore,
    project_root: &Path,
    tree: &WorkingTree,
) -> Result<SnapshotOutcome> {
    let base = ensure_base(store, graph_store, project_root, tree)?;
    if tree.generation == clean_generation(&tree.head) {
        return match base {
            Some(outcome) => from_reconcile(tree, None, outcome, "base refreshed; tree clean"),
            None => Ok(reused(
                tree,
                None,
                format!(
                    "base for {} present; tree clean",
                    super::short_revision(&tree.head)
                ),
            )),
        };
    }

    let (plan, stage) = local_overlay_key(project_root);
    ensure_local_overlay(store, graph_store, project_root, tree, plan, stage, base)
}

fn ensure_base(
    store: &ContextStore,
    graph_store: &GraphStore,
    project_root: &Path,
    tree: &WorkingTree,
) -> Result<Option<SourceGraphOutcome>> {
    if let Some(base) = graph_store.load_base(&tree.head)? {
        if layer_is_current(&base) {
            return Ok(None);
        }
        let path = graph_store.base_path(&tree.head);
        fs::remove_file(&path)
            .with_context(|| format!("Failed to replace stale source graph: {}", path.display()))?;
    }
    reconcile_with_working_tree(
        store,
        graph_store,
        project_root,
        SourceGraphScope::Base {
            revision: tree.head.clone(),
        },
        tree,
    )
    .map(Some)
}

fn ensure_stage_overlay(
    store: &ContextStore,
    graph_store: &GraphStore,
    project_root: &Path,
    tree: &WorkingTree,
    plan: String,
    stage: String,
) -> Result<SnapshotOutcome> {
    let overlay = Some((plan.clone(), stage.clone()));
    if overlay_is_current(graph_store, &plan, &stage, &tree.generation)? {
        return Ok(reused(
            tree,
            overlay,
            format!("stage overlay {plan}/{stage} generation current"),
        ));
    }
    let outcome = reconcile_overlay(store, graph_store, project_root, tree, &plan, &stage)?;
    from_reconcile(tree, overlay, outcome, "stage overlay refreshed")
}

#[allow(clippy::too_many_arguments)]
fn ensure_local_overlay(
    store: &ContextStore,
    graph_store: &GraphStore,
    project_root: &Path,
    tree: &WorkingTree,
    plan: String,
    stage: String,
    base: Option<SourceGraphOutcome>,
) -> Result<SnapshotOutcome> {
    let overlay = Some((plan.clone(), stage.clone()));
    if overlay_is_current(graph_store, &plan, &stage, &tree.generation)? {
        return match base {
            Some(outcome) => from_reconcile(tree, overlay, outcome, "base refreshed"),
            None => Ok(reused(
                tree,
                overlay,
                format!("local overlay {plan}/{stage} generation current"),
            )),
        };
    }

    let mut outcome = reconcile_overlay(store, graph_store, project_root, tree, &plan, &stage)?;
    if let Some(base) = base {
        outcome.counters.accumulate(base.counters);
    }
    from_reconcile(tree, overlay, outcome, "local overlay refreshed")
}

fn reconcile_overlay(
    store: &ContextStore,
    graph_store: &GraphStore,
    project_root: &Path,
    tree: &WorkingTree,
    plan: &str,
    stage: &str,
) -> Result<SourceGraphOutcome> {
    reconcile_with_working_tree(
        store,
        graph_store,
        project_root,
        SourceGraphScope::Overlay {
            plan: plan.to_string(),
            stage: stage.to_string(),
        },
        tree,
    )
}

fn overlay_is_current(
    graph_store: &GraphStore,
    plan: &str,
    stage: &str,
    generation: &str,
) -> Result<bool> {
    Ok(graph_store
        .load_overlay(plan, stage)?
        .as_ref()
        .is_some_and(|layer| layer.generation == generation && layer_is_current(layer)))
}

fn layer_is_current(layer: &GraphLayer) -> bool {
    let extractors = extract::registry();
    layer.files.iter().all(|(path, entry)| {
        let Some(node) = entry.nodes.first() else {
            return true;
        };
        let path = Path::new(path);
        match extractors.iter().find(|extractor| extractor.supports(path)) {
            Some(extractor) => {
                extractor.cache_identity().to_parser_version() == node.parser_version
            }
            None => {
                matches!(&entry.coverage, FileCoverage::Deleted)
                    || node.parser_version == extract::lexical::LEXICAL_PARSER_VERSION
            }
        }
    })
}

fn from_reconcile(
    tree: &WorkingTree,
    overlay: Option<(String, String)>,
    outcome: SourceGraphOutcome,
    reason: &str,
) -> Result<SnapshotOutcome> {
    if outcome.freshness.stale {
        let detail = outcome
            .freshness
            .detail
            .unwrap_or_else(|| "source graph could not be refreshed".to_string());
        return Ok(unavailable_with_tree(tree, detail));
    }
    let action = if outcome.counters.files_reused > 0 {
        SnapshotAction::Updated
    } else {
        SnapshotAction::Rebuilt
    };
    Ok(SnapshotOutcome {
        action,
        reason: reason.to_string(),
        revision: tree.head.clone(),
        generation: tree.generation.clone(),
        overlay,
        counters: outcome.counters,
        elapsed: Duration::ZERO,
    })
}

fn reused(
    tree: &WorkingTree,
    overlay: Option<(String, String)>,
    reason: String,
) -> SnapshotOutcome {
    SnapshotOutcome {
        action: SnapshotAction::Reused,
        reason,
        revision: tree.head.clone(),
        generation: tree.generation.clone(),
        overlay,
        counters: SourceGraphCounters::default(),
        elapsed: Duration::ZERO,
    }
}

fn unavailable(reason: String, elapsed: Duration) -> SnapshotOutcome {
    SnapshotOutcome {
        action: SnapshotAction::Unavailable,
        reason,
        revision: String::new(),
        generation: String::new(),
        overlay: None,
        counters: SourceGraphCounters::default(),
        elapsed,
    }
}

fn unavailable_with_tree(tree: &WorkingTree, reason: String) -> SnapshotOutcome {
    SnapshotOutcome {
        revision: tree.head.clone(),
        generation: tree.generation.clone(),
        ..unavailable(reason, Duration::ZERO)
    }
}

fn tree_state(tree: &WorkingTree) -> &'static str {
    if tree.generation == clean_generation(&tree.head) {
        "tree clean"
    } else {
        "tree dirty"
    }
}

fn describe_counters(counters: &SourceGraphCounters) -> String {
    let mut parts = Vec::new();
    if counters.files_parsed > 0 {
        parts.push(format!("{} parsed", counters.files_parsed));
    }
    if counters.files_reused > 0 {
        parts.push(format!("{} reused", counters.files_reused));
    }
    if counters.files_deleted > 0 {
        parts.push(format!("{} deleted", counters.files_deleted));
    }
    if parts.is_empty() {
        parts.push("0 parsed".to_string());
    }
    parts.join(", ")
}

fn describe_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs_f64();
    if seconds >= 10.0 {
        format!("{seconds:.1}s")
    } else {
        format!("{seconds:.2}s")
    }
}
