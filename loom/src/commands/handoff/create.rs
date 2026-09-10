use anyhow::{Context, Result};
use std::env;
use std::path::Path;
use std::process::Command;

use crate::commands::common::work_dir_path;
use crate::fs::memory::format_memory_for_handoff;
use crate::git::branch::current_branch;
use crate::handoff::generator::{generate_handoff, HandoffContent};
use crate::handoff::HandoffOrigin;
use crate::models::session::{Session, SessionStatus};
use crate::models::stage::{Stage, StageStatus};
use crate::orchestrator::monitor::stage_context_tokens;
use crate::verify::transitions::{load_stage, update_stage};

/// The `--trigger` value an agent uses when its own context ceiling forced the
/// handoff. Only this trigger ends the session's turn on the stage; every other
/// trigger just writes a document.
const CEILING_TRIGGER: &str = "ceiling";

/// What the daemon is told when the stage transition below cannot be written.
///
/// The document is the recovery signal in that case: the state directory's
/// `handoffs/` is writable from a worktree session's sandbox and its `stages/`
/// is not, so the daemon's handoff watch reads the document and does the
/// takedown itself.
const SANDBOX_RECOVERY_NOTE: &str = "\
A worktree session may write the state directory's handoffs/ but not its stages/, so this \
transition failing from inside a stage worktree is expected.\n\
The handoff document above IS the record the daemon acts on: it reads the \
document, ends this session and re-queues the stage for a continuation. \
Nothing is lost and nothing needs repeating.\n\
End your turn now. Do not re-run 'loom handoff' and do not start new work.";

/// Execute the `loom handoff` command
///
/// Creates a handoff file capturing current session state for context recovery.
///
/// # Arguments
/// * `stage_arg` - Optional stage ID from CLI (uses LOOM_STAGE_ID env var if not provided)
/// * `session_arg` - Optional session ID from CLI (uses LOOM_SESSION_ID env var if not provided)
/// * `trigger` - Trigger type (e.g., "manual", "precompact", "session_end")
/// * `message` - Optional message to include in the handoff
pub fn execute(
    stage_arg: Option<String>,
    session_arg: Option<String>,
    trigger: String,
    message: Option<String>,
) -> Result<()> {
    // Resolve stage and session IDs from arguments or environment
    let stage_id = resolve_stage_id(&stage_arg)?;
    let session_id = resolve_session_id(&session_arg)?;

    // Determine the state directory (look in the current dir or via its symlink)
    let work_dir = work_dir_path()?;

    // Load stage (gracefully handle missing stage)
    let stage = load_stage(&stage_id, &work_dir).unwrap_or_else(|_| {
        // Create minimal stage if loading fails
        Stage {
            id: stage_id.clone(),
            name: stage_id.clone(),
            description: None,
            plan_id: None,
            ..Default::default()
        }
    });

    // Build handoff content
    let mut content = HandoffContent::new(session_id.clone(), stage_id.clone());

    // Add plan ID if available
    if let Some(ref plan_id) = stage.plan_id {
        content = content.with_plan_id(Some(plan_id.to_string()));
    }

    // Add goals from stage description if available
    if let Some(ref description) = stage.description {
        content = content.with_goals(description.to_string());
    }

    // Get current branch
    if let Ok(branch) = current_branch(&std::env::current_dir()?) {
        content = content.with_current_branch(Some(branch));
    }

    // Get modified files from git status
    if let Ok(files) = get_modified_files() {
        content = content.with_files_modified(files);
    }

    // Merge the stage's memory journal into the handoff (journals are keyed by
    // stage, not session — `memory/<session_id>.md` never existed, so a
    // CLI-triggered handoff used to carry no memory section at all). Same
    // model as the daemon's own path: orchestrator/monitor/handlers.rs.
    content = content.with_memory_content(format_memory_for_handoff(&work_dir, &stage_id));

    // Add message as a next step if provided
    if let Some(msg) = &message {
        content = content.with_next_steps(vec![msg.clone()]);
    }

    // The heartbeat file is the only place a real context reading exists: the
    // hooks measure the transcript and write it there. Before this was read,
    // every handoff loom wrote recorded a context of zero.
    let context_tokens = stage_context_tokens(&work_dir, &stage_id).unwrap_or(0);
    let invoked_manually = stage_arg.is_some() || session_arg.is_some();
    let content = stamp_trigger(content, &trigger, invoked_manually, context_tokens);

    let mut session = Session::new();
    session.id = session_id.clone();
    session.stage_id = Some(stage_id.clone());
    session.status = SessionStatus::Running;
    session.context_tokens = context_tokens;

    // Generate the handoff file
    let handoff_path = generate_handoff(&session, &stage, content, &work_dir)?;

    // Print the handoff file path (hooks parse this output) before any
    // transition failure below can end the command: the document exists either
    // way, and the path is what both the hooks and the agent need.
    println!("{}", handoff_path.display());

    if trigger == CEILING_TRIGGER {
        end_turn_for_handoff(&stage_id, &work_dir, &handoff_path)?;
    }

    Ok(())
}

