//! Core orchestrator for coordinating stage execution
//!
//! The orchestrator is the heart of `loom run`. It:
//! - Creates worktrees for ready stages
//! - Spawns Claude sessions in terminal windows
//! - Monitors stage completion and session health
//! - Handles crashes and context exhaustion
//! - Manages the execution graph

use std::io::{self, Write};

mod coherence;
mod completion_handler;
mod crash_classification;
mod crash_handler;
mod event_handler;
mod heartbeat_apply;
mod judge_close;
mod merge_handler;
mod orchestrator;
mod orphan_adoption;
mod persistence;
mod recovery;
mod run;
mod run_result;
mod session_adoption;
mod session_lifecycle;
mod spawn_setup;
mod spool_drain;
mod stage_executor;
mod stage_handoff;
mod stage_telemetry;
mod verdict_apply;

pub use orchestrator::{Orchestrator, OrchestratorConfig, OrchestratorResult};

/// Clear the current line (status line) before printing a message.
/// This prevents output from being mangled when the status line is being updated.
pub(super) fn clear_status_line() {
    // \r moves cursor to start of line, \x1B[K clears from cursor to end of line
    print!("\r\x1B[K");
    let _ = io::stdout().flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::frontmatter::extract_yaml_frontmatter;
    use crate::plan::schema::{Implementers, SandboxConfig, StageDefinition, StageSandboxConfig};
    use crate::plan::ExecutionGraph;
    use std::path::PathBuf;
    use std::time::Duration;

    fn create_test_config() -> OrchestratorConfig {
        OrchestratorConfig {
            max_parallel_sessions: 2,
            poll_interval: Duration::from_millis(100),
            manual_mode: true,
            watch_mode: false,
            work_dir: PathBuf::from("/tmp/test-work"),
            repo_root: PathBuf::from("/tmp/test-repo"),
            status_update_interval: Duration::from_secs(30),
            auto_merge: false,
            base_branch: None,
            skills_dir: None,
            enable_skill_routing: false, // Disable for tests
            max_skill_recommendations: 8,
            sandbox_config: SandboxConfig::default(),
            shutdown_flag: None,
        }
    }

    fn create_simple_graph() -> ExecutionGraph {
        let stages = vec![StageDefinition {
            id: "stage-1".to_string(),
            name: "Stage 1".to_string(),
            description: None,
            dependencies: vec![],
            parallel_group: None,
            acceptance: vec![],
            setup: vec![],
            files: vec![],
            auto_merge: None,
            working_dir: ".".to_string(),
            stage_type: None,
            artifacts: vec![],
            wiring: vec![],
            wiring_tests: vec![],
            dead_code_check: None,
            before_stage: vec![],
            after_stage: vec![],
            context_ceiling_tokens: None,
            removed_context_budget: None,
            plan_overview: None,
            sandbox: StageSandboxConfig::default(),
            execution_mode: None,
            bug_fix: None,
            regression_test: None,
            model: None,
            reasoning_effort: None,
            code_review: None,
            ultracode: false,
            implementers: Implementers::default(),
            subagent_timeout_secs: None,
        }];

        ExecutionGraph::build(stages).unwrap()
    }

    #[test]
    fn test_orchestrator_config_default() {
        let config = OrchestratorConfig::default();
        assert_eq!(config.max_parallel_sessions, 4);
        assert_eq!(config.poll_interval, Duration::from_secs(5));
        assert!(!config.manual_mode);
        assert!(!config.watch_mode);
        assert!(config.auto_merge);
    }

    #[test]
    fn test_orchestrator_result_success() {
        let result = OrchestratorResult {
            completed_stages: vec!["stage-1".to_string()],
            failed_stages: vec![],
            needs_handoff: vec![],
            total_sessions_spawned: 1,
            started_at: chrono::Utc::now(),
            completed_at: chrono::Utc::now(),
        };

        assert!(result.is_success());
    }

    #[test]
    fn test_orchestrator_result_failure() {
        let result = OrchestratorResult {
            completed_stages: vec![],
            failed_stages: vec!["stage-1".to_string()],
            needs_handoff: vec![],
            total_sessions_spawned: 1,
            started_at: chrono::Utc::now(),
            completed_at: chrono::Utc::now(),
        };

        assert!(!result.is_success());
    }

    #[test]
    fn test_orchestrator_result_needs_handoff() {
        let result = OrchestratorResult {
            completed_stages: vec![],
            failed_stages: vec![],
            needs_handoff: vec!["stage-1".to_string()],
            total_sessions_spawned: 1,
            started_at: chrono::Utc::now(),
            completed_at: chrono::Utc::now(),
        };

        assert!(!result.is_success());
    }

    #[test]
    fn sandbox_settings_write_failure_is_fatal_to_spawn_setup() {
        let target = tempfile::tempdir().unwrap();
        std::fs::write(target.path().join(".claude"), "not a directory").unwrap();
        let config = crate::sandbox::merge_config(
            &SandboxConfig::default(),
            &StageSandboxConfig::default(),
            crate::plan::schema::StageType::Standard,
            &Implementers::default(),
        );

        let error = stage_executor::write_required_sandbox_settings(
            &config,
            target.path(),
            "sandbox-failure",
        )
        .expect_err("sandbox settings failure must abort spawn setup");

        assert!(error
            .to_string()
            .contains("Failed to enforce sandbox settings"));
    }

    #[test]
    fn install_required_hooks_rejects_missing_hooks_dir() {
        let target = tempfile::tempdir().unwrap();
        let work_dir = tempfile::tempdir().unwrap();

        let error = stage_executor::install_required_hooks(
            None,
            target.path(),
            work_dir.path(),
            crate::plan::schema::PermissionMode::Default,
            "missing-hooks",
        )
        .expect_err("missing hooks directory must abort spawn setup");

        assert!(error.to_string().contains("hooks directory not found"));
    }

    #[test]
    fn install_required_hooks_failure_is_fatal_to_spawn_setup() {
        let target = tempfile::tempdir().unwrap();
        let work_dir = tempfile::tempdir().unwrap();
        // Block `.claude` directory creation the same way the sandbox-settings
        // test above blocks `write_settings`: put a file where a directory
        // needs to go.
        std::fs::write(target.path().join(".claude"), "not a directory").unwrap();

        let error = stage_executor::install_required_hooks(
            Some(PathBuf::from("/nonexistent/hooks/dir")),
            target.path(),
            work_dir.path(),
            crate::plan::schema::PermissionMode::Default,
            "hook-install-failure",
        )
        .expect_err("hook installation failure must abort spawn setup");

        assert!(error
            .to_string()
            .contains("Failed to install Claude Code hooks"));
    }

    #[test]
    #[ignore] // Requires a terminal emulator - skipped in CI
    fn test_running_session_count() {
        let config = create_test_config();
        let graph = create_simple_graph();
        let orchestrator = Orchestrator::new(config, graph).expect("Failed to create orchestrator");

        assert_eq!(orchestrator.running_session_count(), 0);
    }

    #[test]
    fn test_extract_yaml_frontmatter() {
        let content = r#"---
id: stage-1
name: Test Stage
status: Pending
---

# Stage Details
Test content
"#;

        let result = extract_yaml_frontmatter(content);
        assert!(result.is_ok());

        let value = result.unwrap();
        assert!(value.get("id").is_some());
        assert!(value.get("name").is_some());
    }

    #[test]
    fn test_extract_yaml_frontmatter_no_delimiter() {
        let content = "No frontmatter here";
        let result = extract_yaml_frontmatter(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_yaml_frontmatter_not_closed() {
        let content = r#"---
id: stage-1
name: Test Stage
"#;
        let result = extract_yaml_frontmatter(content);
        assert!(result.is_err());
    }
}
