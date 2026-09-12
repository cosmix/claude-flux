//! Shared session-launch preparation for the native and tmux backends.
//!
//! Builds everything a spawn needs UP TO the point of actually starting a
//! process: the resolved tracking key / PID-file key, the kind-specific
//! prompt, the model/effort/permission-mode policy, and the wrapper script
//! that `exec`s claude. Both [`super::NativeBackend::spawn`] and
//! [`super::super::tmux::TmuxBackend::spawn`] call this so the two lanes can
//! never silently diverge on any of these six behaviors.

use anyhow::Result;
use shell_escape::escape;
use std::borrow::Cow;
use std::path::{Path, PathBuf};

use crate::claude::find_claude_path;
use crate::models::session::{Session, SessionType};
use crate::models::stage::Stage;

use super::session_settings::capsule_for;

/// Derive the Remote Control session name for a spawn, prefixed by kind.
///
/// Base is `stage.name` (required by the plan schema); falls back to
/// `stage.id` when `stage.name` is empty after trimming (hand-built stages
/// with no name set).
fn remote_control_session_name(kind: SessionType, stage: &Stage) -> String {
    let trimmed = stage.name.trim();
    let base = if trimmed.is_empty() {
        stage.id.as_str()
    } else {
        trimmed
    };
    match kind {
        SessionType::Stage => base.to_string(),
        SessionType::Merge => format!("Merge: {base}"),
        SessionType::BaseConflict => format!("Base conflict: {base}"),
        SessionType::Knowledge => format!("Knowledge: {base}"),
        SessionType::Adjudication => format!("Adjudication: {base}"),
    }
}

