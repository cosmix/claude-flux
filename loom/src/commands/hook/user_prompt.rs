//! `loom hook user-prompt` — the deterministic UserPromptSubmit entry point.
//!
//! Reads the hook payload from stdin, retrieves a small pack for the prompt,
//! and prints one JSON object carrying the units THIS SESSION has not already
//! been handed in this epoch. There is **no model call and no network call**
//! anywhere below this function: retrieval is pure filesystem and string work.
//!
//! A loom stage is not a precondition. Inside one the brief is keyed to the
//! stage; outside one it is keyed to the checkout's working-tree overlay — the
//! same `(plan, stage)` address `loom map` and `loom knowledge sync` write — so
//! an ordinary session in a mapped repository gets the same retrieval a stage
//! does.
//!
//! Three gates stand between a prompt and a printed brief, in order:
//! `parse_prompt` drops machine-generated payloads before retrieval even
//! runs (task-notification XML, stopped-agent notices — see
//! `is_machine_generated`) and strips the `@` file attachments whose paths
//! would otherwise steer retrieval (`user_prompt_attachments.rs`);
//! `user_prompt_compose.rs`'s admission floor drops retrieval items too weak
//! to be worth saying anything about; and `HookTarget::already_delivered` drops
//! whatever THIS session — not some other session that happens to share the
//! stage — has already been handed this epoch.
//!
//! Every failure is an empty stdout and exit 0. A prompt-submit hook that
//! reports a problem is a prompt-submit hook that interrupts the session, so
//! the only two outcomes here are "one useful object" and "nothing at all".
//!
//! Split across two files to stay under the maintainability line limit: this
//! one owns recipient resolution, the hook's own filters, and the
//! print/record/reconcile ordering; `user_prompt_compose.rs` owns turning a
//! pack into the one stdout line.

use crate::context::config::RetrievalConfig;
use crate::context::delivery::{self, DeliveryRecord};
use crate::context::retrieve::{context_epoch, retrieve_for_stage, StageQuery};
use crate::context::schema::ContextPack;
use crate::fs::work_dir::WorkDir;
use anyhow::Result;
use std::collections::BTreeSet;
use std::io::Read;

use super::target::{non_empty_env, HookTarget};

#[path = "user_prompt_compose.rs"]
mod compose;

pub(crate) use compose::would_emit;

#[path = "user_prompt_attachments.rs"]
mod attachments;

/// Longest stdin payload worth parsing. Anything past it is malformed by
/// definition, which keeps a runaway writer from being read into memory.
const MAX_STDIN_BYTES: u64 = 1024 * 1024;

/// Shortest prompt worth retrieving against — a bare acknowledgement asks
/// nothing, so it earns nothing.
const MIN_PROMPT_CHARS: usize = 24;

/// One prompt's brief: the line to print, and what filing it commits to.
struct Emission {
    target: HookTarget,
    /// The session-scoped delivery key this brief was composed against,
    /// computed once in [`retrieve_for_prompt`] so
    /// [`HookTarget::already_delivered`] and [`HookTarget::record`]
    /// can never disagree about it.
    recipient: String,
    /// The exact payload just printed to stdout. Nothing in production reads
    /// this back — `user_prompt()` has nothing left to do with it once
    /// `retrieve_for_prompt` has already printed it (see the ordering
    /// comment on that call) — so it exists only so
    /// `tests_user_prompt_e2e.rs` can assert on the rendered brief text
    /// directly instead of re-deriving it from `handed_over`.
    #[cfg(test)]
    payload: String,
    handed_over: ContextPack,
}

/// Emit a retrieval brief for the prompt on stdin, or emit nothing.
pub fn user_prompt() -> Result<()> {
    let Some((prompt, session_id)) = read_prompt() else {
        return Ok(());
    };
    let Some(emission) = retrieve_for_prompt(prompt, session_id.as_deref()) else {
        return Ok(());
    };

    // Filed strictly after `retrieve_for_prompt` returns, which is strictly
    // after that function has already printed (or decided there was nothing
    // to print) — see the ordering comment there. A delivery recorded but
    // never printed is a gap: those units stay suppressed for the rest of the
    // epoch without the session ever having seen them.
    emission
        .target
        .record(&emission.recipient, &emission.handed_over);
    Ok(())
}

