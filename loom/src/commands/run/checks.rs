//! Pre-run checks for loom orchestration
//!
//! Contains validation functions that must pass before starting orchestration.

use anyhow::{bail, Result};
use colored::Colorize;
use std::path::Path;

use crate::context::graph_store::GraphStore;
use crate::context::refresh::{ensure_snapshot, SnapshotAction, SnapshotOutcome, SnapshotPolicy};
use crate::context::store::ContextStore;
use crate::fs::work_dir::WorkDir;
use crate::git::{get_uncommitted_changes_summary, has_uncommitted_changes};

/// Ensure the repository is ready for Loom's git worktree operations.
pub fn prepare_repo_for_run(repo_root: &Path) -> Result<()> {
    require_jq()?;

    let repo_bootstrap = crate::git::ensure_repo_ready_for_worktrees(repo_root)?;
    print_repo_bootstrap(repo_bootstrap);

    // Stale per-session identity in the MAIN repo's settings env shadows the
    // wrapper exports of every session this run will spawn (Claude Code
    // applies main-repo settings env to worktree sessions too) — heal before
    // spawning anything.
    for path in crate::fs::permissions::scrub_main_repo_settings_identity(repo_root) {
        println!(
            "{} Removed stale session env vars from {}",
            "✓".green().bold(),
            path.display()
        );
    }

    advisory_sccache_preflight();
    advisory_search_tools_preflight();

    check_for_uncommitted_changes(repo_root)
}

/// Hard requirement — aborts startup. Every loom hook parses the Claude Code
/// hook payload with `jq` (see `hooks/_common.sh`'s `loom_require_jq`); without
/// it the blocking guards cannot read their input and fail closed one by one,
/// and `loom stage complete` is never applied by the completion bridge. Failing
/// here, before any worktree or session is created, surfaces the real missing
/// dependency instead of a run that silently misbehaves stage by stage.
pub fn require_jq() -> Result<()> {
    if which::which("jq").is_err() {
        bail!(
            "jq is not installed. Every loom hook parses the Claude Code hook payload with jq; \
             without it the guards cannot read their input and stage completion is never applied. \
             Install jq (apt install jq / brew install jq) and run loom again."
        );
    }
    Ok(())
}

/// Advisory rg/fd preflight — never aborts startup.
///
/// The installed CLAUDE.md steers every agent toward `rg`/`fd` over
/// `grep`/`find` (Rule 8). Neither is a hard requirement: when one is missing,
/// the `prefer-modern-tools` hook allows the legacy tool through with a
/// warning rather than blocking the session, so this is notice, not
/// enforcement — same posture as [`advisory_codex_lane_preflight`].
fn advisory_search_tools_preflight() {
    let mut missing: Vec<&str> = Vec::new();
    if which::which("rg").is_err() {
        missing.push("rg (ripgrep)");
    }
    if which::which("fd").is_err() {
        missing.push("fd");
    }
    if let Some(warning) = format_search_tools_warning(&missing) {
        println!("{warning}");
    }
}

/// Pure formatter for [`advisory_search_tools_preflight`]'s warning line, so
/// its wording is unit-testable without depending on the machine's actual
/// PATH. Returns `None` for an empty `missing` list.
pub(super) fn format_search_tools_warning(missing: &[&str]) -> Option<String> {
    if missing.is_empty() {
        return None;
    }
    Some(format!(
        "⚠ Search tools missing: {} - CLAUDE.md prefers these over grep/find; the \
         prefer-modern-tools hook will allow the legacy commands until they are installed",
        missing.join(", ")
    ))
}

/// Advisory sccache preflight — never aborts startup.
///
/// Prints ONE line, shared verbatim with `loom doctor`
/// (`orchestrator::terminal::native::sccache_status_line`), reporting
/// whether sccache is available to share compiled dependencies across stage
/// worktrees. Every stage worktree gets its own `target/`, so absent
/// sccache each one recompiles every dependency from scratch before running
/// a single test. Called from [`prepare_repo_for_run`] rather than
/// alongside `advisory_codex_lane_preflight` at each `loom run` call site,
/// so both the foreground and background start paths get it for free
/// without needing their own call.
fn advisory_sccache_preflight() {
    println!(
        "{}",
        crate::orchestrator::terminal::native::sccache_status_line()
    );
}

