//! Main execution entry point for loom init command.

use crate::commands::repair::workspace::{repair_workspace, AppliedRepair};
use crate::fs::permissions::{ensure_loom_permissions, migrate_legacy_trust};
use crate::fs::work_dir::WorkDir;
use crate::fs::work_integrity::validate_work_dir_state;
use crate::git::install_pre_commit_hook;
use crate::models::session::SessionBackendKind;
use anyhow::{bail, Result};
use colored::Colorize;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use super::cleanup::{
    cleanup_orphaned_sessions, cleanup_work_directory, cleanup_worktrees_directory,
    prune_stale_worktrees, remove_work_directory_on_failure, SessionReapMode,
};
use super::plan_setup::{initialize_with_plan, preflight_plan, PreflightedPlan};
use super::work_state::holds_orchestration_state;

/// RAII guard that cleans up the state directory on drop unless disarmed.
/// This ensures cleanup happens on ANY failure path, not just plan parsing.
struct InitGuard {
    repo_root: PathBuf,
    work_created: bool,
    disarmed: bool,
}

impl InitGuard {
    fn new(repo_root: PathBuf) -> Self {
        Self {
            repo_root,
            work_created: false,
            disarmed: false,
        }
    }

    fn mark_work_created(&mut self) {
        self.work_created = true;
    }

    fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for InitGuard {
    fn drop(&mut self) {
        if self.work_created && !self.disarmed {
            let work_dir = crate::commands::common::resolve_state_dir(&self.repo_root);
            println!(
                "  {} Cleaning up {} due to initialization failure",
                "→".yellow().bold(),
                work_dir.display().to_string().dimmed()
            );
            remove_work_directory_on_failure(&self.repo_root);
        }
    }
}

/// Initialize the state directory structure
///
/// # Arguments
/// * `plan_path` - Optional path to a plan file to initialize with
/// * `clean` - If true, clean up stale resources before initialization
/// * `backend` - Terminal backend for sessions (native|tmux); `None` prompts
///   interactively on a TTY, or defaults to native otherwise
/// * `no_repair` - Skip the automatic workspace repair pass
pub fn execute(
    plan_path: Option<PathBuf>,
    clean: bool,
    backend: Option<String>,
    allow_unsafe_plan: bool,
    no_repair: bool,
) -> Result<()> {
    let repo_root = std::env::current_dir()?;
    let preflighted = preflight_plan_if_given(plan_path.as_deref(), allow_unsafe_plan)?;
    let repo_bootstrap = crate::git::ensure_repo_ready_for_worktrees(&repo_root)?;

    // Repair the workspace BEFORE the validator judges it - see `startup_repairs`.
    let repairs = startup_repairs(&repo_root, no_repair)?;

    validate_work_dir_state(&repo_root)?;

    print_header();
    print_repo_bootstrap(repo_bootstrap);
    for line in startup_repair_lines(&repairs) {
        println!("{line}");
    }

    println!("\n{}", "Cleanup".bold());
    println!("{}", "─".repeat(40).dimmed());

    prune_stale_worktrees(&repo_root)?;
    // `--clean` is about to delete the state directory below, which destroys the ONLY
    // record (`<state-dir>/sessions/<id>.md`) that lets a tmux socket ever be
    // attributed to this work dir again. Reap attributed sockets EVEN IF
    // their session is still alive in that case; otherwise stay
    // conservative and only reap truly-dead sessions.
    let session_reap_mode = if clean {
        SessionReapMode::IncludeLiveBeforeClean
    } else {
        SessionReapMode::OrphansOnly
    };
    cleanup_orphaned_sessions(&repo_root, session_reap_mode)?;

    if clean {
        cleanup_work_directory(&repo_root)?;
        cleanup_worktrees_directory(&repo_root)?;
    }

    println!("\n{}", "Initialize".bold());
    println!("{}", "─".repeat(40).dimmed());

    // `loom init` is one-shot: if the state directory already holds real orchestration
    // state, refuse. Pass --clean to wipe and start over. Reusing a stateful
    // state directory would silently overlay the new plan's stages on top of the
    // previous plan's files, producing duplicate ids and an unrecoverable
    // graph. A state directory that merely EXISTS but holds no such state — e.g. one a
    // stale LOOM_WORK_DIR pin made a hook materialize against a since-deleted
    // state directory (see `WorkDir::new`) — is adopted instead of refused.
    let work_dir = WorkDir::new(".")?;
    let work_dir_path = work_dir.root();
    let adopting_empty_work_dir =
        work_dir_path.exists() && !holds_orchestration_state(work_dir_path);
    if work_dir_path.exists() && !adopting_empty_work_dir {
        bail!(
            "{} already initialized.\n\
             Run `loom init <plan> --clean` to wipe existing state and start over,\n\
             or `loom clean` followed by `loom init <plan>`.",
            work_dir_path.display()
        );
    }

    let mut guard = InitGuard::new(repo_root.clone());
    create_or_adopt_work_dir(&work_dir, adopting_empty_work_dir, &mut guard)?;

    // Install git pre-commit hook to prevent state directory commits
    match install_pre_commit_hook(&repo_root) {
        Ok(true) => {
            println!("  {} Git pre-commit hook installed", "✓".green().bold());
        }
        Ok(false) => {
            println!(
                "  {} Git pre-commit hook {} up to date",
                "✓".green().bold(),
                "already".dimmed()
            );
        }
        Err(e) => {
            println!(
                "  {} Git pre-commit hook installation failed: {}",
                "!".yellow().bold(),
                e.to_string().dimmed()
            );
            // Non-fatal - continue with init
        }
    }

    ensure_loom_permissions(&repo_root)?;
    println!("  {} Permissions configured", "✓".green().bold());
    install_codex_hooks_advisory();

    if let Some(home) = dirs::home_dir() {
        let claude_md = home.join(".claude/CLAUDE.md");
        if !claude_md.exists() {
            println!("  {} ~/.claude/CLAUDE.md not found", "!".yellow().bold());
            println!(
                "    {}",
                "Run install.sh or loom update to install loom rules.".dimmed()
            );
        }
    }

    // Clean up legacy trustedDirectories entries (no-op if none exist)
    if let Err(e) = migrate_legacy_trust(&repo_root) {
        eprintln!("  {} Legacy trust migration: {}", "!".yellow().bold(), e);
    }

    if let Some(preflighted) = preflighted {
        let terminal_backend = resolve_backend_choice(backend)?;
        let stage_count = initialize_with_plan(&work_dir, &preflighted, terminal_backend)?;
        print_summary(plan_path.as_deref(), stage_count);
    } else {
        print_summary(None, 0);
    }

    // Publish the source graph as part of init, AFTER the if/else so a
    // plan-less `loom init` gets one too. Overlay fallback is enabled here and
    // nowhere else: `loom init` is commonly the first command run in a dirty
    // checkout, where a base publish is always refused, and publishing nothing
    // would be honest but useless.
    crate::commands::run::checks::advisory_source_graph_preflight(&repo_root, &work_dir, true);

    // Success - disarm the guard to prevent cleanup
    guard.disarm();

    Ok(())
}

fn install_codex_hooks_advisory() {
    match crate::fs::permissions::install_codex_hooks() {
        Ok(count) => println!(
            "  {} Codex hooks configured ({count} assets; review changes with /hooks)",
            "✓".green().bold()
        ),
        Err(error) => eprintln!(
            "  {} Codex hook setup skipped: {}",
            "!".yellow().bold(),
            error
        ),
    }
}

/// Parse and policy-check the plan, when one was given, before `execute`
/// creates or writes anything.
///
/// The ordering matters: `InitGuard` only ever removes the state directory, so
/// a plan that fails any later leaves the pre-commit hook and the
/// `.claude/settings.local.json` edits behind, and makes the operator answer
/// the backend prompt a second time.
fn preflight_plan_if_given(
    plan_path: Option<&Path>,
    allow_unsafe_plan: bool,
) -> Result<Option<PreflightedPlan>> {
    plan_path
        .map(|path| preflight_plan(path, allow_unsafe_plan))
        .transpose()
}

/// Repair the workspace before anything judges it.
///
/// Placement is load-bearing: the caller runs this AFTER
/// `ensure_repo_ready_for_worktrees` (a workspace can only be repaired inside a
/// real repository) and BEFORE `validate_work_dir_state`, which bails on a
/// `.loom/work` symlink in the main repo - the exact failure this pass heals.
/// Called after the validator, it would never run on that case.
///
/// `no_repair` is the `--no-repair` opt-out: no checks, no fixes, nothing
/// touched.
pub(super) fn startup_repairs(repo_root: &Path, no_repair: bool) -> Result<Vec<AppliedRepair>> {
    if no_repair {
        return Ok(Vec::new());
    }
    repair_workspace(repo_root)
}

/// One line per repair applied - and NOTHING at all when none was. `loom init`
/// is run constantly, and a repair banner on every invocation trains people to
/// ignore it.
pub(super) fn startup_repair_lines(applied: &[AppliedRepair]) -> Vec<String> {
    applied
        .iter()
        .map(|repair| format!("  {} Repaired: {}", "✓".green().bold(), repair.description))
        .collect()
}

/// Resolve the terminal backend choice for `loom init`.
///
/// Precedence: an explicit `--backend` flag always wins (clap's
/// `value_parser` already constrains it to "native"/"tmux"), yielding
/// `Some(kind)`. Otherwise, on an interactive terminal, prompt the operator,
/// also yielding `Some(kind)`. Otherwise (programmatic init, non-TTY) yield
/// `None`: nobody chose a backend, so `plan_setup` leaves the workspace
/// `[terminal]` section out of `config.toml` entirely, letting
/// `~/.loom/config.toml`'s `terminal.backend` — then the built-in default —
/// decide at read time (see `fs::work_dir::read_terminal_config`).
fn resolve_backend_choice(flag: Option<String>) -> Result<Option<SessionBackendKind>> {
    let kind = if let Some(value) = flag {
        Some(match value.as_str() {
            "native" => SessionBackendKind::Native,
            "tmux" => SessionBackendKind::Tmux,
            other => bail!("Invalid terminal backend: {other}"),
        })
    } else if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        Some(prompt_backend_choice()?)
    } else {
        None
    };

