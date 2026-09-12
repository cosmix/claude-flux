use anyhow::Result;
use chrono::{DateTime, Utc};

use crate::orchestrator::monitor::heartbeat::DEFAULT_HUNG_TIMEOUT_SECS;
use crate::plan::schema::{detect_stage_type, StageDefinition};

use super::types::{AcceptanceCriterion, Stage, StageOutput, StageStatus};

/// Maximum disputes a single stage may file before further requests
/// are refused (escalation goes through `NeedsHumanReview`).
const MAX_DISPUTES_PER_STAGE: u32 = 3;

impl Stage {
    pub fn new(name: String, description: Option<String>) -> Self {
        Self {
            id: Self::generate_id(&name),
            name,
            description,
            ..Self::default()
        }
    }

    /// Build the canonical runtime record for a parsed plan stage.
    ///
    /// Runtime-only fields retain [`Stage::new`] defaults. Every plan field
    /// represented by `Stage` is copied here so initialization and any recovery
    /// path cannot silently diverge as policy fields are added.
    pub fn from_definition(definition: &StageDefinition, plan_id: &str) -> Self {
        let mut stage = Self::new(definition.name.clone(), definition.description.clone());
        stage.id = definition.id.clone();
        stage.status = if definition.dependencies.is_empty() {
            StageStatus::Queued
        } else {
            StageStatus::WaitingForDeps
        };
        stage.dependencies = definition.dependencies.clone();
        stage.parallel_group = definition.parallel_group.clone();
        stage.acceptance = definition.acceptance.clone();
        stage.setup = definition.setup.clone();
        stage.files = definition.files.clone();
        stage.stage_type = detect_stage_type(definition);
        stage.plan_id = Some(plan_id.to_string());
        stage.auto_merge = definition.auto_merge;
        stage.working_dir = Some(definition.working_dir.clone());
        stage.context_ceiling_tokens = definition.context_ceiling_tokens;
        stage.plan_overview = definition.plan_overview;
        stage.artifacts = definition.artifacts.clone();
        stage.wiring = definition.wiring.clone();
        stage.wiring_tests = definition.wiring_tests.clone();
        stage.dead_code_check = definition.dead_code_check.clone();
        stage.before_stage = definition.before_stage.clone();
        stage.after_stage = definition.after_stage.clone();
        stage.sandbox = definition.sandbox.clone();
        stage.execution_mode = definition.execution_mode;
        stage.bug_fix = definition.bug_fix;
        stage.regression_test = definition.regression_test.clone();
        stage.model = definition.model.clone();
        stage.reasoning_effort = definition.reasoning_effort.clone();
        stage.code_review = definition.code_review.clone();
        stage.ultracode = definition.ultracode;
        stage.implementers = definition.implementers.clone();
        stage.subagent_timeout_secs = definition.subagent_timeout_secs;
        stage
    }

    /// The stage's own `model` if the plan set one, else the stage-type default.
    /// A launched session resolves further, through `crate::fs::work_dir::resolve_stage_model_effort`.
    pub fn effective_model(&self) -> &str {
        self.model
            .as_deref()
            .unwrap_or_else(|| self.stage_type.default_model())
    }

    /// Returns the effective reasoning effort for this stage.
    /// Uses the explicit override if set, otherwise falls back to the stage-type
    /// default, which varies per type — see `crate::fs::work_dir::resolve_stage_model_effort`.
    pub fn effective_reasoning_effort(&self) -> &str {
        if let Some(effort) = self.reasoning_effort.as_deref() {
            return effort;
        }
        self.stage_type.default_reasoning_effort()
    }

    /// Returns the effective subagent response budget for this stage, in seconds.
    /// Uses the plan's explicit override if set, otherwise the built-in default.
    ///
    /// This is the single resolution point for the budget — the orchestrator's
    /// heartbeat check and the signal generator both call it, so a stage can
    /// never be measured against one threshold while being told another.
    pub fn effective_subagent_timeout_secs(&self) -> u64 {
        self.subagent_timeout_secs
            .unwrap_or(DEFAULT_HUNG_TIMEOUT_SECS)
    }

    pub fn generate_id(name: &str) -> String {
        let timestamp = Utc::now().timestamp();
        format!(
            "stage-{}-{}",
            name.to_lowercase().replace(' ', "-"),
            timestamp
        )
    }

    pub fn add_dependency(&mut self, stage_id: String) {
        if !self.dependencies.contains(&stage_id) {
            self.dependencies.push(stage_id);
            self.updated_at = Utc::now();
        }
    }

