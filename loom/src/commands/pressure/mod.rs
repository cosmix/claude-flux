//! `loom pressure` — alternating Claude/Codex plan pressure-testing driver.
//!
//! Each round runs two independent pressure-tests **concurrently**: Claude
//! `/pressure` in the foreground (interactive → subscription billing, the user
//! watches it) and Codex `$pressure` in the background (its noisy event stream
//! captured to a log file). Once both finish, Claude `/address` folds Codex's
//! written review back into the plan. The Codex report is deleted at the start
//! of every round so a failed Codex write can never leave `/address` reading a
//! stale review, plus once more after all rounds as cleanup.
//!
//! ## Why Claude runs in the foreground (and how it auto-exits)
//!
//! Claude Code enters its non-interactive (`-p`) path — which can bill against
//! pay-per-token API credits instead of the subscription — whenever stdout is
//! not a TTY. So Claude's stdout MUST stay the real terminal; it cannot be
//! captured or backgrounded. Interactive Claude also never exits on its own
//! after a slash command. We therefore mirror how the loom daemon terminates a
//! session: the agent signals completion (here, by creating a marker file as
//! its final action, injected via `--append-system-prompt`), the driver watches
//! for that marker, and then SIGTERMs the now-idle session. If the marker never
//! appears the user can still exit manually, exactly as before.
//!
//! The marker lives under `<repo>/.loom/work/pressure/`, NOT `std::env::temp_dir()`:
//! Claude is spawned with `--permission-mode auto`, which sandboxes its Bash
//! tool with `/tmp` mounted read-only, so a temp-dir marker could never be
//! created and the driver would poll forever. The repo working tree is the
//! sandbox's writable root (the child's cwd is `repo_root`), so the marker is
//! re-homed there instead.

use anyhow::Result;
use colored::Colorize;
use std::path::{Path, PathBuf};

use crate::claude::find_claude_path;
use crate::cli::types_pressure::PressureArgs;
use crate::codex::find_codex_path;
use crate::user_config::UserConfig;

mod models;
mod paths;
mod spawn;

use models::PressureModels;
use paths::{
    claude_marker_path, codex_log_path, codex_report_path, delete_file, resolve_plan_path,
    resolve_repo_root,
};
use spawn::{
    claude_args, claude_should_stop, codex_args, run_claude_foreground, should_stop,
    spawn_codex_background, wait_codex, AGENT_TEAMS_ENV,
};

/// One step in the pressure pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Step {
    /// Delete the codex report file if it exists.
    DeleteReport(PathBuf),
    /// Run the two independent pressure-tests concurrently: Claude `/pressure`
    /// in the foreground and Codex `$pressure` in the background.
    Pressure {
        /// Full positional slash invocation, e.g. `/pressure doc/plans/PLAN-foo.md`.
        claude: String,
        /// Full positional skill invocation, e.g. `$pressure doc/plans/PLAN-foo.md`.
        codex: String,
    },
    /// Run Claude `/address` in the foreground to fold the review into the plan.
    Address(String),
}

/// Build the ordered list of steps for `rounds` rounds.
///
/// Each round: delete the report (so a failed Codex write can't leave
/// `/address` reading the previous round's report) → run Claude `/pressure` and
/// Codex `$pressure` concurrently → Claude `/address`. After all rounds, one
/// final report deletion as cleanup.
pub(super) fn plan_steps(rounds: u32, invocation: &str, report: &Path) -> Vec<Step> {
    let mut steps = Vec::new();
    for _ in 0..rounds {
        steps.push(Step::DeleteReport(report.to_path_buf()));
        steps.push(Step::Pressure {
            claude: format!("/pressure {invocation}"),
            codex: format!("$pressure {invocation}"),
        });
        steps.push(Step::Address(format!("/address {invocation}")));
    }
    steps.push(Step::DeleteReport(report.to_path_buf()));
    steps
}

/// Render the exact commands `--dry-run` would execute.
///
/// Uses the same `claude_args`/`codex_args` builders as the real spawns, so
/// the preview can never diverge from what actually runs.
fn render_dry_run(
    rounds: u32,
    invocation: &str,
    report: &Path,
    repo_root: &Path,
    marker: &Path,
    codex_log: &Path,
    models: &PressureModels,
) -> String {
    let mut out = format!(
        "Dry run: {rounds} round(s) of pressure-testing for {invocation}\n\
         Codex report:            {}\n\
         Codex log (captured):    {}\n\
         Claude auto-close marker: {}\n\
         Models:                  claude={}  codex={}  address={}\n\n",
        report.display(),
        codex_log.display(),
        marker.display(),
        models.claude,
        models.codex,
        models.address
    );
    for (n, step) in plan_steps(rounds, invocation, report)
        .into_iter()
        .enumerate()
    {
        render_dry_run_step(&mut out, n + 1, step, repo_root, marker, models);
    }
    out
}

/// Render the dry-run preview lines for one step (numbered `n`), appending to
/// `out`. Split out of [`render_dry_run`] purely to keep that function under
/// the maintainability line limit.
fn render_dry_run_step(
    out: &mut String,
    n: usize,
    step: Step,
    repo_root: &Path,
    marker: &Path,
    models: &PressureModels,
) {
    match step {
        Step::DeleteReport(p) => {
            out.push_str(&format!("  {n}. delete report {}\n", p.display()));
        }
        Step::Pressure { claude, codex } => {
            out.push_str(&format!(
                "  {n}. [parallel] Claude (foreground) + Codex (background → log):\n"
            ));
            out.push_str(&format!(
                "       {AGENT_TEAMS_ENV}=1 claude {}\n",
                claude_args(&claude, marker, &models.claude).join(" ")
            ));
            out.push_str(&format!(
                "       codex {}\n",
                codex_args(repo_root, &codex, &models.codex).join(" ")
            ));
        }
        Step::Address(slash) => {
            out.push_str(&format!(
                "  {n}. {AGENT_TEAMS_ENV}=1 claude {}\n",
                claude_args(&slash, marker, &models.address).join(" ")
            ));
        }
    }
}

