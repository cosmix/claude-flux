//! Foreground execution mode for the orchestrator.

use anyhow::{bail, Context, Result};
use colored::Colorize;
use std::time::Duration;

use super::checks::prepare_repo_for_run;
use super::graph_loader::build_execution_graph;
use super::resolve_backend_flag;
use crate::commands::status::render::print_completion_summary;
use crate::daemon::collect_completion_summary;
use crate::fs::plan_lifecycle;
use crate::fs::work_dir::WorkDir;
use crate::orchestrator::{Orchestrator, OrchestratorConfig, OrchestratorResult};

/// Execute plan stages in foreground (for --foreground flag)
/// Usage: `loom run --foreground [--manual] [--max-parallel <n>] [--watch] [--no-merge] [--backend <native|tmux>]`
pub fn execute(
    manual: bool,
    max_parallel: Option<usize>,
    watch: bool,
    auto_merge: bool,
    backend: Option<String>,
) -> Result<()> {
    // Ensure git worktree prerequisites are met before starting.
    let repo_root = std::env::current_dir()?;
    prepare_repo_for_run(&repo_root)?;

    let work_dir = WorkDir::new(".")?;
    work_dir.load()?;

    super::plan_inputs::require_committed_plan(&work_dir)?;

    resolve_backend_flag(&work_dir, backend, "loom run --foreground")?;

    run_preflights(&work_dir)?;
    super::plan_inputs::mark_plan_in_progress(&work_dir)?;

    // Publish against the committed active filename and the revision stages inherit.
    super::checks::advisory_source_graph_preflight(&repo_root, &work_dir, false);

    crate::utils::print_logo_header("Run (foreground)");

    execute_foreground(manual, max_parallel, watch, auto_merge, &work_dir)
}

/// Startup preflights, in order: advisory Remote Control, the hard
/// sandbox-prerequisite check (like `require_jq`: a missing `bwrap`/`socat` or
/// WSL1 makes every session exit at startup), then the advisory codex lane.
fn run_preflights(work_dir: &WorkDir) -> Result<()> {
    if let Ok(claude_path) = crate::claude::find_claude_path() {
        crate::remote_control::run_startup_preflight(&claude_path, work_dir.root());
    }
    super::sandbox_preflight::require_sandbox_prerequisites(work_dir.root())?;
    super::checks::advisory_codex_lane_preflight(work_dir.root());
    Ok(())
}

/// Execute orchestrator in foreground mode (for debugging)
fn execute_foreground(
    manual: bool,
    max_parallel: Option<usize>,
    watch: bool,
    auto_merge: bool,
    work_dir: &WorkDir,
) -> Result<()> {
    let (graph, plan_sandbox) = build_execution_graph(work_dir)?;

    // Parse config.toml to extract base_branch
    let base_branch = crate::fs::parse_base_branch_from_config(work_dir.root())?;

    let config = OrchestratorConfig {
        max_parallel_sessions: max_parallel.unwrap_or(4),
        poll_interval: Duration::from_secs(5),
        manual_mode: manual,
        watch_mode: watch,
        work_dir: work_dir.root().to_path_buf(),
        repo_root: std::env::current_dir()?,
        status_update_interval: Duration::from_secs(30),
        auto_merge,
        base_branch,
        skills_dir: None, // Use default ~/.claude/skills/
        enable_skill_routing: true,
        max_skill_recommendations: 8,
        sandbox_config: plan_sandbox,
        shutdown_flag: None,
    };

    // Harmless even with no `orchestrator.pid` lock: the adopt-side liveness gate covers it.
    crate::fs::tmux_tmpdir::record_tmux_tmpdir_best_effort(work_dir.root());

    let mut orchestrator =
        Orchestrator::new(config, graph).context("Failed to create orchestrator")?;

    announce_run_mode(watch);
    let result = orchestrator.run()?;

    report_completion(work_dir, &result);

    // If successful, check if all stages are merged and mark plan as done
    if result.is_success() {
        plan_lifecycle::mark_plan_done_if_all_merged(work_dir)?;
        Ok(())
    } else {
        bail!("Orchestration completed with failures")
    }
}

fn report_completion(work_dir: &WorkDir, result: &OrchestratorResult) {
    // Collect and print the completion summary with timing and execution graph
    match collect_completion_summary(work_dir.root()) {
        Ok(summary) => {
            print_completion_summary(&summary);
        }
        Err(e) => {
            eprintln!("Warning: Failed to collect completion summary: {e}");
            // Fall back to basic result printing
            print_result(result);
        }
    }

    // Print additional details for stages that need attention
    print_needs_attention(result);
}

/// Print the startup banner for foreground mode: watch mode explains it will
/// keep running until Ctrl+C, one-shot mode just announces the run.
fn announce_run_mode(watch: bool) {
    if watch {
        println!(
            "{} Running in watch mode {}",
            "→".cyan().bold(),
            "(continuous execution)".dimmed()
        );
        println!("  {} Press {} to stop\n", "→".dimmed(), "Ctrl+C".bold());
    } else {
        println!("{} Running all ready stages...", "→".cyan().bold());
    }
}

/// Print orchestrator result summary (fallback for when completion summary fails)
fn print_result(result: &OrchestratorResult) {
    crate::utils::print_logo_header("Orchestration Complete");

    if !result.completed_stages.is_empty() {
        println!(
            "\n{} {}",
            "Completed".green().bold(),
            format!("({})", result.completed_stages.len()).dimmed()
        );
        println!("{}", "─".repeat(40).dimmed());
        for stage in &result.completed_stages {
            println!("  {} {}", "✓".green().bold(), stage);
        }
    }

    if !result.failed_stages.is_empty() {
        println!(
            "\n{} {}",
            "Failed".red().bold(),
            format!("({})", result.failed_stages.len()).dimmed()
        );
        println!("{}", "─".repeat(40).dimmed());
        for stage in &result.failed_stages {
            println!("  {} {}", "✗".red().bold(), stage);
        }
    }

    if !result.needs_handoff.is_empty() {
        println!(
            "\n{} {}",
            "Needs Handoff".yellow().bold(),
            format!("({})", result.needs_handoff.len()).dimmed()
        );
        println!("{}", "─".repeat(40).dimmed());
        for stage in &result.needs_handoff {
            println!("  {} {}", "⚠".yellow().bold(), stage);
        }
        println!(
            "\n  {} Run {} to continue",
            "→".dimmed(),
            "loom resume <stage-id>".cyan()
        );
    }

    println!();
    println!("{}", "═".repeat(40).dimmed());
    println!(
        "Sessions spawned: {}",
        result.total_sessions_spawned.to_string().bold()
    );

    if result.is_success() {
        println!(
            "\n{} All stages completed successfully!",
            "✓".green().bold()
        );
    }
}

/// Print additional details for stages that need attention (handoff/failures).
///
/// This supplements the completion summary with actionable information.
fn print_needs_attention(result: &OrchestratorResult) {
    if !result.needs_handoff.is_empty() {
        println!(
            "{} {}",
            "Needs Handoff".yellow().bold(),
            format!("({})", result.needs_handoff.len()).dimmed()
        );
        println!("{}", "─".repeat(40).dimmed());
        for stage in &result.needs_handoff {
            println!("  {} {}", "⚠".yellow().bold(), stage);
        }
        println!(
            "\n  {} Run {} to continue",
            "→".dimmed(),
            "loom resume <stage-id>".cyan()
        );
        println!();
    }
}
