//! Main Orchestrator struct and public interface

use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use crate::language::{detect_project_languages, DetectedLanguage};
use crate::models::session::Session;
use crate::models::worktree::Worktree;
use crate::orchestrator::adjudication::AdjudicatorRegistry;
use crate::orchestrator::monitor::{Monitor, MonitorConfig};
use crate::plan::schema::SandboxConfig;
use crate::plan::ExecutionGraph;
use crate::skills::SkillIndex;

use super::clear_status_line;
use crate::orchestrator::liveness::LivenessService;
use crate::orchestrator::terminal::backend::SessionBackend;

/// Configuration for the orchestrator
#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    pub max_parallel_sessions: usize,
    pub poll_interval: Duration,
    pub manual_mode: bool,
    /// Watch mode: continuously spawn ready stages until all are terminal
    pub watch_mode: bool,
    pub work_dir: PathBuf,
    pub repo_root: PathBuf,
    /// How often to print status updates during polling (default: 30 seconds)
    pub status_update_interval: Duration,
    /// Enable automatic merge when stages complete (default: true)
    pub auto_merge: bool,
    /// Base branch to use for stages with no dependencies (from config.toml)
    pub base_branch: Option<String>,
    /// Directory containing skill files (default: ~/.claude/skills/)
    pub skills_dir: Option<PathBuf>,
    /// Enable skill routing recommendations in signals (default: true)
    pub enable_skill_routing: bool,
    /// Maximum number of skill recommendations per signal (default: 5)
    pub max_skill_recommendations: usize,
    /// Plan-level sandbox configuration (defaults for all stages)
    pub sandbox_config: SandboxConfig,
    /// Shutdown flag for graceful termination (used by daemon)
    pub shutdown_flag: Option<Arc<AtomicBool>>,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            max_parallel_sessions: 4,
            poll_interval: Duration::from_secs(5),
            manual_mode: false,
            watch_mode: false,
            work_dir: PathBuf::from(".loom/work"),
            repo_root: PathBuf::from("."),
            status_update_interval: Duration::from_secs(30),
            auto_merge: true,
            base_branch: None,
            skills_dir: None, // Will default to ~/.claude/skills/ when loading
            enable_skill_routing: true,
            max_skill_recommendations: 8,
            sandbox_config: SandboxConfig::default(),
            shutdown_flag: None,
        }
    }
}

/// Main orchestrator coordinating stage execution
pub struct Orchestrator {
    pub(super) config: OrchestratorConfig,
    pub(super) graph: ExecutionGraph,
    pub(super) active_sessions: HashMap<String, Session>,
    pub(super) active_worktrees: HashMap<String, Worktree>,
    pub(super) monitor: Monitor,
    /// Track reported crashes to avoid duplicate messages
    pub(super) reported_crashes: HashSet<String>,
    /// Session backend (native or tmux, selected by the persisted `[terminal]`
    /// config) — spawns and kills sessions.
    pub(super) backend: Arc<SessionBackend>,
    /// Liveness probe (shared with the monitor thread).
    pub(super) liveness: LivenessService,
    /// Skill index for generating skill recommendations in signals
    pub(super) skill_index: Option<SkillIndex>,
    /// Detected project languages for signal skill injection
    pub(super) detected_languages: Vec<DetectedLanguage>,
    /// Stage IDs that have had a one-shot auto-merge retry attempted during
    /// this daemon session. Prevents the retry from firing on every 5-second
    /// poll for stages that remain stuck as `Completed + !merged`.
    ///
    /// Lifecycle: in-memory only; reset on next `loom run`.
    pub(super) merge_retry_attempted: HashSet<String>,
    /// Stage IDs whose `Completed + merged=true` ancestry has already been
    /// git-verified during this daemon session. Memoizes the result so
    /// `verify_merged_true_or_revert` does not re-run `git symbolic-ref` +
    /// `git merge-base --is-ancestor` per Completed+merged stage every 5s
    /// tick (P-3) — these facts cannot change absent a history rewrite.
    ///
    /// Invalidated (entry removed) whenever a reconcile mutation flips a
    /// stage's merged flag, so a phantom-merge revert forces re-verification.
    ///
    /// Lifecycle: in-memory only; reset on next `loom run`.
    pub(super) verified_merged: HashSet<String>,
    /// Stage IDs for which spawn-time dependency verification has already
    /// logged a skip reason. Prevents the 5-second poll loop from flooding
    /// the logs when a dependent stage cannot start because of a phantom
    /// merge. Used by `stage_executor.rs::start_stage`.
    pub(super) spawn_skip_logged: HashSet<String>,
    /// Stages whose spool drain has already been reported as failing.
    /// Prevents the 5-second poll loop from flooding the logs.
    pub(super) spool_drain_error_logged: HashSet<String>,
    /// Adjudicator entry points. Stateless: dispute state lives on disk, so
    /// this survives a daemon restart without losing anything.
    pub(super) adjudicators: AdjudicatorRegistry,
    /// Why each ready-but-unstarted stage did not spawn on the current tick.
    ///
    /// `start_stage` records its decline reason here and clears the entry when
    /// the stage does spawn; `start_ready_stages` serialises the map to
    /// `.loom/work/scheduling.json` at the end of every pass. Populated fresh each
    /// tick, so it never reports a reason that has since gone away.
    pub(super) spawn_blocks: HashMap<String, crate::orchestrator::scheduling_report::BlockReason>,
    /// When each stage was first seen ready-but-unstarted, for "queued for X".
    ///
    /// In-memory only: a daemon restart is a fresh scheduling attempt, so
    /// resetting the clock is the honest reading.
    pub(super) queued_since: HashMap<String, DateTime<Utc>>,
}

