//! Run command - execute plan stages via orchestrator.
//!
//! This module provides commands for running loom plans either in foreground
//! (debugging) or background (daemon) mode.

pub(crate) mod checks;
mod foreground;
mod graph_loader;
mod plan_inputs;
mod sandbox_preflight;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_checks;

use anyhow::{bail, Result};
use colored::Colorize;

use crate::daemon::{DaemonConfig, DaemonServer};
use crate::fs::work_dir::{read_terminal_config, write_terminal_config, WorkDir};
use crate::models::session::{SessionBackendKind, TerminalConfig};

use checks::prepare_repo_for_run;

// Re-export the main entry point for foreground mode
pub use foreground::execute;

// Re-export plan lifecycle functions for daemon use (now from fs module)
pub use crate::fs::plan_lifecycle::mark_plan_done_if_all_merged;

/// Execute orchestrator in background (daemon mode)
/// Usage: `loom run [--manual] [--max-parallel <n>] [--no-merge] [--backend <native|tmux>]`
pub fn execute_background(
    manual: bool,
    max_parallel: Option<usize>,
    auto_merge: bool,
    backend: Option<String>,
) -> Result<()> {
    let work_dir = prepare_background_run(backend)?;

    crate::utils::print_logo_header("Run");

    if DaemonServer::is_running(work_dir.root()) {
        println!("{} Daemon is already running", "─".dimmed());
        println!();
        println!("  {}  Check status", "loom status".cyan());
        print_stop_guidance();
        return Ok(());
    }

    // Detect terminal BEFORE daemonizing (daemon loses terminal context after fork)
    // Store in environment variable so it can be read back after the fork
    if let Ok(terminal) = crate::orchestrator::terminal::native::detect_terminal() {
        // SAFETY: This runs in main() before the tokio runtime spawns any threads,
        // so there are no concurrent readers of the environment.
        unsafe { std::env::set_var("LOOM_TERMINAL", terminal.display_name()) };
    }

    let daemon_config = DaemonConfig {
        manual_mode: manual,
        max_parallel,
        watch_mode: true, // Background daemon mode continuously watches by design.
        auto_merge,
    };

    let daemon = DaemonServer::with_config(work_dir.root(), daemon_config);
    daemon.start()?;

    println!("{} Daemon started", "✓".green().bold());
    if !auto_merge {
        println!("  {} Auto-merge disabled", "→".dimmed());
    }
    println!();
    println!("  {}  Monitor progress", "loom status".cyan());
    print_stop_guidance();

    Ok(())
}

fn prepare_background_run(backend: Option<String>) -> Result<WorkDir> {
    // Ensure git worktree prerequisites are met before starting.
    let repo_root = std::env::current_dir()?;
    prepare_repo_for_run(&repo_root)?;

    // Absolute, not ".": a long-lived daemon must get an absolute base — a
    // relative state-directory root silently diverges from every other
    // process's view of the same paths (e.g. `loom attach`, which always
    // resolves absolute).
    let work_dir = WorkDir::new(&repo_root)?;
    work_dir.load()?;

    plan_inputs::require_committed_plan(&work_dir)?;

    resolve_backend_flag(&work_dir, backend, "loom run")?;

    // Hard requirement — like `require_jq`: a missing sandbox prerequisite on
    // Linux/WSL makes every session exit at startup, so fail here instead of
    // burning the retry budget on a deterministic startup refusal.
    sandbox_preflight::require_sandbox_prerequisites(work_dir.root())?;

    // Advisory Remote Control preflight — never aborts startup.
    if let Ok(claude_path) = crate::claude::find_claude_path() {
        crate::remote_control::run_startup_preflight(&claude_path, work_dir.root());
    }

    // Advisory Codex lane preflight — never aborts startup.
    checks::advisory_codex_lane_preflight(work_dir.root());

    plan_inputs::mark_plan_in_progress(&work_dir)?;

    // Publish against the committed active filename and the revision stages inherit.
    checks::advisory_source_graph_preflight(&repo_root, &work_dir);

    Ok(work_dir)
}

