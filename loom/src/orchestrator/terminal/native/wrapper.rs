//! Authoring of the per-session wrapper script that `exec`s claude.
//!
//! Split out of `pid_tracking`, which owns PID files and process discovery.
//! This module owns the other half of the contract: the exact environment a
//! stage session is born with. That environment is a security boundary (the
//! script rebuilds it from an allowlist rather than inheriting the host's) and
//! a correctness boundary (`LOOM_STAGE_ID` and `LOOM_WORKTREE_PATH` are read by
//! hooks, the CLI and the daemon), so it is worth reading on its own.

use super::pid_tracking::{
    create_pid_dir, create_wrappers_dir, pid_file_path, wrapper_script_path,
};
use super::session_log::{create_logs_dir, stderr_log_path};
use crate::fs::permissions::state_root::RIPGREP_CONFIG_FILE;
use crate::models::session::SessionType;
use anyhow::{Context, Result};
use script_text::env_allowlist;
use shell_escape::escape;
use std::fs;
use std::path::{Path, PathBuf};

/// Line continuation inside the generated `exec env -i …` invocation.
const CONTINUATION: &str = "\\\n";

/// Create a wrapper script that writes its PID before exec'ing claude.
///
/// The wrapper script:
/// 1. Changes to the working directory (important for macOS where terminals
///    can't reliably set cwd before spawning)
/// 2. Writes its own PID (`$$`) — and, on Linux, its start-time — to the PID
///    file, so liveness probes can detect PID reuse
/// 3. `exec`s the claude command under a rebuilt, allowlisted environment,
///    with claude's stderr teed into `logs/<session_id>.stderr.log` so a
///    refusal to start outlives the terminal pane that showed it
///
/// # Arguments
/// * `work_dir` - The .loom/work directory path
/// * `pid_key` - The per-session tracking key naming the PID file / wrapper
///   script (the session's stage-key + `session.id`). Distinct from `stage_id`
///   so two consecutive sessions for the same stage never share a PID file.
/// * `stage_id` - The value exported as `LOOM_STAGE_ID`. Always the plain plan
///   stage id, for every session kind. The kind-prefixed form (`merge-…`,
///   `knowledge-…`, `base-conflict-…`) names OS resources only — it must never
///   reach the environment, because every consumer (`loom memory`,
///   `loom handoff`, the heartbeat files, `session-end.sh`'s stage-file glob)
///   looks the value up as a real stage id.
/// * `session_id` - The session identifier (for LOOM_SESSION_ID env var)
/// * `claude_cmd` - The claude command to execute (e.g., "claude 'prompt here'")
/// * `working_dir` - The working directory to cd into before running claude
/// * `kind` - The session kind. Exported directly as `LOOM_SESSION_TYPE`, and
///   also drives the two conditional env vars that are NOT derivable from
///   `stage_id`; see `kind_env`.
/// * `context_ceiling_tokens` - The session's resolved context ceiling
///   (tokens), used to size `CLAUDE_CODE_AUTO_COMPACT_WINDOW`; see
///   `auto_compact_window_tokens`.
///
/// # Returns
/// The path to the created wrapper script
///
/// Delegates to `create_session_wrapper_script` with `rustc_wrapper_allowed = false` — no
/// merged sandbox config to ask here — which keeps `wrapper/tests.rs`'s byte-pinned assertions
/// independent of the host's own sccache install.
// Mirrors `create_session_wrapper_script`'s first eight params positionally; a struct would
// force editing every call site, including `wrapper/tests.rs`'s byte-exact assertions.
#[allow(clippy::too_many_arguments)]
pub fn create_wrapper_script(
    work_dir: &Path,
    pid_key: &str,
    stage_id: &str,
    session_id: &str,
    claude_cmd: &str,
    working_dir: Option<&Path>,
    kind: SessionType,
    context_ceiling_tokens: u32,
) -> Result<PathBuf> {
    create_session_wrapper_script(
        work_dir,
        pid_key,
        stage_id,
        session_id,
        claude_cmd,
        working_dir,
        kind,
        context_ceiling_tokens,
        false,
    )
}