    if kind == Some(SessionBackendKind::Tmux) && which::which("tmux").is_err() {
        eprintln!(
            "  {} tmux backend selected but tmux was not found on PATH - install tmux \
             before running `loom run`, or re-run `loom init` with `--backend native`",
            "!".yellow().bold()
        );
    }

    Ok(kind)
}

/// Interactively prompt for the terminal backend, re-prompting on invalid input.
fn prompt_backend_choice() -> Result<SessionBackendKind> {
    loop {
        print!("Terminal backend for sessions [native/tmux] (native): ");
        std::io::stdout().flush().ok();

        let mut response = String::new();
        let bytes_read = std::io::stdin().read_line(&mut response)?;
        if bytes_read == 0 {
            // EOF - default to native.
            return Ok(SessionBackendKind::Native);
        }

        match response.trim().to_ascii_lowercase().as_str() {
            "" | "native" => return Ok(SessionBackendKind::Native),
            "tmux" => return Ok(SessionBackendKind::Tmux),
            _ => println!("  Please enter 'native' or 'tmux'."),
        }
    }
}

/// Lay out the state directory, either creating it or adopting a stateless
/// one that is already there.
///
/// An adopted directory is deliberately NOT marked on the guard: `InitGuard`
/// deletes the state directory wholesale on a later failure, and it may only
/// delete a directory THIS run created.
fn create_or_adopt_work_dir(
    work_dir: &WorkDir,
    adopting: bool,
    guard: &mut InitGuard,
) -> Result<()> {
    let label = work_dir.root().display().to_string();
    if adopting {
        work_dir.adopt_existing()?;
        println!(
            "  {} Adopted existing empty {}",
            "✓".green().bold(),
            label.dimmed()
        );
        println!(
            "    {}",
            format!("Derived caches ({label}/context/, {label}/.loom/) were left in place.")
                .dimmed()
        );
    } else {
        work_dir.initialize()?;
        guard.mark_work_created();
        println!(
            "  {} Directory structure created {}",
            "✓".green().bold(),
            label.dimmed()
        );
    }
    Ok(())
}

fn print_repo_bootstrap(repo_bootstrap: crate::git::RepoBootstrapResult) {
    if !repo_bootstrap.changed() {
        return;
    }

    println!("\n{}", "Git".bold());
    println!("{}", "─".repeat(40).dimmed());

    if repo_bootstrap.initialized_repo {
        println!("  {} Initialized git repository", "✓".green().bold());
    }

    if repo_bootstrap.created_initial_commit {
        println!(
            "  {} Created bootstrap commit for worktree support",
            "✓".green().bold()
        );
    }
}

/// Print the loom init header
fn print_header() {
    crate::utils::print_logo_header("Initializing...");
}

/// Print the final summary
fn print_summary(plan_path: Option<&Path>, stage_count: usize) {
    println!();
    println!("{}", "═".repeat(40).dimmed());

    if let Some(path) = plan_path {
        println!(
            "{} Initialized from {}",
            "✓".green().bold(),
            path.display().to_string().cyan()
        );
        println!(
            "  {} stage{} ready for execution",
            stage_count.to_string().bold(),
            if stage_count == 1 { "" } else { "s" }
        );
    } else {
        println!("{} Empty workspace initialized", "✓".green().bold());
    }

    println!();
    println!("{}", "Next steps:".bold());
    println!("  {}  Start execution", "loom run".cyan());
    println!("  {}  View dashboard", "loom status".cyan());
    println!();
}