/// Everything a pipeline step needs to run, built once in [`execute`] and
/// passed by reference so the per-step helpers below take a single argument
/// each instead of accumulating a long parameter list as the pipeline grows.
struct StepContext<'a> {
    claude_path: &'a Path,
    codex_path: &'a Path,
    repo_root: &'a Path,
    marker: &'a Path,
    codex_log: &'a Path,
    report: &'a Path,
    models: &'a PressureModels,
}

/// Print the pressure run's header: round count and target, then the
/// per-role model selection. Split out of [`execute`] purely to keep that
/// function under the maintainability line limit.
fn print_run_header(rounds: u32, invocation: &str, models: &PressureModels) {
    println!(
        "{} {} round(s) on {}",
        "→".cyan().bold(),
        rounds,
        invocation.cyan()
    );
    println!(
        "{} models: claude={}  codex={}  address={}\n",
        "→".cyan().bold(),
        models.claude,
        models.codex,
        models.address
    );
}

/// Run the concurrent Claude/Codex pressure-test step: Codex reviews the plan
/// independently in the background (quiet, captured to a log) while Claude
/// pressure-tests in the foreground (interactive → subscription billing).
/// Returns whether the pipeline should stop. Split out of [`execute`]'s
/// `Step::Pressure` arm purely to keep that function under the
/// maintainability line limit.
fn run_pressure_step(ctx: &StepContext, claude: &str, codex: &str) -> Result<bool> {
    let codex_child = spawn_codex_background(
        ctx.codex_path,
        ctx.repo_root,
        codex,
        ctx.codex_log,
        &ctx.models.codex,
    )?;
    println!(
        "{} codex review started in background (log: {})",
        "→".cyan().bold(),
        ctx.codex_log.display()
    );
    let claude_outcome = run_claude_foreground(
        ctx.claude_path,
        ctx.repo_root,
        claude,
        ctx.marker,
        &ctx.models.claude,
    )?;
    let claude_stop = claude_should_stop(claude_outcome);
    let codex_status = wait_codex(codex_child, ctx.codex_log)?;
    let codex_stop = should_stop("codex", codex_status, Some(ctx.codex_log));
    if codex_status.success() {
        if ctx.report.is_file() {
            println!(
                "{} codex review written → {}",
                "✓".green().bold(),
                ctx.report.display()
            );
        } else {
            println!(
                "{} codex exited cleanly but wrote no review at {} — /address will run without it",
                "!".yellow().bold(),
                ctx.report.display()
            );
        }
    }
    Ok(claude_stop || codex_stop)
}

/// Print the run header and execute every step of the pipeline in order,
/// stopping early when a step signals to. Split out of [`execute`] purely to
/// keep that function under the maintainability line limit.
fn run_pipeline(ctx: &StepContext, rounds: u32, invocation: &str, report: &Path) -> Result<()> {
    crate::utils::print_logo_header("Pressure Test");
    print_run_header(rounds, invocation, ctx.models);

    for step in plan_steps(rounds, invocation, report) {
        let stop = match step {
            Step::DeleteReport(path) => {
                delete_file(&path)?;
                false
            }
            Step::Pressure { claude, codex } => run_pressure_step(ctx, &claude, &codex)?,
            Step::Address(slash) => {
                let outcome = run_claude_foreground(
                    ctx.claude_path,
                    ctx.repo_root,
                    &slash,
                    ctx.marker,
                    &ctx.models.address,
                )?;
                claude_should_stop(outcome)
            }
        };
        if stop {
            return Ok(());
        }
    }

    println!("\n{} Pressure test complete.", "✓".green().bold());
    Ok(())
}

/// Execute the pressure pipeline.
pub fn execute(args: PressureArgs) -> Result<()> {
    let PressureArgs {
        plan,
        rounds,
        dry_run,
        claude_model,
        codex_model,
        address_model,
    } = args;

    let config = UserConfig::load();
    let models = PressureModels::resolve(claude_model, codex_model, address_model, &config);

    let repo_root = resolve_repo_root()?;
    let repo_root = repo_root.canonicalize().unwrap_or(repo_root);
    let resolved = resolve_plan_path(&plan, &repo_root)?;
    let report = codex_report_path(&resolved.fs_path);
    let marker = claude_marker_path(&repo_root);
    let codex_log = codex_log_path();

    if dry_run {
        print!(
            "{}",
            render_dry_run(
                rounds,
                &resolved.invocation,
                &report,
                &repo_root,
                &marker,
                &codex_log,
                &models
            )
        );
        return Ok(());
    }

    let claude_path = find_claude_path()?;
    let codex_path = find_codex_path()?;
    let ctx = StepContext {
        claude_path: &claude_path,
        codex_path: &codex_path,
        repo_root: &repo_root,
        marker: &marker,
        codex_log: &codex_log,
        report: &report,
        models: &models,
    };
    run_pipeline(&ctx, rounds, &resolved.invocation, &report)
}

#[cfg(test)]
mod tests;