/// Whether `.loom/work/config.toml`'s `[context]` table has `prompt_cache_split =
/// true`. Defaults to `false`: the split ships DISABLED by default (no env
/// var, no heuristic) — only this explicit config key turns it on.
fn prompt_cache_split_enabled(work_dir: &Path) -> bool {
    let Ok(Some(mut config)) = crate::fs::work_dir::load_config(work_dir) else {
        return false;
    };
    config
        .as_toml_mut()
        .get("context")
        .and_then(|table| table.get("prompt_cache_split"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

/// Resolve the path to hand `claude` via `--append-system-prompt-file`, or
/// `None` when the split is off (the default) or the prefix file could not be
/// written.
///
/// Enabled, this writes the stage's stable prefix
/// ([`crate::orchestrator::signals::stable_prefix_for`]) to
/// `<work_dir>/signals/prefix/<stage-id>.md` and returns that path, so the
/// immutable half of the signal is handed over as its own cacheable unit
/// instead of being re-sent behind the volatile half on every turn. The file
/// is the same string the signal's stable section renders, byte for byte,
/// never an approximation of it — including the pointer to `~/.claude/CLAUDE.md`
/// that now stands in for the doctrine this stage moved out of the signal.
///
/// Every failure degrades to `None`, which produces exactly the command line
/// the split-off default produces: this is an optimisation, and a failed
/// optimisation must never fail the spawn.
fn resolve_prompt_cache_split_prefix_file(work_dir: &Path, stage: &Stage) -> Option<String> {
    if !prompt_cache_split_enabled(work_dir) {
        return None;
    }
    let prefix_dir = work_dir.join("signals").join("prefix");
    if let Err(error) = std::fs::create_dir_all(&prefix_dir) {
        tracing::debug!(stage_id = %stage.id, %error, "could not create the prefix directory; launching without the prompt-cache split");
        return None;
    }
    let prefix_file = prefix_dir.join(format!("{}.md", stage.id));
    let prefix = crate::orchestrator::signals::stable_prefix_for(stage.stage_type);
    if let Err(error) = crate::fs::locking::locked_write(&prefix_file, &prefix) {
        tracing::debug!(stage_id = %stage.id, %error, "could not write the stable prefix; launching without the prompt-cache split");
        return None;
    }
    Some(prefix_file.to_str()?.to_string())
}

/// Model and reasoning effort POLICY per session kind (kept explicit, not
/// buried in the spawn).
///
/// * Merge and base-conflict resolution always run on the strongest model at
///   high reasoning effort, regardless of the originating stage's settings.
/// * Adjudication judges a criterion the stage's own agent could not satisfy,
///   and running it on the disputing stage's model would let a plan pick its
///   own judge — so it uses the adjudicator's own model
///   (`.loom/work/config.toml::[adjudication] model`, default `opus`).
/// * Stage and knowledge sessions resolve through the four-tier chain in
///   `crate::fs::work_dir::resolve_stage_model_effort`: the stage's own plan
///   fields, then `.loom/work/config.toml`'s `[models]`, then
///   `~/.loom/config.toml`'s `[models]`, then the stage type's built-in.
fn model_and_effort(kind: SessionType, stage: &Stage, work_dir: &Path) -> (String, String) {
    match kind {
        SessionType::Merge | SessionType::BaseConflict => ("opus".to_string(), "high".to_string()),
        SessionType::Adjudication => (
            crate::orchestrator::adjudication::resolve_model(work_dir),
            "high".to_string(),
        ),
        SessionType::Stage | SessionType::Knowledge => {
            crate::fs::work_dir::resolve_stage_model_effort(
                work_dir,
                stage.stage_type,
                stage.model.as_deref(),
                stage.reasoning_effort.as_deref(),
            )
        }
    }
}

/// Prepare everything needed to launch a session, short of actually starting
/// the terminal/tmux process.
///
/// Returns `(session, title, pid_key, wrapper_path_abs)`:
/// * `session` — the input session with `session_type` and the stage
///   assignment (and therefore `tracking_key`) applied.
/// * `title` — the window title / tmux session name (`session.tracking_key`).
/// * `pid_key` — the per-session PID-file key (`title + "-" + session.id`).
/// * `wrapper_path_abs` — the absolute path to the wrapper script that
///   `exec`s claude.
pub(crate) fn prepare_session_launch(
    work_dir: &Path,
    kind: SessionType,
    stage: &Stage,
    session: Session,
    signal_path: &Path,
    cwd: &Path,
) -> Result<(Session, String, String, PathBuf)> {
    // Assign the stage first so `tracking_key` is set for the (stage, kind)
    // pair; the window title IS the tracking_key and the PID-file key is
    // derived from it. (Idempotent for knowledge sessions, which derived it
    // at construction.)
    let mut session = session;
    session.session_type = kind;
    session.assign_to_stage(stage.id.clone());

    // Window title. `tracking_key` is `loom-[<kind>-]<stage-id>`; the kind
    // prefix namespaces OS resources and stops there — it must never reach
    // `LOOM_STAGE_ID`, which is why `stage.id` is passed below (see wrapper.rs).
    let title = session.tracking_key.clone();

    // Per-session PID-file key (tracking_key + session.id) so two
    // consecutive sessions for the same stage never share a PID file (O-14).
    let pid_key = format!("{}-{}", title, session.id);

    // Build the kind-specific initial prompt.
    let signal_path_str = signal_path.to_string_lossy();
    let initial_prompt = match kind {
        SessionType::Stage => {
            // The literal keyword "ultracode" in the prompt is what licenses
            // Claude Code's Workflow tool for the session.
            let ultracode_suffix = if stage.ultracode {
                " This stage is licensed for ultracode workflow orchestration."
            } else {
                ""
            };
            format!(
                "Read the signal file at {signal_path_str} and execute the assigned stage work. \
                 This file contains your assignment, tasks, acceptance criteria, \
                 and context files to read.{ultracode_suffix}"
            )
        }
        SessionType::Merge => format!(
            "Read the merge signal file at {signal_path_str} and resolve the merge conflicts. \
             This file contains the conflicting files, merge context, and resolution instructions."
        ),
        SessionType::BaseConflict => format!(
            "Read the base conflict signal file at {signal_path_str} and resolve the merge conflicts. \
             This file contains the conflicting files from merging dependency branches, \
             and instructions for resolution. After resolving, tell the user to run `loom retry {}`.",
            stage.id
        ),
        SessionType::Knowledge => format!(
            "Read the signal file at {signal_path_str} and execute the assigned knowledge gathering work. \
             This file contains your assignment, tasks, acceptance criteria, \
             and instructions for populating the knowledge base."
        ),
        SessionType::Adjudication => format!(
            "Read the adjudication signal file at {signal_path_str} and judge the disputed \
             acceptance criterion. This file contains the dispute, the evidence available to \
             you, and the command that records your verdict. Judge the dispute; change nothing."
        ),
    };
    let escaped_prompt = escape(Cow::Borrowed(&initial_prompt));

    let (model, effort) = model_and_effort(kind, stage, work_dir);

    // Resolve the Claude Code permission mode and pass it on the CLI. Loom
    // stages run autonomously with no human at the terminal, so they must
    // START in the resolved mode (default: `auto`). Writing
    // `permissions.defaultMode` into the worktree's settings.local.json is
    // NOT sufficient: Claude Code v2.1.142+ ignores `defaultMode: "auto"`
    // from project/local settings files, so only `--permission-mode` is
    // honored (see build_claude_command). Resolved from the same
    // `[plan_sandbox]` snapshot the settings generator reads
    // (OrchestratorConfig.sandbox_config is loaded from it too), so the CLI
    // flag and the generated settings file never disagree.
    let sandbox = {
        let plan_sandbox = crate::fs::work_dir::read_plan_sandbox(work_dir)
            .ok()
            .flatten()
            .unwrap_or_default();
        crate::sandbox::merge_config(
            &plan_sandbox,
            &stage.sandbox,
            stage.stage_type,
            &stage.implementers,
        )
    };
    // Find claude's absolute path (needed for macOS where terminals don't inherit PATH).
    // build_claude_command shell-escapes the path, model, effort, and mode (S-3).
    let claude_path = find_claude_path()?;
    let rc_name = remote_control_session_name(kind, stage);
    let remote_control = crate::remote_control::resolve_invocation(work_dir, &rc_name);
    let prefix_file = resolve_prompt_cache_split_prefix_file(work_dir, stage);
    let capsule = capsule_for(&claude_path, cwd, work_dir, &session, prefix_file)?;
    let claude_cmd = super::build_claude_command(
        &claude_path.display().to_string(),
        &model,
        &effort,
        sandbox.permission_mode.as_settings_value(),
        &capsule,
        &remote_control,
        &escaped_prompt,
    );

    // The context ceiling for CLAUDE_CODE_AUTO_COMPACT_WINDOW, resolved through
    // the shared order (stage value -> `[context] ceiling_tokens` -> default)
    // so the window the session runs under is the number the signal quotes and
    // the daemon backstops on. See wrapper.rs's CONTRACT-ordered resolution.
    let context_ceiling_tokens =
        crate::fs::work_dir::resolve_context_ceiling_tokens(work_dir, stage.context_ceiling_tokens);

    // Create the wrapper script (writes PID + start-time before exec'ing
    // claude). `stage.id` sets LOOM_STAGE_ID; `pid_key` names the per-session
    // PID file. Pass cwd so the script can cd there (macOS).
    let wrapper_path = super::wrapper::create_session_wrapper_script(
        work_dir,
        &pid_key,
        &stage.id,
        &session.id,
        &claude_cmd,
        Some(cwd),
        kind,
        context_ceiling_tokens,
        super::build_cache::sccache_usable_in(&sandbox),
    )?;

    // Build the command that runs the wrapper script.
    // IMPORTANT: Use absolute path because macOS terminals open in home directory.
    let wrapper_path_abs = wrapper_path.canonicalize().unwrap_or(wrapper_path);

    Ok((session, title, pid_key, wrapper_path_abs))
}

#[cfg(test)]
#[path = "tests_launch.rs"]
mod tests;
#[cfg(test)]
#[path = "tests_launch_capsule.rs"]
mod tests_launch_capsule;
