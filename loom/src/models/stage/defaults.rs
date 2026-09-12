//! The one full `Stage` literal; every other constructor starts from it.

use super::types::{Implementers, Stage, StageStatus, StageType};

impl Default for Stage {
    fn default() -> Self {
        let now = chrono::Utc::now();
        Self {
            id: String::new(),
            name: String::new(),
            description: None,
            status: StageStatus::WaitingForDeps,
            dependencies: Vec::new(),
            parallel_group: None,
            acceptance: Vec::new(),
            setup: Vec::new(),
            files: Vec::new(),
            stage_type: StageType::default(),
            plan_id: None,
            worktree: None,
            session: None,
            held: false,
            parent_stage: None,
            child_stages: Vec::new(),
            created_at: now,
            updated_at: now,
            completed_at: None,
            started_at: None,
            duration_secs: None,
            execution_secs: None,
            attempt_started_at: None,
            close_reason: None,
            auto_merge: None,
            working_dir: Some(".".to_string()),
            retry_count: 0,
            max_retries: None,
            last_failure_at: None,
            failure_info: None,
            resolved_base: None,
            base_branch: None,
            base_merged_from: Vec::new(),
            outputs: Vec::new(),
            completed_commit: None,
            cleanup_warning: None,
            merged: false,
            merge_assumed: false,
            merge_conflict: false,
            verification_status: Default::default(),
            context_ceiling_tokens: None,
            plan_overview: None,
            artifacts: Vec::new(),
            wiring: Vec::new(),
            wiring_tests: Vec::new(),
            dead_code_check: None,
            before_stage: Vec::new(),
            after_stage: Vec::new(),
            code_review: None,
            fix_attempts: 0,
            dispute_count: 0,
            evidence_rounds: 0,
            amendments_applied: 0,
            stall_recoveries: 0,
            sandbox: Default::default(),
            execution_mode: None,
            max_fix_attempts: None,
            review_reason: None,
            bug_fix: None,
            regression_test: None,
            model: None,
            reasoning_effort: None,
            ultracode: false,
            implementers: Implementers::default(),
            subagent_timeout_secs: None,
        }
    }
}

/// Built-in per-[`StageType`] model and reasoning-effort defaults.
///
/// This is the LAST tier of a four-tier resolution chain, in precedence
/// order: a stage's own `model`/`reasoning_effort` plan fields, then
/// `.loom/work/config.toml`'s `[models]` section, then `~/.loom/config.toml`'s
/// `[models]` section, then the built-ins below.
/// [`crate::fs::work_dir::resolve_stage_model_effort`] is the single place
/// that walks the whole chain. A stage should OMIT `model`/`reasoning_effort`
/// so the configured tiers apply — it sets them only as a deliberate
/// per-stage override, not as the default path plans should take.
///
/// Every implementation stage's MAIN AGENT is an orchestrator: it reads
/// context, plans the work, and delegates implementation to subagents (sonnet
/// or codex terra workers for common implementation and integration tests,
/// codex luna workers for boilerplate/scaffolding/simple unit tests, opus
/// workers only where architecture or algorithm judgment is required). Model
/// choice for the actual implementation work happens at the subagent level,
/// not here. The one exception is knowledge-distill: a single-agent sonnet
/// pass over memories that are already compact summaries — no subagents.
impl StageType {
    /// Fallback model when neither a plan field nor a config tier sets one.
    pub fn default_model(&self) -> &'static str {
        match self {
            // Knowledge stages: the main agent orchestrates exploration and
            // delegates to Explore/sonnet subagents, curating their findings itself.
            StageType::Knowledge => "opus",
            // KnowledgeDistill curates stage memories into permanent knowledge:
            // a linear read-synthesize-write pass driven by sonnet with NO
            // subagents — the memories are already compact summaries, so the
            // volume and the judgment both fit a single sonnet session.
            StageType::KnowledgeDistill => "sonnet",
            // Standard and integration-verify stages: the main agent orchestrates
            // and delegates implementation/review work to subagents.
            StageType::Standard | StageType::IntegrationVerify => "opus",
        }
    }

    /// Fallback reasoning effort when neither a plan field nor a config tier
    /// sets one — per stage type, not uniform.
    pub fn default_reasoning_effort(&self) -> &'static str {
        match self {
            // A read-and-summarize pass over the tree: medium keeps the
            // knowledge bootstrap fast without shortchanging the map it builds.
            StageType::Knowledge => "medium",
            // Reconciles conflicting stage memories into permanent knowledge —
            // worth the deeper pass despite running on sonnet.
            StageType::KnowledgeDistill => "high",
            // The final quality gate, combining code review and functional
            // verification, gets the highest effort available.
            StageType::IntegrationVerify => "xhigh",
            // Ordinary orchestration: plan the work, delegate, verify.
            StageType::Standard => "high",
        }
    }
}