/// Everything between reading stdin and writing stdout: resolve the
/// recipient, retrieve, gate on the emit floor, drop what this session
/// already holds, print, and nudge the background reconcile — in that order.
///
/// `None` wherever there is nothing honest to print, but the reconcile nudge
/// runs regardless: see the comment above that call.
fn retrieve_for_prompt(prompt: String, session_id: Option<&str>) -> Option<Emission> {
    let target = match HookTarget::from_environment() {
        Some(target) => target,
        None => {
            emit_no_target(session_id);
            return None;
        }
    };
    let config = target.retrieval_config();
    let recipient = delivery::hook_recipient_id(&target.stage_id, session_id);

    let pack = retrieve_pack(&target, prompt, session_id, &config)?;

    let delivered = target.already_delivered(&pack, &recipient);
    let composed =
        compose::compose_with_reason(target.pull_stage.as_deref(), &pack, &delivered, &config);

    let (payload, handed_over) = match composed {
        compose::ComposeOutcome::Emitted(payload, handed_over) => (payload, *handed_over),
        compose::ComposeOutcome::Abstained(reason) => {
            emit_abstention(&target, session_id, reason);
            crate::commands::hook::reconcile_graph::spawn_if_needed(&pack, &target.project_root);
            return None;
        }
    };

    // Print before telemetry, reconcile, or delivery state can imply the
    // session saw a brief that stdout never received.
    println!("{payload}");

    emit_brief(&target, session_id, &handed_over);

    crate::commands::hook::reconcile_graph::spawn_if_needed(&pack, &target.project_root);

    Some(Emission {
        target,
        recipient,
        #[cfg(test)]
        payload,
        handed_over,
    })
}

fn retrieve_pack(
    target: &HookTarget,
    prompt: String,
    session_id: Option<&str>,
    config: &RetrievalConfig,
) -> Option<ContextPack> {
    let mut query = StageQuery::new(&target.project_root, prompt);
    query.overlay = target.overlay.clone();
    match retrieve_for_stage(&query, config.prompt_budget_tokens) {
        Ok(pack) => Some(pack),
        Err(error) => {
            tracing::debug!(%error, "No retrieval available for this prompt");
            emit_abstention(target, session_id, "no-retrieval");
            None
        }
    }
}

fn emit_brief(target: &HookTarget, session_id: Option<&str>, handed_over: &ContextPack) {
    if !target.exists() {
        return;
    }
    let _ = crate::telemetry::emit(
        &target.work_dir,
        &crate::telemetry::TelemetryEvent::PromptBrief {
            stage_id: target.pull_stage.clone(),
            session_id: session_id.map(str::to_string),
            items: handed_over.items.len(),
            estimated_tokens: handed_over.estimated_tokens,
            omitted: handed_over.omitted.omitted,
        },
    );
}

fn emit_abstention(target: &HookTarget, session_id: Option<&str>, reason: &str) {
    if !target.exists() {
        return;
    }
    let _ = crate::telemetry::emit(
        &target.work_dir,
        &crate::telemetry::TelemetryEvent::PromptAbstained {
            stage_id: target.pull_stage.clone(),
            session_id: session_id.map(str::to_string),
            reason: reason.to_string(),
        },
    );
}

/// Record target-resolution failure only when an existing state root can be
/// resolved safely. Telemetry must not create a phantom `.loom/work` tree.
fn emit_no_target(session_id: Option<&str>) {
    let hint = non_empty_env("LOOM_WORK_DIR").unwrap_or_else(|| ".".to_string());
    let Ok(work_dir) = WorkDir::new(hint) else {
        return;
    };
    if !work_dir.root().exists() {
        return;
    }
    let _ = crate::telemetry::emit(
        work_dir.root(),
        &crate::telemetry::TelemetryEvent::PromptAbstained {
            stage_id: non_empty_env("LOOM_STAGE_ID"),
            session_id: session_id.map(str::to_string),
            reason: "no-target".to_string(),
        },
    );
}

impl HookTarget {
    /// The `(node_id, content_hash)` pairs THIS session already holds under
    /// the pack's epoch: its own prior deliveries this epoch under
    /// `recipient`, plus — when this process IS the session the stage
    /// spawned — the units the spawn brief already put in the same context
    /// window. Unreadable records read as "nothing delivered": a repeat
    /// delivery is a waste, a missing one is a gap.
    ///
    /// `recipient` is session-scoped (see [`delivery::hook_recipient_id`]),
    /// not stage-scoped: a brand-new Claude session, with a brand-new empty
    /// context window, must not be suppressed on what a DIFFERENT, possibly
    /// dead, session already consumed from this same stage.
    ///
    /// The stage spawn record is filed under loom's OWN session id
    /// (`orchestrator/signals/retrieval.rs::persist_delivery` passes
    /// `session_id` from the `Session` model), a different id space from the
    /// Claude Code hook `session_id` that `recipient` was built from.
    /// `LOOM_SESSION_ID` is the join the worktree wrapper exports for exactly
    /// this purpose — `hooks/pre-compact.sh` already reads it the same way.
    /// Getting this wrong fails SILENTLY in the expensive direction: the hook
    /// would re-deliver everything the spawn brief already put in this same
    /// context window, so it is read fresh here rather than assumed equal to
    /// anything already known about `recipient`.
    fn already_delivered(&self, pack: &ContextPack, recipient: &str) -> BTreeSet<(String, String)> {
        let epoch = context_epoch(pack);
        let spawn_recipient = non_empty_env("LOOM_SESSION_ID");
        match delivery::load_deliveries(&self.work_dir, &self.plan, &self.stage_id) {
            Ok(records) => delivery::delivered_to_session(
                &records,
                &epoch,
                recipient,
                spawn_recipient.as_deref(),
            ),
            Err(error) => {
                tracing::debug!(%error, "Unreadable delivery records; assuming none");
                BTreeSet::new()
            }
        }
    }