fn print_stop_guidance() {
    println!("  {}  Stop daemon", "loom stop".cyan());
}

/// Resolve `--backend`, persisting an explicit selection, then run the
/// advisory tmux preflight. Shared by `loom run` (daemon mode, see
/// [`execute_background`]) and `loom run --foreground` (see
/// [`foreground::execute`]) — this is the ONLY path that clears
/// `<state-dir>/terminal-backend-fallback`, so keeping the logic in one place means
/// a fix here reaches both callers instead of risking a fix landing in one
/// copy and not the other.
///
/// Guards against desync with an already-running daemon: its backend is
/// fixed at construction, so a config flip alone cannot reach it. `loom run`
/// never prompts — only `loom init` does.
///
/// `invocation` is the exact command text the caller was invoked as (e.g.
/// `"loom run"` or `"loom run --foreground"`). The desync hint below tells
/// the operator to re-run with `--backend <value>` appended to THIS text —
/// each call site supplies its own so a foreground invocation is never told
/// to drop `--foreground` and run a different command than the one it typed.
///
/// When `backend` is `None`, nothing is persisted; only the preflight check
/// (against whatever backend is already persisted) still runs.
fn resolve_backend_flag(
    work_dir: &WorkDir,
    backend: Option<String>,
    invocation: &str,
) -> Result<()> {
    if let Some(value) = backend {
        let requested = match value.as_str() {
            "native" => SessionBackendKind::Native,
            "tmux" => SessionBackendKind::Tmux,
            other => bail!("Invalid terminal backend: {other}"),
        };

        let persisted = read_terminal_config(work_dir.root())?.backend;

        if DaemonServer::is_running(work_dir.root()) && requested != persisted {
            println!(
                "{} {}",
                "─".dimmed(),
                backend_restart_hint(invocation, &value)
            );
        } else {
            if requested == SessionBackendKind::Tmux {
                // An explicit re-selection is a request to retry tmux.
                crate::orchestrator::terminal::backend::clear_fallback_marker(work_dir.root());
            }
            write_terminal_config(work_dir.root(), &TerminalConfig { backend: requested })?;
        }
    }

    // Advisory tmux preflight — never aborts startup.
    if read_terminal_config(work_dir.root())?.backend == SessionBackendKind::Tmux
        && which::which("tmux").is_err()
    {
        eprintln!(
            "tmux backend selected but tmux not found - sessions will fail to spawn until tmux is \
             installed or the backend is set back to native"
        );
    }

    Ok(())
}

/// Builds the daemon-desync hint text for [`resolve_backend_flag`], isolated
/// from the `println!` so the FIX-critical part — re-running the SAME
/// command shape the operator actually invoked, not a different one — is
/// unit-testable without a live daemon (see `backend_flag_tests` below for
/// why the branch that calls this is not).
fn backend_restart_hint(invocation: &str, value: &str) -> String {
    format!(
        "backend change requires a daemon restart: run `loom stop`, then `{invocation} --backend {value}`"
    )
}

/// Tests for [`resolve_backend_flag`] — the shared `--backend` resolution and
/// marker-clearing logic used by both `loom run` and `loom run --foreground`.
///
/// Kept inline (rather than in `run/tests.rs`) since these exercise a
/// module-private function; none of these need a live daemon, so the
/// daemon-desync branch (`DaemonServer::is_running(..) && requested !=
/// persisted`) is intentionally not covered here — it would require an
/// actually-running daemon process, which is out of scope for a unit test.
#[cfg(test)]
mod backend_flag_tests {
    use super::resolve_backend_flag;
    use crate::fs::work_dir::{read_terminal_config, write_terminal_config, WorkDir};
    use crate::models::session::{SessionBackendKind, TerminalConfig};
    use std::fs;
    use tempfile::TempDir;

    /// Filename of the fallback marker, mirroring
    /// `orchestrator::terminal::backend::TERMINAL_BACKEND_FALLBACK_MARKER`
    /// (private to that module, so duplicated here as a literal).
    const FALLBACK_MARKER_FILE: &str = "terminal-backend-fallback";