impl Orchestrator {
    /// Create a new orchestrator from config and execution graph
    pub fn new(config: OrchestratorConfig, graph: ExecutionGraph) -> Result<Self> {
        anyhow::ensure!(
            config.max_parallel_sessions > 0,
            "max_parallel_sessions must be at least 1"
        );
        let monitor_config = MonitorConfig {
            poll_interval: config.poll_interval,
            work_dir: config.work_dir.clone(),
            ..Default::default()
        };

        let mut monitor = Monitor::new(monitor_config);

        let backend = Arc::new(SessionBackend::from_config(config.work_dir.clone())?);
        let liveness = LivenessService::new(Arc::clone(&backend));
        monitor.set_liveness(liveness.clone());

        // Load skill index if skill routing is enabled
        let skill_index = if config.enable_skill_routing {
            Self::load_skill_index(&config)
        } else {
            None
        };

        // Detect project languages for skill recommendations
        let detected_languages = detect_project_languages(&config.repo_root);

        // Disputes are adjudicated by a session the terminal backend spawns on
        // demand, so there is nothing to resolve up front: whether a session
        // can be started at all is the backend's answer, given per spawn, and
        // a failure escalates the dispute it was for rather than the run.
        let adjudicators = AdjudicatorRegistry::new();

        // Reconcile any orphaned plan-amendment snapshots from a prior
        // crash. This is cheap (no I/O when no snapshots exist) and must
        // happen BEFORE the first poll tick reads stage acceptance arrays.
        if let Err(e) = crate::plan::amendment::verify_plan_versions_consistency(
            &resolve_plan_path_for_startup(&config.work_dir).unwrap_or_default(),
            &config.work_dir,
        ) {
            tracing::warn!(
                target: "loom::adjudication",
                error = %e,
                "plan-amendment consistency check failed at startup",
            );
        }

        Ok(Self {
            config,
            graph,
            active_sessions: HashMap::new(),
            active_worktrees: HashMap::new(),
            monitor,
            reported_crashes: HashSet::new(),
            backend,
            liveness,
            skill_index,
            detected_languages,
            merge_retry_attempted: HashSet::new(),
            verified_merged: HashSet::new(),
            spawn_skip_logged: HashSet::new(),
            spool_drain_error_logged: HashSet::new(),
            adjudicators,
            spawn_blocks: HashMap::new(),
            queued_since: HashMap::new(),
        })
    }

    /// Load the skill index from the configured or default directory
    fn load_skill_index(config: &OrchestratorConfig) -> Option<SkillIndex> {
        // Determine skills directory: use config or default to ~/.claude/skills/
        let skills_dir = config.skills_dir.clone().unwrap_or_else(|| {
            dirs::home_dir()
                .map(|h| h.join(".claude").join("skills"))
                .unwrap_or_else(|| PathBuf::from(".claude/skills"))
        });

        if !skills_dir.exists() {
            return None;
        }

        match crate::skills::load_with_catalog(&skills_dir) {
            Ok(index) => {
                if index.is_empty() {
                    None
                } else {
                    Some(index)
                }
            }
            Err(e) => {
                eprintln!("Warning: Failed to load skill index: {e}");
                None
            }
        }
    }

    /// Count currently running sessions
    pub fn running_session_count(&self) -> usize {
        self.active_sessions.len()
    }

    /// Poll `.loom/work/disputes/` and start an adjudication session for every
    /// dispute that needs one. See
    /// [`AdjudicatorRegistry::start_pending_adjudications`] and
    /// [`AdjudicatorRegistry::disputes_awaiting_session`] for the guards.
    pub(crate) fn check_pending_disputes(&mut self) -> Result<()> {
        let started = self.adjudicators.start_pending_adjudications(
            &self.backend,
            &self.config.work_dir,
            &self.config.repo_root,
        )?;
        for started in started {
            clear_status_line();
            eprintln!(
                "Spawned adjudication session for stage '{}' dispute {}: {}",
                started.stage_id, started.dispute_id, started.session_id
            );
        }
        Ok(())
    }
}

/// Resolve the plan source_path from `.loom/work/config.toml` for daemon-
/// startup recovery. Returns `None` when there is no config yet (e.g.
/// during a test that doesn't initialise one).
fn resolve_plan_path_for_startup(work_dir: &std::path::Path) -> Option<PathBuf> {
    let cfg = crate::fs::work_dir::load_config(work_dir).ok().flatten()?;
    let path = cfg.source_path()?;
    if path.is_absolute() {
        Some(path)
    } else {
        let root = work_dir
            .canonicalize()
            .ok()
            .and_then(|wd| wd.parent().map(|p| p.to_path_buf()))?;
        Some(root.join(path))
    }
}

/// Result of orchestrator run
#[derive(Debug)]
pub struct OrchestratorResult {
    pub completed_stages: Vec<String>,
    pub failed_stages: Vec<String>,
    pub needs_handoff: Vec<String>,
    pub total_sessions_spawned: usize,
    /// When the orchestrator started running
    pub started_at: DateTime<Utc>,
    /// When the orchestrator finished running
    pub completed_at: DateTime<Utc>,
}

impl OrchestratorResult {
    pub fn is_success(&self) -> bool {
        self.failed_stages.is_empty() && self.needs_handoff.is_empty()
    }
}