    pub fn add_acceptance_criterion(&mut self, criterion: AcceptanceCriterion) {
        self.acceptance.push(criterion);
        self.updated_at = Utc::now();
    }

    pub fn add_file_pattern(&mut self, pattern: String) {
        if !self.files.contains(&pattern) {
            self.files.push(pattern);
            self.updated_at = Utc::now();
        }
    }

    pub fn set_worktree(&mut self, worktree_id: Option<String>) {
        self.worktree = worktree_id;
        self.updated_at = Utc::now();
    }

    pub fn set_resolved_base(&mut self, base: Option<String>) {
        self.resolved_base = base;
        self.updated_at = Utc::now();
    }

    pub fn assign_session(&mut self, session_id: String) {
        self.session = Some(session_id);
        self.updated_at = Utc::now();
    }

    pub fn release_session(&mut self) {
        self.session = None;
        self.updated_at = Utc::now();
    }

    /// Attempt to transition the stage to a new status with validation.
    ///
    /// This is the primary method for changing stage status. It validates
    /// that the transition is allowed before applying it.
    ///
    /// # Arguments
    /// * `new_status` - The target status to transition to
    ///
    /// # Returns
    /// `Ok(())` if the transition succeeded, `Err` if the transition is invalid
    pub fn try_transition(&mut self, new_status: StageStatus) -> Result<()> {
        let validated_status = self.status.try_transition(new_status)?;
        self.status = validated_status;
        self.updated_at = Utc::now();
        Ok(())
    }

    /// Force the stage into `status`, bypassing transition validation.
    ///
    /// This is the **single sanctioned** direct-assignment API. Every place that
    /// previously caught a `try_transition` error and assigned `stage.status`
    /// directly should route through here so the bypass is uniform and visible.
    ///
    /// A forced assignment means the transition was deemed illegal by the state
    /// machine yet applied anyway — usually because a recovery/failure path knows
    /// better than the table, or papers over a caller whose mental model is wrong.
    /// Either way it is noteworthy, so it logs at `tracing::error!` with the stage
    /// id, the from/to statuses, and the caller-supplied `reason`. Prefer adding a
    /// legal edge to `can_transition_to` (and using `try_mark_*`) over forcing.
    ///
    /// # Arguments
    /// * `status` - The status to assign unconditionally
    /// * `reason` - Why the forced assignment is being made (logged)
    pub fn force_status_with_reason(&mut self, status: StageStatus, reason: &str) {
        let from = self.status.clone();
        tracing::error!(
            stage_id = %self.id,
            from = %from,
            to = %status,
            reason = %reason,
            "Forced stage status assignment bypassing transition validation"
        );
        self.status = status;
        self.updated_at = Utc::now();
    }

    /// Stamp `completed_at` now and `duration_secs` from `started_at`.
    fn stamp_completed(&mut self) {
        let now = Utc::now();
        self.completed_at = Some(now);
        if let Some(start) = self.started_at {
            self.duration_secs = Some(now.signed_duration_since(start).num_seconds());
        }
    }

    /// Complete the stage with validation.
    ///
    /// Computes and stores `duration_secs` from `started_at` to completion.
    ///
    /// # Returns
    /// `Ok(())` if the transition succeeded, `Err` if invalid
    pub fn try_complete(&mut self, reason: Option<String>) -> Result<()> {
        self.try_transition(StageStatus::Completed)?;
        self.close_reason = reason;
        self.stamp_completed();
        Ok(())
    }

    /// Mark the stage as needing handoff with validation.
    ///
    /// # Returns
    /// `Ok(())` if the transition succeeded, `Err` if invalid
    pub fn try_mark_needs_handoff(&mut self) -> Result<()> {
        self.try_transition(StageStatus::NeedsHandoff)
    }

    /// Mark the stage as queued with validation.
    ///
    /// # Returns
    /// `Ok(())` if the transition succeeded, `Err` if invalid
    pub fn try_mark_queued(&mut self) -> Result<()> {
        self.try_transition(StageStatus::Queued)
    }

    /// Mark the stage as executing with validation.
    ///
    /// Sets `started_at` timestamp if not already set (preserves original
    /// start time across retries).
    ///
    /// # Returns
    /// `Ok(())` if the transition succeeded, `Err` if invalid
    pub fn try_mark_executing(&mut self) -> Result<()> {
        self.try_transition(StageStatus::Executing)?;
        // Only set started_at on first execution (preserve across retries)
        if self.started_at.is_none() {
            self.started_at = Some(Utc::now());
        }
        Ok(())
    }