/// Advisory Codex lane preflight — never aborts startup.
///
/// When any stage licenses the codex lane but the codex CLI or its plugin's
/// companion runtime is missing on this machine, print ONE warning naming the
/// stages. The stage signals independently route codex-tier work to sonnet
/// (the fallback branch of `format_codex_implementers_section`), so this is
/// notice, not enforcement.
pub fn advisory_codex_lane_preflight(repo_root: &Path) {
    let Ok(stages) = crate::verify::transitions::list_all_stages(repo_root) else {
        return;
    };
    let codex_stage_ids: Vec<&str> = stages
        .iter()
        .filter(|s| s.implementers.includes_codex())
        .map(|s| s.id.as_str())
        .collect();
    if codex_stage_ids.is_empty() {
        return;
    }
    if let Err(reason) = crate::codex::codex_lane_status() {
        eprintln!(
            "codex lane licensed for stage(s) {} but unavailable ({reason}) - \
             terra/luna-tier work will fall back to sonnet.",
            codex_stage_ids.join(", ")
        );
        return;
    }
    // Lane installed — but on Linux codex's own workspace-write sandbox must
    // exclude /tmp: it masks `.git` under every writable root, and inside the
    // stage sandbox (read-only /tmp) bwrap cannot create the missing
    // /tmp/.git mountpoint, so every forward dies before the model runs a
    // single command.
    if cfg!(target_os = "linux") {
        if let Some(config_path) = crate::codex::codex_config_path() {
            if !crate::codex::codex_config_excludes_slash_tmp(&config_path) {
                eprintln!(
                    "codex lane licensed for stage(s) {} but ~/.codex/config.toml does not set \
                     sandbox_workspace_write.exclude_slash_tmp - inside the stage sandbox every \
                     codex exec fails with `bwrap: Can't mkdir /tmp/.git: Read-only file system`. \
                     Run `loom repair --fix` to set it.",
                    codex_stage_ids.join(", ")
                );
            }
        }
    }
}

/// Advisory source-graph preflight - never aborts startup.
///
/// Ensures the immutable base source graph for HEAD. Silent when reused;
/// writes and unavailable snapshots get the shared one-line description.
pub fn advisory_source_graph_preflight(repo_root: &Path, work_dir: &WorkDir) {
    match preflight_snapshot(repo_root, work_dir) {
        Ok(outcome) if outcome.action != SnapshotAction::Reused => {
            eprintln!("{}", outcome.describe());
        }
        Ok(_) => {}
        Err(error) => eprintln!("source graph: unavailable ({error:#})"),
    }
}

fn preflight_snapshot(repo_root: &Path, work_dir: &WorkDir) -> Result<SnapshotOutcome> {
    let store = ContextStore::open(work_dir)?;
    store.ensure()?;
    let graph_store = GraphStore::new(store.root(), work_dir.root());
    Ok(ensure_snapshot(
        &store,
        &graph_store,
        repo_root,
        SnapshotPolicy::BaseOnly,
    ))
}

fn print_repo_bootstrap(result: crate::git::RepoBootstrapResult) {
    if !result.changed() {
        return;
    }

    if result.initialized_repo {
        println!("{} Initialized git repository", "✓".green().bold());
    }

    if result.created_initial_commit {
        println!(
            "{} Created bootstrap commit for worktree support",
            "✓".green().bold()
        );
    }

    if result.removed_stale_git_locks {
        println!(
            "{} Removed stale .git lock files that blocked git init",
            "✓".green().bold()
        );
    }

    if result.backed_up_git_config {
        println!(
            "{} Moved unreadable .git/config aside (kept as .git/config.loom-backup-*)",
            "!".yellow().bold()
        );
    }
}

/// Check for uncommitted changes and bail if found
///
/// This prevents starting orchestration with a dirty repository, which could
/// cause issues with worktree creation and branch management.
pub fn check_for_uncommitted_changes(repo_root: &Path) -> Result<()> {
    if has_uncommitted_changes(repo_root)? {
        let summary = get_uncommitted_changes_summary(repo_root)?;
        eprintln!(
            "{} Cannot start loom run with uncommitted changes",
            "✗".red().bold()
        );
        eprintln!();
        if !summary.is_empty() {
            for line in summary.lines() {
                eprintln!("  {}", line.dimmed());
            }
            eprintln!();
        }
        eprintln!("  {} Commit or stash your changes first:", "→".dimmed());
        eprintln!(
            "    {}  Commit changes",
            "git commit -am \"message\"".cyan()
        );
        eprintln!("    {}  Or stash them", "git stash".cyan());
        bail!("Uncommitted changes in repository - commit or stash before running loom");
    }
    Ok(())
}
