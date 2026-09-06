//! Formatting for recovery signal markdown files.

use crate::models::stage::Stage;

use super::cache::stable_prefix_for;
use super::format::{
    format_codex_implementers_section, format_stage_brief, format_subagent_timeout_section,
};
use super::recovery_types::{LastHeartbeatInfo, RecoverySignalContent};
use super::retrieval::STAGE_QUERY_INPUTS;
use super::types::EmbeddedContext;

/// Render the "### Last Known State" block from the previous session's last
/// heartbeat, split out of [`format_recovery_header`] to keep it short.
fn format_last_known_state(hb: &LastHeartbeatInfo) -> String {
    let mut section = String::from("### Last Known State\n\n");
    section.push_str(&format!(
        "- **Timestamp**: {}\n",
        hb.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
    ));
    if let Some(tokens) = hb.context_tokens {
        section.push_str(&format!("- **Context Tokens**: {tokens} tokens\n"));
    }
    if let Some(ref tool) = hb.last_tool {
        section.push_str(&format!("- **Last Tool**: {tool}\n"));
    }
    if let Some(ref activity) = hb.activity {
        section.push_str(&format!("- **Activity**: {activity}\n"));
    }
    section.push('\n');
    section
}

/// Render the "## Recovery Context" / "### Last Known State" / "### Recovery
/// Actions" block: what triggered this recovery, the previous session's last
/// heartbeat, and the suggested next actions.
///
/// Extracted from [`format_recovery_signal`] so that function stays under its
/// line-count ceiling as new sections are added.
fn format_recovery_header(content: &RecoverySignalContent) -> String {
    let mut header = String::new();

    header.push_str("## Recovery Context\n\n");
    header
        .push_str("**This is a RECOVERY session.** The previous session encountered an issue.\n\n");
    header.push_str(&format!("- **Reason**: {}\n", content.reason));
    header.push_str(&format!(
        "- **Previous Session**: {}\n",
        content.previous_session_id
    ));
    header.push_str(&format!(
        "- **Recovery Attempt**: #{}\n",
        content.recovery_attempt
    ));
    header.push_str(&format!(
        "- **Detected At**: {}\n",
        content.detected_at.format("%Y-%m-%d %H:%M:%S UTC")
    ));

    if let Some(ref crash_path) = content.crash_report_path {
        header.push_str(&format!("- **Crash Report**: {}\n", crash_path.display()));
    }

    header.push('\n');

    if let Some(ref hb) = content.last_heartbeat {
        header.push_str(&format_last_known_state(hb));
    }

    header.push_str("### Recovery Actions\n\n");
    for (i, action) in content.recovery_actions.iter().enumerate() {
        header.push_str(&format!("{}. {action}\n", i + 1));
    }
    header.push('\n');

    header
}

/// Format a recovery signal as markdown
pub fn format_recovery_signal(
    content: &RecoverySignalContent,
    stage: &Stage,
    embedded_context: &EmbeddedContext,
) -> String {
    let mut signal = String::new();

    // Header
    signal.push_str(&format!("# Recovery Signal: {}\n\n", content.session_id));

    signal.push_str(&format_recovery_header(content));

    // Full stable prefix for this stage type: worktree context, isolation and path boundaries, the
    // "## Execution Rules" header (a pointer at ~/.claude/CLAUDE.md plus the knowledge-consumption
    // contract), the subagent no-verify block, and — for code stages (Standard / IntegrationVerify)
    // — the mini adversarial code review, plus commit-timing and completion rules. Built outside
    // the KV-cache path, so without this a resumed stage would miss all of that guidance.
    signal.push_str(&stable_prefix_for(stage.stage_type));

    // The codex lane's rules are SEMI-STABLE and per-stage (which lanes THIS stage
    // licenses), so they are never part of the stable prefix and this signal does
    // not embed them elsewhere. Emit the gated block here too, or a resumed codex
    // stage loses foreground-only fan-out, the concurrency cap, and "verification
    // stays with you" for its licensed lanes.
    if stage.implementers.includes_codex() {
        signal.push_str(&format_codex_implementers_section(
            &stage.implementers,
            embedded_context.codex_available,
        ));
    }

    // Same reasoning for the response budget: it is a gated SEMI-STABLE block, so
    // a resumed stage would otherwise be held to a budget it was never told about
    // while the orchestrator keeps measuring it against exactly that budget.
    if let Some(timeout_secs) = stage.subagent_timeout_secs {
        signal.push_str(&format_subagent_timeout_section(timeout_secs));
    }

    // The `## Knowledge Brief` section is also SEMI-STABLE and this signal does
    // not embed it either — without this a resumed stage silently loses the
    // brief it was spawned with, even though the SAME retrieval ran again to
    // build `embedded_context`.
    if let Some(pack) = &embedded_context.context_pack {
        signal.push_str(&format_stage_brief(pack, &stage.id, STAGE_QUERY_INPUTS));
    }

    // Target information
    signal.push_str("## Target\n\n");
    signal.push_str(&format!("- **Session**: {}\n", content.session_id));
    signal.push_str(&format!("- **Stage**: {}\n", content.stage_id));
    if let Some(ref plan_id) = stage.plan_id {
        signal.push_str(&format!("- **Plan**: {plan_id}\n"));
    }
    if let Some(ref worktree) = stage.worktree {
        signal.push_str(&format!("- **Worktree**: {worktree}\n"));
    }
    signal.push_str(&format!("- **Branch**: loom/{}\n", content.stage_id));
    signal.push('\n');

    // Assignment from stage
    signal.push_str("## Assignment\n\n");
    signal.push_str(&format!("{}\n\n", stage.name));
    if let Some(ref desc) = stage.description {
        signal.push_str(&format!("{desc}\n\n"));
    }

    // Acceptance criteria
    if !stage.acceptance.is_empty() {
        signal.push_str("## Acceptance Criteria\n\n");
        for criteria in &stage.acceptance {
            signal.push_str(&format!("- [ ] {criteria}\n"));
        }
        signal.push('\n');
    }

    // Files to modify
    if !stage.files.is_empty() {
        signal.push_str("## Files to Modify\n\n");
        for file in &stage.files {
            signal.push_str(&format!("- {file}\n"));
        }
        signal.push('\n');
    }

    // Embedded context - handoff
    if let Some(ref handoff) = embedded_context.handoff_content {
        signal.push_str("## Previous Session Handoff\n\n");
        signal.push_str("<handoff>\n");
        signal.push_str(handoff);
        signal.push_str("\n</handoff>\n\n");
    }

    // Embedded context - plan overview
    if let Some(ref overview) = embedded_context.plan_overview {
        signal.push_str("## Plan Overview\n\n");
        signal.push_str("<plan-overview>\n");
        signal.push_str(overview);
        signal.push_str("\n</plan-overview>\n\n");
    }

    signal
}