/// Real body behind [`create_wrapper_script`]; same contract, plus `rustc_wrapper_allowed` —
/// whether this session's merged sandbox config permits sccache; see `sccache_env`.
// Flat like `create_wrapper_script`, which this mirrors one-for-one plus the trailing gate;
// collapsing into a struct would force editing every call site, including
// `wrapper/tests.rs`'s byte-exact assertions, which must stay untouched.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_session_wrapper_script(
    work_dir: &Path,
    pid_key: &str,
    stage_id: &str,
    session_id: &str,
    claude_cmd: &str,
    working_dir: Option<&Path>,
    kind: SessionType,
    context_ceiling_tokens: u32,
    rustc_wrapper_allowed: bool,
) -> Result<PathBuf> {
    create_wrappers_dir(work_dir)?;
    create_logs_dir(work_dir)?;
    create_pid_dir(work_dir)?;

    let wrapper_path = wrapper_script_path(work_dir, pid_key);
    let script = build_wrapper_script(
        work_dir,
        &pid_file_path(work_dir, pid_key),
        stage_id,
        session_id,
        claude_cmd,
        working_dir,
        kind,
        context_ceiling_tokens,
        rustc_wrapper_allowed,
    );

    fs::write(&wrapper_path, &script)
        .with_context(|| format!("Failed to write wrapper script: {}", wrapper_path.display()))?;

    // Make the script executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&wrapper_path)?.permissions();
        // Owner-only execute: wrapper scripts are run by the same user, no need for
        // group/other execute permissions. This prevents other users from reading
        // or executing the script, which contains session IDs and paths.
        perms.set_mode(0o700);
        fs::set_permissions(&wrapper_path, perms)?;
    }

    Ok(wrapper_path)
}