    #[test]
    fn tmux_selection_clears_marker_and_persists_tmux() {
        let temp_dir = TempDir::new().unwrap();
        let work_dir = WorkDir::new(temp_dir.path()).unwrap();
        fs::create_dir_all(work_dir.root()).unwrap();
        let marker_path = work_dir.root().join(FALLBACK_MARKER_FILE);
        fs::write(&marker_path, "fell back").unwrap();

        resolve_backend_flag(&work_dir, Some("tmux".to_string()), "loom run").unwrap();

        assert!(
            !marker_path.exists(),
            "an explicit `--backend tmux` re-selection is the operator's only route back to \
             tmux after a fallback and must clear the marker"
        );
        assert_eq!(
            read_terminal_config(work_dir.root()).unwrap().backend,
            SessionBackendKind::Tmux
        );
    }

    #[test]
    fn native_selection_leaves_marker_untouched() {
        let temp_dir = TempDir::new().unwrap();
        let work_dir = WorkDir::new(temp_dir.path()).unwrap();
        fs::create_dir_all(work_dir.root()).unwrap();
        let marker_path = work_dir.root().join(FALLBACK_MARKER_FILE);
        fs::write(&marker_path, "fell back").unwrap();

        resolve_backend_flag(&work_dir, Some("native".to_string()), "loom run").unwrap();

        assert!(
            marker_path.exists(),
            "selecting native is not a request to retry tmux; the marker must survive"
        );
        assert_eq!(
            read_terminal_config(work_dir.root()).unwrap().backend,
            SessionBackendKind::Native
        );
    }

    #[test]
    fn invalid_backend_value_errors_without_touching_config() {
        let temp_dir = TempDir::new().unwrap();
        let work_dir = WorkDir::new(temp_dir.path()).unwrap();
        fs::create_dir_all(work_dir.root()).unwrap();
        write_terminal_config(
            work_dir.root(),
            &TerminalConfig {
                backend: SessionBackendKind::Native,
            },
        )
        .unwrap();

        let result = resolve_backend_flag(&work_dir, Some("screen".to_string()), "loom run");

        assert!(result.is_err());
        assert_eq!(
            read_terminal_config(work_dir.root()).unwrap().backend,
            SessionBackendKind::Native,
            "an invalid --backend value must never touch the persisted config"
        );
    }

    #[test]
    fn omitted_backend_flag_writes_nothing_and_leaves_marker_untouched() {
        let temp_dir = TempDir::new().unwrap();
        let work_dir = WorkDir::new(temp_dir.path()).unwrap();
        fs::create_dir_all(work_dir.root()).unwrap();
        let marker_path = work_dir.root().join(FALLBACK_MARKER_FILE);
        fs::write(&marker_path, "fell back").unwrap();
        let config_path = work_dir.root().join("config.toml");
        assert!(!config_path.exists());

        resolve_backend_flag(&work_dir, None, "loom run").unwrap();

        assert!(
            marker_path.exists(),
            "omitting --backend must not touch the fallback marker"
        );
        assert!(
            !config_path.exists(),
            "omitting --backend must not write config.toml"
        );
    }

    #[test]
    fn backend_restart_hint_uses_the_callers_invocation_text() {
        // The daemon-desync branch itself needs a live daemon (see the
        // module doc comment above), but the text it prints is a pure
        // function of `invocation` and is exactly what FIX 3 is about: a
        // foreground caller must be told to re-run `loom run --foreground
        // --backend <x>`, not `loom run --backend <x>`.
        assert_eq!(
            super::backend_restart_hint("loom run", "tmux"),
            "backend change requires a daemon restart: run `loom stop`, then `loom run --backend tmux`"
        );
        assert_eq!(
            super::backend_restart_hint("loom run --foreground", "native"),
            "backend change requires a daemon restart: run `loom stop`, then `loom run --foreground \
             --backend native`"
        );
    }
}
