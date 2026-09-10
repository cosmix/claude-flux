//! Cleanup functions for loom init command.

use anyhow::{Context, Result};
use colored::Colorize;
use std::fs;
use std::path::Path;

use crate::commands::common::resolve_state_dir;
use crate::fs::memory::archive_run_state;
use crate::fs::work_dir::WorkDir;
use crate::git::branch::branch_name_for_stage;
use crate::git::runner::run_git;
use crate::orchestrator::terminal::tmux::{
    kill_socket_server, list_loom_sockets, socket_session_is_alive, LoomSocket,
};

/// Prune stale git worktrees that have been deleted but are still registered
pub fn prune_stale_worktrees(repo_root: &Path) -> Result<()> {
    let result = run_git(&["worktree", "prune"], repo_root);

    match result {
        Ok(output) if output.status.success() => {
            println!("  {} Stale worktrees pruned", "✓".green().bold());
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            println!(
                "  {} Worktree prune: {}",
                "⚠".yellow().bold(),
                stderr.trim().dimmed()
            );
        }
        Err(e) => {
            println!(
                "  {} Worktree prune: {}",
                "⚠".yellow().bold(),
                e.to_string().dimmed()
            );
        }
    }

    Ok(())
}

/// Controls which attributed sockets [`cleanup_orphaned_sessions`] is
/// allowed to reap. Named explicitly (rather than a bare `bool`) so call
/// sites read as a decision, not a magic flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionReapMode {
    /// The conservative default: reap only sockets whose session is no
    /// longer alive. A LIVE session is left running — it still has a
    /// visible window (or, for tmux, an attachable session) somewhere.
    OrphansOnly,
    /// `loom init --clean` is about to delete the state directory, which is the ONLY
    /// thing that makes socket attribution possible (a socket is attributed
    /// when `<state-dir>/sessions/<id>.md` exists). Reap every socket attributed
    /// to this work dir — alive or not — before that attribution is
    /// destroyed, or a live tmux-hosted session leaks forever with no way to
    /// find it again. Unattributed sockets are still never touched in this
    /// mode either.
    IncludeLiveBeforeClean,
}

/// Outcome of [`handle_socket`] deciding what to do with a single socket.
enum SocketOutcome {
    /// Not attributed to this work dir — never touched.
    Unattributed,
    /// Attributed but left alone (an `OrphansOnly` sweep skipping a live one).
    Kept,
    /// Attributed and reaped; `was_alive` distinguishes a genuinely dead
    /// (orphaned) session from a live one reaped early by
    /// `IncludeLiveBeforeClean`.
    Reaped { was_alive: bool },
}

/// Decide what to do with one socket found by `list_loom_sockets`, and act on
/// that decision. Split out of [`cleanup_orphaned_sessions`] so the
/// sweep/report loop there stays under the line-count cap; this owns the
/// per-socket policy.
///
/// If `kill_socket_server` fails while the session still appeared alive, the
/// socket is still removed (a failed kill against an already-dead socket is
/// the COMMON case for a genuinely orphaned session, so unconditional removal
/// is what actually reaps stale sockets) but the operator is warned, since
/// discarding the file in that specific case may be the only handle to a
/// server that is genuinely still running.
fn handle_socket(work_dir: &Path, socket: &LoomSocket, mode: SessionReapMode) -> SocketOutcome {
    if !socket.attributed {
        return SocketOutcome::Unattributed;
    }

    let alive = socket_session_is_alive(work_dir, &socket.session_id);
    if alive && mode == SessionReapMode::OrphansOnly {
        return SocketOutcome::Kept;
    }

    let killed = kill_socket_server(&socket.path);
    if alive && !killed {
        println!(
            "  {} kill-server failed for session {} while its process still appears alive; \
             removing the socket anyway — it may leak. Try `tmux -S {} kill-server` manually.",
            "⚠".yellow().bold(),
            socket.session_id,
            socket.path.display()
        );
    }
    let _ = fs::remove_file(&socket.path);
    SocketOutcome::Reaped { was_alive: alive }
}