/// Record how and why the handoff was written: the trigger note in the goals,
/// the resident-context reading, and — for a ceiling handoff — the origin the
/// daemon's handoff watch acts on.
fn stamp_trigger(
    content: HandoffContent,
    trigger: &str,
    invoked_manually: bool,
    context_tokens: u32,
) -> HandoffContent {
    let source = if invoked_manually {
        "manual CLI"
    } else {
        "environment"
    };
    let goals = format!(
        "{}\n\nHandoff created via: {source} (trigger: {trigger})",
        content.goals
    );
    let content = content
        .with_goals(goals)
        .with_context_tokens(context_tokens);
    match origin_for(trigger) {
        Some(origin) => content.with_origin(origin),
        None => content,
    }
}

/// The origin stamped on a document written for `trigger`.
///
/// Only the ceiling trigger gets one: it is the single trigger that asks for
/// the session to end, and the daemon's handoff watch acts on exactly that
/// origin. A precompact or session_end document records context for a session
/// that keeps working, so stamping it would have the daemon kill a healthy
/// agent every time it compacted.
fn origin_for(trigger: &str) -> Option<HandoffOrigin> {
    (trigger == CEILING_TRIGGER).then_some(HandoffOrigin::AgentCeiling)
}

/// Mark the stage `NeedsHandoff` so the daemon takes the session down and
/// spawns a successor with this handoff inlined.
///
/// Only meaningful while the stage is `Executing`: a stage that already moved
/// on has an authority of its own, and a CLI-side write must not overrule it.
/// That skip is a benign no-op and stays one.
///
/// A failed WRITE is not benign and is no longer swallowed. It used to print a
/// warning and exit 0, which read as "handoff complete": the agent stopped, the
/// stage stayed `Executing`, and the daemon — which arms its recovery on the
/// status — had nothing to react to. The error says instead what actually
/// happened and what recovers it, so the agent ends its turn knowing the
/// handoff stands rather than retrying a write it can never land.
fn end_turn_for_handoff(stage_id: &str, work_dir: &Path, handoff_path: &Path) -> Result<()> {
    let mut skipped = None;
    let result = update_stage(stage_id, work_dir, |stage| {
        if stage.status != StageStatus::Executing {
            skipped = Some(stage.status.clone());
            return Ok(());
        }
        stage.try_mark_needs_handoff()
    });

    match (result, skipped) {
        (Err(e), _) => Err(e.context(format!(
            "Could not mark stage '{stage_id}' NeedsHandoff.\n\
             The handoff document was written: {}\n{SANDBOX_RECOVERY_NOTE}",
            handoff_path.display()
        ))),
        (Ok(_), Some(status)) => {
            eprintln!(
                "Note: Stage '{stage_id}' is {status:?}, not executing. \
                 Skipping handoff transition."
            );
            Ok(())
        }
        (Ok(_), None) => {
            eprintln!("Stage '{stage_id}' marked NeedsHandoff; session will end.");
            Ok(())
        }
    }
}

/// Resolve stage ID from argument or LOOM_STAGE_ID environment variable
fn resolve_stage_id(stage_arg: &Option<String>) -> Result<String> {
    if let Some(stage) = stage_arg {
        return Ok(stage.clone());
    }

    env::var("LOOM_STAGE_ID").context(
        "No stage ID provided and LOOM_STAGE_ID environment variable not set. \
         Use --stage <ID> or run from a loom session.",
    )
}

/// Resolve session ID from argument or LOOM_SESSION_ID environment variable
fn resolve_session_id(session_arg: &Option<String>) -> Result<String> {
    if let Some(session) = session_arg {
        return Ok(session.clone());
    }

    env::var("LOOM_SESSION_ID").context(
        "No session ID provided and LOOM_SESSION_ID environment variable not set. \
         Use --session <ID> or run from a loom session.",
    )
}

/// Get list of modified files from git status
fn get_modified_files() -> Result<Vec<String>> {
    let output = Command::new("git")
        .args(["status", "--short"])
        .output()
        .context("Failed to run git status")?;

    if output.status.success() {
        let status_output =
            String::from_utf8(output.stdout).context("Invalid UTF-8 in git output")?;

        let files: Vec<String> = status_output
            .lines()
            .filter_map(|line| {
                // Git status --short format: "XY filename"
                // Where X is staged status, Y is unstaged status
                if line.len() >= 3 {
                    Some(line[3..].trim().to_string())
                } else {
                    None
                }
            })
            .collect();

        Ok(files)
    } else {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests;