    /// Mark the stage as waiting for input with validation.
    ///
    /// # Returns
    /// `Ok(())` if the transition succeeded, `Err` if invalid
    pub fn try_mark_waiting_for_input(&mut self) -> Result<()> {
        self.try_transition(StageStatus::WaitingForInput)
    }

    /// Mark the stage as blocked with validation.
    ///
    /// # Returns
    /// `Ok(())` if the transition succeeded, `Err` if invalid
    pub fn try_mark_blocked(&mut self) -> Result<()> {
        self.try_transition(StageStatus::Blocked)
    }

    /// Mark the stage as skipped with validation.
    ///
    /// # Returns
    /// `Ok(())` if the transition succeeded, `Err` if invalid
    pub fn try_skip(&mut self, reason: Option<String>) -> Result<()> {
        self.try_transition(StageStatus::Skipped)?;
        self.close_reason = reason;
        Ok(())
    }

    /// Mark the stage as having merge conflicts.
    ///
    /// This sets both the status to MergeConflict and the merge_conflict flag.
    /// The stage work is complete but cannot be merged due to conflicts.
    ///
    /// # Returns
    /// `Ok(())` if the transition succeeded, `Err` if invalid
    pub fn try_mark_merge_conflict(&mut self) -> Result<()> {
        self.try_transition(StageStatus::MergeConflict)?;
        self.merge_conflict = true;
        Ok(())
    }

    /// Complete the merge: clear `merge_conflict` and set `merged`. Unless the stage is
    /// already `Completed` (auto-merge disabled leaves it there, and `loom stage merge`
    /// then runs against it), also transition to `Completed` and stamp the timestamps.
    ///
    /// # Returns
    /// `Ok(())` if the transition succeeded, `Err` if invalid
    pub fn try_complete_merge(&mut self) -> Result<()> {
        if self.status != StageStatus::Completed {
            self.try_transition(StageStatus::Completed)?;
            self.stamp_completed();
        }
        self.merge_conflict = false;
        self.merged = true;
        Ok(())
    }

    /// Mark the stage as completed with failures (acceptance criteria failed).
    ///
    /// This indicates the stage finished executing but acceptance criteria failed.
    /// The stage can be retried by transitioning back to Executing.
    ///
    /// # Returns
    /// `Ok(())` if the transition succeeded, `Err` if invalid
    pub fn try_complete_with_failures(&mut self) -> Result<()> {
        self.try_transition(StageStatus::CompletedWithFailures)
    }

    /// Mark the stage as merge blocked (merge failed with actual error, not conflicts).
    ///
    /// This indicates the merge operation failed due to an error (not conflicts).
    /// The stage can be retried by transitioning back to Executing.
    ///
    /// # Returns
    /// `Ok(())` if the transition succeeded, `Err` if invalid
    pub fn try_mark_merge_blocked(&mut self) -> Result<()> {
        self.try_transition(StageStatus::MergeBlocked)
    }

    /// Request human review for this stage.
    ///
    /// Transitions from Executing to NeedsHumanReview and records the reason.
    ///
    /// # Arguments
    /// * `reason` - Why the stage needs human review
    ///
    /// # Returns
    /// `Ok(())` if the transition succeeded, `Err` if invalid
    pub fn try_request_human_review(&mut self, reason: String) -> Result<()> {
        self.try_transition(StageStatus::NeedsHumanReview)?;
        self.review_reason = Some(reason);
        Ok(())
    }

    /// Approve human review and queue a fresh session.
    ///
    /// Transitions from NeedsHumanReview to Queued: the escalating agent's session is already gone.
    ///
    /// # Returns
    /// `Ok(())` if the transition succeeded, `Err` if invalid
    pub fn try_approve_review(&mut self) -> Result<()> {
        self.try_transition(StageStatus::Queued)?;
        self.review_reason = None;
        Ok(())
    }

    /// Reject human review and block the stage.
    ///
    /// Transitions from NeedsHumanReview to Blocked and updates the review reason.
    ///
    /// # Arguments
    /// * `reason` - Why the review was rejected
    ///
    /// # Returns
    /// `Ok(())` if the transition succeeded, `Err` if invalid
    pub fn try_reject_review(&mut self, reason: String) -> Result<()> {
        self.try_transition(StageStatus::Blocked)?;
        self.review_reason = Some(reason);
        Ok(())
    }