/// Absolute form of `path`, falling back to the input when it cannot be
/// resolved. Paths are absolutized because the script may `cd` elsewhere.
///
/// Shared with `native::capsule`: the `--settings` path it resolves and the
/// `cd` target built here must resolve to the same root, or the wrapper's
/// `cd` moves the process to a directory the `--settings` path was never
/// made relative to.
pub(super) fn absolute(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Absolute form of a file that does not exist yet: resolve the parent and
/// re-attach the file name.
fn absolute_target(path: &Path) -> PathBuf {
    if path.exists() {
        return absolute(path);
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => parent
            .canonicalize()
            .map(|p| p.join(name))
            .unwrap_or_else(|_| path.to_path_buf()),
        _ => path.to_path_buf(),
    }
}

/// The `cd` preamble, empty when the session has no working directory.
fn cd_section(working_dir: Option<&Path>) -> String {
    let Some(dir) = working_dir else {
        return String::new();
    };
    let dir_escaped = escape(absolute(dir).display().to_string().into());
    format!(
        r#"# Change to working directory
cd {dir_escaped} || {{ echo "Failed to cd to working directory"; exit 1; }}

"#,
    )
}

/// The two env assignments that depend on the session KIND rather than on any
/// id, returned as `(merge_session_env, worktree_path_env)`.
///
/// * `LOOM_MERGE_SESSION` — only `Merge`, which `commit-guard.sh` lets exit
///   without a commit. Keyed on the kind, never on a `merge-` prefix in
///   `stage_id`: that id is the plain plan stage id for every kind, and a plan
///   is free to name a stage `merge-anything`.
/// * `LOOM_WORKTREE_PATH` — only `Stage`, the one kind that runs inside a loom
///   worktree. Merge, knowledge and base-conflict sessions `cd` into the main
///   repo; exporting the var for them makes presence-based gates
///   (`sandbox_control_session`, `loom-control-complete.sh`) misread a
///   main-repo agent as a sandboxed worktree agent, which is what once made
///   knowledge stages impossible to complete.
fn kind_env(kind: SessionType, working_dir: Option<&Path>) -> (String, String) {
    let merge = if kind == SessionType::Merge {
        format!("    LOOM_MERGE_SESSION=1 {CONTINUATION}")
    } else {
        String::new()
    };
    let worktree = match working_dir.filter(|_| kind == SessionType::Stage) {
        Some(dir) => {
            let assignment = format!("LOOM_WORKTREE_PATH={}", absolute(dir).display());
            format!("    {} {CONTINUATION}", escape(assignment.into()))
        }
        None => String::new(),
    };
    (merge, worktree)
}

/// Records the PID and, best-effort on Linux, the process start-time on line 2
/// so liveness probes can detect PID reuse. `exec` preserves both, so they
/// identify the claude process after it replaces this shell.
fn pid_capture(pid_file: &str) -> String {
    format!(
        r#"# Write our PID, then (best-effort, Linux) the process start-time on
# line 2 so liveness probes can detect PID reuse. exec preserves the PID and
# start-time, so these identify the claude process after exec replaces us.
echo $$ > {pid_file}
if [ -r "/proc/$$/stat" ]; then
    # Field 22 of /proc/<pid>/stat is starttime. The comm field (2) is wrapped
    # in parens and may contain spaces, so strip through the last ')' first.
    _loom_stat=$(cat "/proc/$$/stat" 2>/dev/null)
    _loom_after=${{_loom_stat##*) }}
    _loom_start=$(echo "$_loom_after" | awk '{{print $20}}')
    if [ -n "$_loom_start" ]; then
        echo "$_loom_start" >> {pid_file}
    fi
fi
"#
    )
}

/// `LOOM_WORK_DIR`, plus `RIPGREP_CONFIG_PATH` when the daemon has published
/// `<work_dir>/ripgreprc` (`daemon::server::storage::publish_search_exclusions`),
/// rendered as shell-escaped `exec env` assignments. The config excludes
/// `admin.token`/`user.token` from every `rg` in the session, `-uu` and
/// `--no-ignore` included, so a sandboxed agent never trips the read-deny rule
/// and stalls auto mode on an operator prompt. The export is gated on the
/// file: `loom run --foreground` runs no daemon, publishes no tokens and no
/// config, and an rg pointed at a missing config prints an error on every
/// call. The host's own `RIPGREP_CONFIG_PATH` is deliberately absent from
/// `ENV_ALLOWLIST`.
fn work_dir_env(work_dir: &Path) -> String {
    let work_dir = absolute(work_dir);
    let mut block = format!(
        "    {} {CONTINUATION}",
        escape(format!("LOOM_WORK_DIR={}", work_dir.display()).into())
    );
    let ripgrep_config = work_dir.join(RIPGREP_CONFIG_FILE);
    if ripgrep_config.is_file() {
        block.push_str(&format!(
            "    {} {CONTINUATION}",
            escape(format!("RIPGREP_CONFIG_PATH={}", ripgrep_config.display()).into())
        ));
    }
    block
}

/// Rendered above the exec line; own constant, like `ENV_ALLOWLIST`, to keep `build_wrapper_script`'s body under its line cap.
const EXEC_COMMENT: &str = r#"# Loom stages record knowledge through `loom memory` / `loom knowledge`; Claude
# Code auto-memory writes to a location invisible to orchestration, so disable
# it at the process boundary rather than by instruction alone.
#
# claude renders its TUI on stdout and prints refusals/fatal errors on
# stderr; teeing only stderr keeps the pane's TTY intact while preserving
# claude's last words after the pane is gone. The tee runs under its own
# `env -i "${_loom_env[@]}"` — the process substitution forks before the
# `exec env -i` below runs, so without it tee would keep the operator's full
# host environment for the whole session, readable from /proc/<tee pid>/environ.
# Replace this process with claude under only the explicit stage contract.
"#;

/// Upper clamp applied to `CLAUDE_CODE_AUTO_COMPACT_WINDOW` before export.
/// The installed binary re-clamps to `[1, 1_000_000]` and then again to the
/// model's own context window, so only this upper bound needs applying here
/// — no lower clamp is required.
const AUTO_COMPACT_WINDOW_MAX_TOKENS: u32 = 1_000_000;

/// `CLAUDE_CODE_AUTO_COMPACT_WINDOW` is 1.5x the session's resolved context
/// ceiling, clamped to [`AUTO_COMPACT_WINDOW_MAX_TOKENS`].
///
/// Intended ordering for a session that never hands off on its own: the
/// harness's own context-budget hook instruction fires at 1.0x the ceiling,
/// the daemon's kill at 1.25x, and this Claude Code compaction trigger at
/// 1.5x — effectively unreachable in practice, with `pre-compact.sh`'s
/// block-then-allow hook as the true last resort.
fn auto_compact_window_tokens(context_ceiling_tokens: u32) -> u32 {
    let scaled = (u64::from(context_ceiling_tokens) * 3) / 2;
    scaled.min(u64::from(AUTO_COMPACT_WINDOW_MAX_TOKENS)) as u32
}

/// The three shell-quoted `exec env` assignments that bound and observe a
/// session's resource usage, rendered together as one block. Split out of
/// `build_wrapper_script` purely to keep that function under the line-count
/// cap, the way `kind_env` and `cd_section` already factor out other parts
/// of that same script.
///
/// BASH_MAX_TIMEOUT_MS: lets a foregrounded `loom subagents watch --timeout
/// 3600` run to completion inside a session without the harness's own
/// Bash-call timeout cutting it off first.
///
/// BASH_MAX_OUTPUT_LENGTH: truncates a bare `git show`, an unpiped `rg`, or
/// a `cat` of a large file at the PROCESS boundary rather than letting it
/// enter the session's context whole. The installed Claude Code binary's
/// resolver for this env var takes (name, env, default=30000, cap=150000)
/// with NO LOWER CLAMP, so 12000 is honored as given — the default is
/// 30000, never 150,000.
///
/// CLAUDE_CODE_AUTO_COMPACT_WINDOW: see `auto_compact_window_tokens`.
fn resource_limit_env(context_ceiling_tokens: u32) -> String {
    let auto_compact_window = auto_compact_window_tokens(context_ceiling_tokens);
    format!(
        r#"    "BASH_MAX_TIMEOUT_MS=3600000" \
    "BASH_MAX_OUTPUT_LENGTH=12000" \
    "CLAUDE_CODE_AUTO_COMPACT_WINDOW={auto_compact_window}" \
"#
    )
}

/// Shell-escaped absolute path to this session's stderr capture log, used in
/// the `tee` redirect at the end of the exec line.
fn stderr_log_escaped(work_dir: &Path, session_id: &str) -> String {
    escape(
        absolute_target(&stderr_log_path(work_dir, session_id))
            .display()
            .to_string()
            .into(),
    )
    .into_owned()
}

/// `RUSTC_WRAPPER=<path>` to export, or empty when either `rustc_wrapper_allowed` (the caller's
/// sandbox verdict; see `build_cache::sccache_usable_in`) is false, or nothing resolves. The
/// candidate itself — resolved path vs. the operator's own `RUSTC_WRAPPER` — comes from
/// `build_cache::rustc_wrapper_candidate`; never probed here, since this script itself runs
/// unsandboxed.
fn sccache_env(rustc_wrapper_allowed: bool) -> String {
    if !rustc_wrapper_allowed {
        return String::new();
    }
    let Some(candidate) = super::build_cache::rustc_wrapper_candidate() else {
        return String::new();
    };
    let assignment = escape(format!("RUSTC_WRAPPER={candidate}").into());
    format!("    {assignment} {CONTINUATION}")
}

/// Render the wrapper script text. Pure: every path is resolved by the caller
/// or by `absolute*`, and nothing is written.
// Mirrors `create_wrapper_script`'s parameter list one-for-one; see that
// function's `#[allow(clippy::too_many_arguments)]` for why it stays flat.
#[allow(clippy::too_many_arguments)]
fn build_wrapper_script(
    work_dir: &Path,
    host_pid_file: &Path,
    stage_id: &str,
    session_id: &str,
    claude_cmd: &str,
    working_dir: Option<&Path>,
    kind: SessionType,
    context_ceiling_tokens: u32,
    rustc_wrapper_allowed: bool,
) -> String {
    let cd_section = cd_section(working_dir);
    let (merge_session_env, worktree_path_env) = kind_env(kind, working_dir);
    let session_env = escape(format!("LOOM_SESSION_ID={session_id}").into());
    let session_type_env = escape(format!("LOOM_SESSION_TYPE={kind}").into());
    let stage_env = escape(format!("LOOM_STAGE_ID={stage_id}").into());
    let work_dir_env = work_dir_env(work_dir);
    let pid_file = escape(absolute_target(host_pid_file).display().to_string().into());
    let pid_capture = pid_capture(&pid_file);
    let stderr_log = stderr_log_escaped(work_dir, session_id);
    let resource_limit_env = resource_limit_env(context_ceiling_tokens);
    let sccache_env = sccache_env(rustc_wrapper_allowed);
    let env_allowlist = env_allowlist();
    format!(
        r#"#!/bin/bash
# Loom stage wrapper
# Writes PID to file before exec'ing claude

{cd_section}{pid_capture}
{env_allowlist}
{EXEC_COMMENT}
exec env -i "${{_loom_env[@]}}" \
    {session_env} \
    {session_type_env} \
    {stage_env} \
{work_dir_env}    "LOOM_MAIN_AGENT_PID=$$" \
    "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1" \
    "CLAUDE_CODE_DISABLE_AUTO_MEMORY=1" \
    "CLAUDE_REMOTE_CONTROL_SESSION_NAME_PREFIX=loom" \
{resource_limit_env}{sccache_env}{merge_session_env}{worktree_path_env}    {claude_cmd} 2> >(env -i "${{_loom_env[@]}}" tee -a {stderr_log})
"#
    )
}

mod script_text;

#[cfg(test)]
mod tests;