    /// File what was just handed over so the same units stay suppressed for
    /// the rest of the epoch, under `recipient` — the same session-scoped key
    /// [`Self::already_delivered`] read against.
    ///
    /// `recipient` is stable across one session's repeated prompts, so
    /// repeated questions in one epoch converge on a single record instead of
    /// accumulating one file per question. What makes that safe is that
    /// [`delivery::record_delivery`] folds a delivery into the record already
    /// there rather than replacing it. A failure costs a repeat delivery and
    /// nothing else, so it is logged and ignored — the hook has already
    /// printed.
    ///
    /// With no state directory there is nowhere to file anything, and that is not a
    /// failure either: a prompt hook running in a project that never asked for
    /// loom must not create a state directory as a side effect of answering one
    /// question. The cost is that suppression cannot work there — every prompt
    /// re-delivers what the last one was given — which is the same "nothing
    /// delivered" the unreadable-record path already accepts.
    fn record(&self, recipient: &str, handed_over: &ContextPack) {
        if !self.exists() {
            tracing::debug!("No state directory to file a prompt-hook delivery record in");
            return;
        }
        let record = DeliveryRecord::from_pack(recipient.to_string(), handed_over);
        if let Err(error) =
            delivery::record_delivery(&self.work_dir, &self.plan, &self.stage_id, &record)
        {
            tracing::debug!(%error, "Could not file a prompt-hook delivery record");
        }
    }
}

/// The prompt and session id from the hook payload on stdin, or `None` when
/// there is nothing worth retrieving against — see [`parse_prompt`].
///
/// The shell side owns the timeout; the byte cap here is what keeps a
/// pathological payload bounded.
fn read_prompt() -> Option<(String, Option<String>)> {
    let mut raw = String::new();
    std::io::stdin()
        .lock()
        .take(MAX_STDIN_BYTES)
        .read_to_string(&mut raw)
        .ok()?;
    let prompt = parse_prompt(&raw)?;
    Some((prompt, parse_session_id(&raw)))
}

/// Extract a usable prompt from a hook payload. `None` for empty or malformed
/// JSON, a missing `prompt` field, a prompt too short to have asked anything,
/// or a prompt shaped like machine output rather than a human question (see
/// [`is_machine_generated`]).
///
/// File attachments are removed before any of that (see
/// [`attachments::strip_attachments`]), so the length check runs against what
/// will actually be retrieved against: a prompt that was nothing but an
/// attached file asked no question and earns no brief.
fn parse_prompt(raw: &str) -> Option<String> {
    let payload: serde_json::Value = serde_json::from_str(raw).ok()?;
    let prompt = payload.get("prompt")?.as_str()?.trim();
    if is_machine_generated(prompt) {
        return None;
    }
    let prompt = attachments::strip_attachments(prompt);
    let prompt = prompt.trim();
    (prompt.chars().count() >= MIN_PROMPT_CHARS).then(|| prompt.to_string())
}

/// True for prompt text that did not come from a human typing a question:
/// task-notification XML the harness injects, or one of the fixed English
/// sentences it prints when a background agent is stopped or when a turn
/// opens with a caveat. Retrieving against these produces a brief of pure
/// noise — there is no question in them to retrieve against.
///
/// A slash-command prompt (`/foo …`) is deliberately NOT caught here: an
/// agent typing `/loop 5m check the deploy` is asking a real question, and
/// none of the three prefixes below match it anyway. Plain `starts_with`, no
/// regex — these are fixed, known-verbatim prefixes, not a pattern language.
fn is_machine_generated(prompt: &str) -> bool {
    prompt.starts_with('<')
        || prompt.starts_with("Background agent ")
        || prompt.starts_with("Caveat: ")
}

/// The `session_id` from a hook payload, or `None` when it is absent, blank,
/// or not a string. Never a parse failure in its own right: [`parse_prompt`]
/// already accepted this same raw JSON, so a second failed parse here just
/// means there was no usable session id, not that the payload was malformed.
fn parse_session_id(raw: &str) -> Option<String> {
    let payload: serde_json::Value = serde_json::from_str(raw).ok()?;
    let session_id = payload.get("session_id")?.as_str()?.trim();
    (!session_id.is_empty()).then(|| session_id.to_string())
}

#[cfg(test)]
#[path = "tests_user_prompt.rs"]
mod tests;
