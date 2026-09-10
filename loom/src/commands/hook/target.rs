//! Shared environment-to-scope resolution for every hook delegate.

use std::path::PathBuf;

use crate::context::config::RetrievalConfig;
use crate::context::delivery;
use crate::context::local_overlay::{local_overlay_key, OverlayScope};
use crate::context::refresh::SnapshotPolicy;
use crate::fs::work_dir::WorkDir;
use crate::validation::validate_id;

/// The one project, delivery address, and graph overlay a hook invocation targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HookTarget {
    /// The `.loom/work` root. It may not exist yet for an ordinary checkout.
    pub work_dir: PathBuf,
    pub project_root: PathBuf,
    /// A real stage's plan key, or `_local` for a checkout.
    pub plan: String,
    /// A real stage id, or the checkout's `map-<dir>` key.
    pub stage_id: String,
    /// The real stage a "Pull more" footer may name; checkouts have none.
    pub pull_stage: Option<String>,
    /// The source-graph overlay retrieval must read.
    pub overlay: OverlayScope,
}

impl HookTarget {
    /// Resolve a real stage first, then fall back to the current checkout.
    pub(crate) fn from_environment() -> Option<Self> {
        Self::for_stage().or_else(Self::for_checkout)
    }

    fn for_stage() -> Option<Self> {
        let stage_id = non_empty_env("LOOM_STAGE_ID")?;
        validate_id(&stage_id).ok()?;
        let work_dir = WorkDir::new(non_empty_env("LOOM_WORK_DIR")?).ok()?;
        let stage = crate::verify::load_stage(&stage_id, work_dir.root()).ok()?;
        let plan = delivery::plan_key(&stage).to_string();
        let project_root = work_dir.project_root()?.to_path_buf();
        Some(HookTarget {
            work_dir: work_dir.root().to_path_buf(),
            project_root,
            plan: plan.clone(),
            stage_id: stage_id.clone(),
            pull_stage: Some(stage_id.clone()),
            overlay: OverlayScope::Stage {
                plan,
                stage: stage_id,
            },
        })
    }

    fn for_checkout() -> Option<Self> {
        let hint = non_empty_env("LOOM_WORK_DIR").unwrap_or_else(|| ".".to_string());
        let work_dir = WorkDir::new(hint).ok()?;
        let project_root = work_dir.project_root()?.to_path_buf();
        let (plan, stage_id) = local_overlay_key(&project_root);
        Some(HookTarget {
            work_dir: work_dir.root().to_path_buf(),
            project_root,
            plan,
            stage_id,
            pull_stage: None,
            overlay: OverlayScope::Local,
        })
    }

    /// Retrieval tunables loaded from the main project behind this checkout.
    pub(crate) fn retrieval_config(&self) -> RetrievalConfig {
        let main_root = WorkDir::new(&self.project_root)
            .ok()
            .and_then(|work_dir| work_dir.main_project_root())
            .unwrap_or_else(|| self.project_root.clone());
        RetrievalConfig::load(&main_root)
    }

    /// The snapshot policy matching [`Self::overlay`].
    pub(crate) fn snapshot_policy(&self) -> SnapshotPolicy {
        match &self.overlay {
            OverlayScope::Local => SnapshotPolicy::LocalCurrent,
            OverlayScope::Stage { plan, stage } => SnapshotPolicy::StageOverlay {
                plan: plan.clone(),
                stage: stage.clone(),
            },
        }
    }

    /// Whether the state root already exists on disk.
    pub(crate) fn exists(&self) -> bool {
        self.work_dir.exists()
    }
}

/// A set environment variable with non-blank content.
pub(crate) fn non_empty_env(name: &str) -> Option<String> {
    let value = std::env::var(name).ok()?;
    (!value.trim().is_empty()).then_some(value)
}

#[cfg(test)]
#[path = "tests_hook_target.rs"]
mod tests;