/// Kill loom tmux sessions attributed to this work dir, per `mode`.
///
/// Sockets that cannot be attributed to this work dir are reported but never
/// touched, in EITHER mode — the tmux socket directory is per-user, not
/// per-repository, so an unattributed socket may belong to a colleague's, or
/// another checkout's, live session.
///
/// With [`SessionReapMode::OrphansOnly`], an attributed socket is reaped only
/// when its session is no longer alive. With
/// [`SessionReapMode::IncludeLiveBeforeClean`], an attributed socket is
/// reaped regardless of liveness — intended to run immediately before
/// the state directory is deleted, since deletion is what destroys the ability to ever
/// attribute (and thus find) that socket again.
pub fn cleanup_orphaned_sessions(repo_root: &Path, mode: SessionReapMode) -> Result<()> {
    let work_dir = resolve_state_dir(repo_root);

    let mut orphaned_reaped = 0;
    let mut live_reaped = 0;
    let mut unattributed = 0;

    for socket in list_loom_sockets(&work_dir) {
        match handle_socket(&work_dir, &socket, mode) {
            SocketOutcome::Unattributed => unattributed += 1,
            SocketOutcome::Kept => {}
            SocketOutcome::Reaped { was_alive: true } => live_reaped += 1,
            SocketOutcome::Reaped { was_alive: false } => orphaned_reaped += 1,
        }
    }

    let total_reaped = orphaned_reaped + live_reaped;
    if total_reaped == 0 {
        println!("  {} No orphaned sessions to clean", "✓".green().bold());
    } else if live_reaped == 0 {
        println!(
            "  {} Reaped {} orphaned tmux session{}",
            "✓".green().bold(),
            orphaned_reaped,
            if orphaned_reaped == 1 { "" } else { "s" }
        );
    } else {
        println!(
            "  {} Reaped {} tmux session{} ({} orphaned, {} still live — about to be \
             orphaned by --clean)",
            "✓".green().bold(),
            total_reaped,
            if total_reaped == 1 { "" } else { "s" },
            orphaned_reaped,
            live_reaped
        );
    }

    if unattributed > 0 {
        println!(
            "  {} {} unattributable tmux socket{} left untouched",
            "─".dimmed(),
            unattributed,
            if unattributed == 1 { "" } else { "s" }
        );
    }

    Ok(())
}

/// Remove the existing state directory
pub fn cleanup_work_directory(repo_root: &Path) -> Result<()> {
    let work_dir = resolve_state_dir(repo_root);

    if !work_dir.exists() {
        return Ok(());
    }

    let workspace = WorkDir::new(&work_dir).ok();
    let plan_id = workspace
        .as_ref()
        .and_then(|work_dir| work_dir.load_config().ok().flatten())
        .and_then(|config| config.plan_id().map(str::to_owned));
    let main_root = workspace
        .and_then(|work_dir| work_dir.main_project_root())
        .unwrap_or_else(|| repo_root.to_path_buf());
    archive_run_state(&work_dir, &main_root, plan_id.as_deref());

    fs::remove_dir_all(&work_dir)
        .with_context(|| format!("Failed to remove state directory at {}", work_dir.display()))?;
    println!(
        "  {} Removed old {}",
        "✓".green().bold(),
        work_dir.display().to_string().dimmed()
    );

    Ok(())
}

/// Remove the state directory silently (used for cleanup on initialization failure)
pub fn remove_work_directory_on_failure(repo_root: &Path) {
    let work_dir = resolve_state_dir(repo_root);

    if work_dir.exists() {
        let _ = fs::remove_dir_all(&work_dir);
    }
}

/// Remove existing loom worktrees and the .worktrees/ directory
pub fn cleanup_worktrees_directory(repo_root: &Path) -> Result<()> {
    let worktrees_dir = repo_root.join(".worktrees");

    if !worktrees_dir.exists() {
        return Ok(());
    }

    if let Ok(entries) = fs::read_dir(&worktrees_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let stage_id = entry.file_name().to_string_lossy().to_string();

                let path_str = path.to_string_lossy().to_string();
                let _ = run_git(&["worktree", "remove", "--force", &path_str], repo_root);

                let branch_name = branch_name_for_stage(&stage_id);
                let _ = run_git(&["branch", "-D", &branch_name], repo_root);
            }
        }
    }

    let _ = run_git(&["worktree", "prune"], repo_root);

    if worktrees_dir.exists() {
        fs::remove_dir_all(&worktrees_dir).with_context(|| {
            format!(
                "Failed to remove .worktrees/ directory at {}",
                worktrees_dir.display()
            )
        })?;
    }

    println!(
        "  {} Removed old {}",
        "✓".green().bold(),
        ".worktrees/".dimmed()
    );

    Ok(())
}

#[cfg(test)]
mod archive_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn cleanup_work_directory_archives_memory_before_removal() {
        let temp = TempDir::new().unwrap();
        let work_dir = temp.path().join(".loom/work");
        fs::create_dir_all(work_dir.join("memory")).unwrap();
        fs::write(work_dir.join("memory/stage.md"), "journal").unwrap();

        cleanup_work_directory(temp.path()).unwrap();

        let archive_root = temp.path().join(".loom/memory/archive");
        let archive = fs::read_dir(archive_root).unwrap().next().unwrap().unwrap();
        assert_eq!(
            fs::read_to_string(archive.path().join("memory/stage.md")).unwrap(),
            "journal"
        );
    }
}
