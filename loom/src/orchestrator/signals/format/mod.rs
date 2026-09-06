use crate::handoff::git_handoff::GitHistory;
use crate::models::session::Session;
use crate::models::stage::Stage;
use crate::models::worktree::Worktree;

use super::cache::{stable_prefix_for, SignalMetrics};
use super::types::{DependencyStatus, EmbeddedContext};

mod brief;
mod codex;
mod helpers;
mod sandbox_section;
mod sections;
mod skills;

// Re-export public functions
pub use helpers::format_dependency_table;
pub use skills::format_skill_recommendations;
// The recovery signal is built outside the semi-stable path but still needs the
// codex lane's rules, the stage's response budget, and the knowledge brief; see
// recovery_format.rs.
pub(crate) use brief::format_knowledge_brief;
pub(crate) use codex::format_codex_implementers_section;
pub(crate) use helpers::format_subagent_timeout_section;

// Re-export for external use
#[allow(unused_imports)]
pub use helpers::extract_tasks_from_description;

/// Result of formatting a signal with structured sections
pub struct FormattedSignal {
    /// The complete signal content
    pub content: String,
    /// Metrics about the signal sections
    pub metrics: SignalMetrics,
}

/// Format signal content using structured sections for KV-cache efficiency
///
/// Signal sections (Manus pattern):
/// 1. STABLE PREFIX - Fixed header that never changes (cached by KV-cache)
/// 2. SEMI-STABLE - Changes per stage (knowledge map, facts, learnings)
/// 3. DYNAMIC - Changes per session (current task, handoff, dependencies)
/// 4. RECITATION - At end for maximum attention (memory, immediate tasks)
pub fn format_signal_content(
    session: &Session,
    stage: &Stage,
    worktree: &Worktree,
    dependencies_status: &[DependencyStatus],
    handoff_file: Option<&str>,
    git_history: Option<&GitHistory>,
    embedded_context: &EmbeddedContext,
) -> String {
    let formatted = format_signal_with_metrics(
        session,
        stage,
        worktree,
        dependencies_status,
        handoff_file,
        git_history,
        embedded_context,
    );
    formatted.content
}

/// Format signal content with metrics about section sizes
pub fn format_signal_with_metrics(
    session: &Session,
    stage: &Stage,
    worktree: &Worktree,
    dependencies_status: &[DependencyStatus],
    handoff_file: Option<&str>,
    git_history: Option<&GitHistory>,
    embedded_context: &EmbeddedContext,
) -> FormattedSignal {
    // Build each section separately for metrics
    let header = format!("# Signal: {}\n\n", session.id);
    // Select stable prefix based on stage type (shared with the recovery path)
    let stable_prefix = stable_prefix_for(stage.stage_type);
    let semi_stable =
        sections::format_semi_stable_section(embedded_context, stage.stage_type, &stage.id);
    let dynamic = sections::format_dynamic_section(
        session,
        stage,
        worktree,
        dependencies_status,
        handoff_file,
        git_history,
        embedded_context,
    );
    let recitation = sections::format_recitation_section(stage, embedded_context);

    // Combine stable prefix with header for hash (header is session-specific but tiny)
    let stable_with_header = format!("{header}{stable_prefix}");

    let metrics =
        SignalMetrics::from_sections(&stable_with_header, &semi_stable, &dynamic, &recitation);

    let content = format!("{stable_with_header}{semi_stable}{dynamic}{recitation}");

    FormattedSignal { content, metrics }
}