    /// Request adjudication for this stage's acceptance criterion.
    ///
    /// Transitions to NeedsAdjudication. From `Executing` this is a
    /// direct transition; from `CompletedWithFailures` the stage steps
    /// through `Executing` first (the same two-step pattern the
    /// pre-Stage 2 `dispute_criteria` used for human review).
    ///
    /// `reason` is recorded in `review_reason` for now — the
    /// authoritative copy lives in `request.md` written by the daemon.
    pub fn try_request_adjudication(&mut self, reason: Option<String>) -> Result<()> {
        match self.status {
            StageStatus::Executing => {
                self.try_transition(StageStatus::NeedsAdjudication)?;
            }
            StageStatus::CompletedWithFailures => {
                // Two-step: CompletedWithFailures → Executing →
                // NeedsAdjudication, mirroring the existing dispute path.
                self.try_transition(StageStatus::Executing)?;
                self.try_transition(StageStatus::NeedsAdjudication)?;
            }
            _ => {
                // Defer to the state machine for the bail message.
                self.try_transition(StageStatus::NeedsAdjudication)?;
            }
        }
        if let Some(r) = reason {
            self.review_reason = Some(r);
        }
        Ok(())
    }

    /// Maximum number of disputes a stage may file before further
    /// dispute requests are refused. See
    /// `dispute_budget_exhausted`.
    pub fn max_disputes_per_stage(&self) -> u32 {
        MAX_DISPUTES_PER_STAGE
    }

    /// True when `dispute_count` has reached `max_disputes_per_stage`.
    pub fn dispute_budget_exhausted(&self) -> bool {
        self.dispute_count >= self.max_disputes_per_stage()
    }

    /// Increment the fix attempt counter and return the new count.
    pub fn increment_fix_attempts(&mut self) -> u32 {
        self.fix_attempts += 1;
        self.updated_at = Utc::now();
        self.fix_attempts
    }

    /// Check if the stage has reached its fix attempt limit.
    pub fn is_at_fix_limit(&self) -> bool {
        self.fix_attempts >= self.get_effective_max_fix_attempts()
    }

    /// Get the effective maximum fix attempts (default 3 if not set).
    pub fn get_effective_max_fix_attempts(&self) -> u32 {
        self.max_fix_attempts.unwrap_or(3)
    }

    pub fn hold(&mut self) {
        if !self.held {
            self.held = true;
            self.updated_at = Utc::now();
        }
    }

    pub fn release(&mut self) {
        if self.held {
            self.held = false;
            self.updated_at = Utc::now();
        }
    }

    /// Add or update an output for this stage.
    ///
    /// If an output with the same key already exists, it will be replaced.
    ///
    /// # Arguments
    /// * `output` - The output to add or update
    ///
    /// # Returns
    /// `true` if the output was added, `false` if it replaced an existing output
    pub fn set_output(&mut self, output: StageOutput) -> bool {
        self.updated_at = Utc::now();

        // Check if an output with this key already exists
        if let Some(existing) = self.outputs.iter_mut().find(|o| o.key == output.key) {
            *existing = output;
            false
        } else {
            self.outputs.push(output);
            true
        }
    }

    /// Get an output by key.
    ///
    /// # Arguments
    /// * `key` - The key of the output to retrieve
    ///
    /// # Returns
    /// The output if found, None otherwise
    pub fn get_output(&self, key: &str) -> Option<&StageOutput> {
        self.outputs.iter().find(|o| o.key == key)
    }

    /// Remove an output by key.
    ///
    /// # Arguments
    /// * `key` - The key of the output to remove
    ///
    /// # Returns
    /// `true` if an output was removed, `false` if no output with that key existed
    pub fn remove_output(&mut self, key: &str) -> bool {
        let len_before = self.outputs.len();
        self.outputs.retain(|o| o.key != key);
        if self.outputs.len() != len_before {
            self.updated_at = Utc::now();
            true
        } else {
            false
        }
    }

    /// Check if an output key already exists in this stage.
    ///
    /// # Arguments
    /// * `key` - The key to check
    ///
    /// # Returns
    /// `true` if the key exists, `false` otherwise
    pub fn has_output(&self, key: &str) -> bool {
        self.outputs.iter().any(|o| o.key == key)
    }

    /// Check if this stage has any goal-backward verification checks defined.
    pub fn has_any_goal_checks(&self) -> bool {
        !self.artifacts.is_empty()
            || !self.wiring.is_empty()
            || !self.wiring_tests.is_empty()
            || self.dead_code_check.is_some()
    }

    /// Begin a new execution attempt.
    ///
    /// Sets `attempt_started_at` to the given timestamp and initializes
    /// `execution_secs` to 0 if not already set.
    pub fn begin_attempt(&mut self, now: DateTime<Utc>) {
        self.attempt_started_at = Some(now);
        if self.execution_secs.is_none() {
            self.execution_secs = Some(0);
        }
    }

    /// Accumulate time from the current execution attempt.
    ///
    /// Calculates elapsed time since `attempt_started_at`, adds it to
    /// `execution_secs`, and clears `attempt_started_at`.
    /// No-op if `attempt_started_at` is None.
    pub fn accumulate_attempt_time(&mut self, now: DateTime<Utc>) {
        if let Some(start) = self.attempt_started_at.take() {
            let elapsed = now.signed_duration_since(start).num_seconds().max(0);
            let current = self.execution_secs.unwrap_or(0);
            self.execution_secs = Some(current.saturating_add(elapsed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::stage::{
        CommandConfinement, DeadCodeCheck, ExecutionMode, Implementer, Implementers,
        PermissionMode, RegressionTest, StageSandboxConfig, StageType, SuccessCriteria, TruthCheck,
        WiringCheck, WiringTest,
    };
    use crate::plan::schema::CodeReviewConfig;
    use chrono::{Duration, Utc};

    #[test]
    fn from_definition_copies_all_runtime_policy_fields() {
        let definition = StageDefinition {
            id: "policy-stage".to_string(),
            name: "Policy Stage".to_string(),
            description: Some("full conversion".to_string()),
            dependencies: vec!["bootstrap".to_string()],
            parallel_group: Some("policy".to_string()),
            acceptance: vec![AcceptanceCriterion::Simple("cargo test".to_string())],
            setup: vec!["cargo build".to_string()],
            files: vec!["src/policy.rs".to_string()],
            auto_merge: Some(true),
            working_dir: "loom".to_string(),
            stage_type: Some(StageType::IntegrationVerify),
            artifacts: vec!["target/policy".to_string()],
            wiring: vec![WiringCheck {
                source: "src/lib.rs".to_string(),
                pattern: "policy".to_string(),
                description: "policy is exported".to_string(),
            }],
            wiring_tests: vec![WiringTest {
                name: "runtime policy".to_string(),
                command: "cargo test policy".to_string(),
                success_criteria: SuccessCriteria {
                    exit_code: Some(0),
                    ..SuccessCriteria::default()
                },
                description: Some("runtime wiring".to_string()),
            }],
            dead_code_check: Some(DeadCodeCheck {
                command: "cargo check".to_string(),
                fail_patterns: vec!["unused".to_string()],
                ignore_patterns: vec!["fixture".to_string()],
            }),
            before_stage: vec![TruthCheck {
                command: "test ! -e target/policy".to_string(),
                stdout_contains: vec![],
                stdout_not_contains: vec![],
                stderr_empty: Some(true),
                exit_code: Some(0),
                description: Some("absent before".to_string()),
            }],
            after_stage: vec![TruthCheck {
                command: "test -e target/policy".to_string(),
                stdout_contains: vec![],
                stdout_not_contains: vec![],
                stderr_empty: Some(true),
                exit_code: Some(0),
                description: Some("present after".to_string()),
            }],
            context_ceiling_tokens: Some(71),
            removed_context_budget: None,
            plan_overview: Some(true),
            sandbox: StageSandboxConfig {
                enabled: Some(true),
                auto_allow: Some(false),
                allow_unsandboxed_escape: Some(false),
                excluded_commands: vec!["danger".to_string()],
                filesystem: None,
                network: None,
                linux: None,
                permission_mode: Some(PermissionMode::Plan),
                command_confinement: Some(CommandConfinement::Inherit),
            },
            execution_mode: Some(ExecutionMode::Team),
            bug_fix: Some(true),
            regression_test: Some(RegressionTest {
                file: "tests/policy.rs".to_string(),
                must_contain: vec!["regression".to_string()],
            }),
            model: Some("opus".to_string()),
            reasoning_effort: Some("xhigh".to_string()),
            code_review: Some(CodeReviewConfig {
                dimensions: vec!["security".to_string(), "wiring".to_string()],
                require_all: true,
            }),
            ultracode: true,
            implementers: Implementers::new(vec![Implementer::Codex, Implementer::Claude]),
            subagent_timeout_secs: Some(900),
        };

        let stage = Stage::from_definition(&definition, "plan-policy");

        assert_eq!(stage.id, definition.id);
        assert_eq!(stage.name, definition.name);
        assert_eq!(stage.description, definition.description);
        assert_eq!(stage.status, StageStatus::WaitingForDeps);
        assert_eq!(stage.dependencies, definition.dependencies);
        assert_eq!(stage.parallel_group, definition.parallel_group);
        assert_eq!(stage.acceptance, definition.acceptance);
        assert_eq!(stage.setup, definition.setup);
        assert_eq!(stage.files, definition.files);
        assert_eq!(stage.stage_type, StageType::IntegrationVerify);
        assert_eq!(stage.plan_id.as_deref(), Some("plan-policy"));
        assert_eq!(stage.auto_merge, Some(true));
        assert_eq!(stage.working_dir.as_deref(), Some("loom"));
        assert_eq!(stage.context_ceiling_tokens, Some(71));
        assert_eq!(stage.plan_overview, Some(true));
        assert_eq!(stage.artifacts, definition.artifacts);
        assert_eq!(stage.wiring[0].source, "src/lib.rs");
        assert_eq!(stage.wiring_tests[0].command, "cargo test policy");
        assert_eq!(
            stage.dead_code_check.as_ref().unwrap().command,
            "cargo check"
        );
        assert_eq!(stage.before_stage[0].exit_code, Some(0));
        assert_eq!(stage.after_stage[0].exit_code, Some(0));
        assert_eq!(stage.sandbox.enabled, Some(true));
        assert_eq!(stage.sandbox.permission_mode, Some(PermissionMode::Plan));
        assert_eq!(
            stage.sandbox.command_confinement,
            Some(CommandConfinement::Inherit)
        );
        assert_eq!(stage.execution_mode, Some(ExecutionMode::Team));
        assert_eq!(stage.bug_fix, Some(true));
        assert_eq!(
            stage.regression_test.as_ref().unwrap().file,
            "tests/policy.rs"
        );
        assert_eq!(stage.model.as_deref(), Some("opus"));
        assert_eq!(stage.reasoning_effort.as_deref(), Some("xhigh"));
        assert_eq!(
            stage.code_review.as_ref().unwrap().dimensions,
            vec!["security".to_string(), "wiring".to_string()]
        );
        assert!(stage.ultracode);
        assert_eq!(stage.implementers.preferred(), Implementer::Codex);
        assert_eq!(stage.subagent_timeout_secs, Some(900));
    }

    #[test]
    fn test_begin_attempt_initializes_execution_secs() {
        let mut stage = Stage::default();
        assert!(stage.execution_secs.is_none());
        assert!(stage.attempt_started_at.is_none());

        let now = Utc::now();
        stage.begin_attempt(now);

        assert_eq!(stage.execution_secs, Some(0));
        assert_eq!(stage.attempt_started_at, Some(now));
    }

    #[test]
    fn test_begin_attempt_preserves_existing_execution_secs() {
        let mut stage = Stage {
            execution_secs: Some(60),
            ..Stage::default()
        };

        let now = Utc::now();
        stage.begin_attempt(now);

        assert_eq!(stage.execution_secs, Some(60)); // Preserved
        assert_eq!(stage.attempt_started_at, Some(now));
    }

    #[test]
    fn test_accumulate_attempt_time() {
        let mut stage = Stage::default();
        let start = Utc::now() - Duration::seconds(30);
        stage.attempt_started_at = Some(start);
        stage.execution_secs = Some(0);

        stage.accumulate_attempt_time(Utc::now());

        assert!(stage.execution_secs.unwrap() >= 29); // Allow for timing variance
        assert!(stage.attempt_started_at.is_none()); // Cleared
    }

    #[test]
    fn test_accumulate_attempt_time_adds_to_existing() {
        let start = Utc::now() - Duration::seconds(50);
        let mut stage = Stage {
            execution_secs: Some(100),
            attempt_started_at: Some(start),
            ..Stage::default()
        };

        stage.accumulate_attempt_time(Utc::now());

        assert!(stage.execution_secs.unwrap() >= 149); // 100 + ~50
        assert!(stage.attempt_started_at.is_none());
    }

    #[test]
    fn test_accumulate_attempt_time_noop_without_started() {
        let mut stage = Stage {
            execution_secs: Some(100),
            ..Stage::default()
        };
        // attempt_started_at is None

        stage.accumulate_attempt_time(Utc::now());

        assert_eq!(stage.execution_secs, Some(100)); // Unchanged
    }
}
