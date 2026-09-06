//! Knowledge stage signal generation
//!
//! Knowledge stages run in the main repository (not worktrees) and focus on
//! exploring and documenting the codebase. They require commits but not merges.

use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::models::session::Session;
use crate::models::stage::Stage;

use super::cache::generate_knowledge_stable_prefix;
use super::format::{extract_tasks_from_description, format_stage_brief};
use super::generate::build_embedded_context_for_stage;
use super::retrieval::STAGE_QUERY_INPUTS;
use super::types::DependencyStatus;

/// Generate a signal file for a knowledge stage
///
/// Knowledge stages differ from regular stages:
/// - Run in the main repository (no worktree)
/// - Commits required, but no merging (auto merged=true)
/// - Focus on populating doc/loom/knowledge/
pub fn generate_knowledge_signal(
    session: &Session,
    stage: &Stage,
    repo_root: &Path,
    dependencies_status: &[DependencyStatus],
    work_dir: &Path,
    handoff_file: Option<&str>,
) -> Result<PathBuf> {
    // Build embedded context
    let mut embedded_context = build_embedded_context_for_stage(work_dir, handoff_file, &stage.id);

    // A knowledge stage is the one stage told to BACKFILL gaps and retire stale
    // entries, so what the tree already holds is exactly its input — and its
    // stable prefix promises a Knowledge Brief like every other prefix. Retrieve
    // it here (an empty tree simply selects nothing and emits no section).
    embedded_context.context_pack = super::helpers::retrieve_stage_pack(work_dir, stage);
    embedded_context.knowledge_tree_empty = super::helpers::knowledge_tree_is_empty(work_dir);

    let content = format_knowledge_signal_content(
        session,
        stage,
        repo_root,
        dependencies_status,
        &embedded_context,
    );

    // Record before writing, as both spawn paths in `generate.rs` do: a session
    // that was briefed must never read back as having received nothing.
    super::helpers::persist_delivery(work_dir, stage, &session.id, &embedded_context);
    super::helpers::write_signal_file(&session.id, &content, work_dir)
}

/// Format the content for a knowledge stage signal
fn format_knowledge_signal_content(
    session: &Session,
    stage: &Stage,
    repo_root: &Path,
    dependencies_status: &[DependencyStatus],
    embedded_context: &super::types::EmbeddedContext,
) -> String {
    let mut content = String::new();

    // Header with session ID
    content.push_str(&format!("# Signal: {}\n\n", session.id));

    // Knowledge-specific stable prefix
    content.push_str(&generate_knowledge_stable_prefix());

    // The brief the prefix just promised ("your signal carries a Knowledge
    // Brief — read it first"). Placed directly after the prefix so it sits
    // where the semi-stable section puts it on every other signal path.
    if let Some(pack) = &embedded_context.context_pack {
        content.push_str(&format_stage_brief(pack, &stage.id, STAGE_QUERY_INPUTS));
    }

    // Target section
    content.push_str(&super::helpers::format_knowledge_target_section(
        &session.id,
        &stage.id,
        stage.plan_id.as_deref(),
        &repo_root.display().to_string(),
    ));

    // Plan overview if available
    if let Some(plan_overview) = &embedded_context.plan_overview {
        content.push_str("## Plan Overview\n\n");
        content.push_str("<plan-overview>\n");
        content.push_str(plan_overview);
        content.push_str("\n</plan-overview>\n\n");
    }

    // Assignment section
    content.push_str("## Assignment\n\n");
    content.push_str(&format!("{}: ", stage.name));
    if let Some(desc) = &stage.description {
        content.push_str(desc);
    } else {
        content.push_str("(no description provided)");
    }
    content.push_str("\n\n");

    // Dependencies status
    if !dependencies_status.is_empty() {
        content.push_str("## Dependencies Status\n\n");
        content.push_str(&super::format::format_dependency_table(dependencies_status));
        content.push('\n');
    }

    // Acceptance Criteria and Files to Explore
    content.push_str(&format_knowledge_criteria_and_files(stage));

    // Immediate tasks
    content.push_str("## Immediate Tasks\n\n");
    let tasks = extract_tasks_from_stage(stage);
    if tasks.is_empty() {
        content.push_str("1. Explore the codebase starting from entry points\n");
        content
            .push_str("2. Document key architectural patterns in doc/loom/knowledge/patterns.md\n");
        content.push_str("3. Document coding conventions in doc/loom/knowledge/conventions.md\n");
        content.push_str("4. Verify acceptance criteria are met\n");
        content.push_str(&format!("5. Run `loom stage complete {}`\n", stage.id));
    } else {
        for (i, task) in tasks.iter().enumerate() {
            content.push_str(&format!("{}. {task}\n", i + 1));
        }
    }
    content.push('\n');

    content
}

/// Format the "## Acceptance Criteria" and "## Files to Explore" sections
/// shared by knowledge-stage signals.
///
/// Extracted out of `format_knowledge_signal_content` to keep that function
/// under the file's line budget: the two sections are always emitted
/// together, right after Dependencies Status, so they read as one cohesive
/// unit rather than two unrelated pieces split apart for their own sake.
fn format_knowledge_criteria_and_files(stage: &Stage) -> String {
    let mut content = String::new();

    content.push_str("## Acceptance Criteria\n\n");
    if stage.acceptance.is_empty() {
        content.push_str("- [ ] Knowledge files populated in doc/loom/knowledge/\n");
        content.push_str("- [ ] Entry points documented\n");
        content.push_str("- [ ] Key patterns documented\n");
        content.push_str("- [ ] Coding conventions documented\n");
    } else {
        for criterion in &stage.acceptance {
            content.push_str(&format!("- [ ] {criterion}\n"));
        }
    }
    content.push('\n');

    // Files to explore (for knowledge stages this is read-only focus)
    if !stage.files.is_empty() {
        content.push_str("## Files to Explore\n\n");
        for file in &stage.files {
            content.push_str(&format!("- {file}\n"));
        }
        content.push('\n');
    }

    content
}

/// Extract tasks from stage description
fn extract_tasks_from_stage(stage: &Stage) -> Vec<String> {
    let mut tasks = Vec::new();

    if let Some(desc) = &stage.description {
        tasks.extend(extract_tasks_from_description(desc));
    }

    if tasks.is_empty() && !stage.acceptance.is_empty() {
        for criterion in &stage.acceptance {
            tasks.push(criterion.command().to_string());
        }
    }

    tasks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::stage::StageStatus;
    use crate::plan::schema::AcceptanceCriterion;

    fn create_test_stage() -> Stage {
        Stage {
            id: "knowledge-bootstrap".to_string(),
            name: "Bootstrap Knowledge Base".to_string(),
            description: Some("Explore the codebase and document findings.".to_string()),
            status: StageStatus::Queued,
            acceptance: vec![
                AcceptanceCriterion::Simple(
                    "grep -q '## ' doc/loom/knowledge/entry-points.md".to_string(),
                ),
                AcceptanceCriterion::Simple(
                    "grep -q '## ' doc/loom/knowledge/patterns.md".to_string(),
                ),
            ],
            files: vec!["src/**/*.rs".to_string()],
            stage_type: crate::models::stage::StageType::Knowledge,
            plan_id: Some("test-plan".to_string()),
            ..Stage::default()
        }
    }

    #[test]
    fn test_format_knowledge_signal_contains_required_sections() {
        let session = Session::new();
        let stage = create_test_stage();
        let repo_root = PathBuf::from("/repo");
        let deps: Vec<DependencyStatus> = vec![];
        let embedded = super::super::types::EmbeddedContext::default();

        let content =
            format_knowledge_signal_content(&session, &stage, &repo_root, &deps, &embedded);

        assert!(content.contains("# Signal:"));
        assert!(content.contains("## Knowledge Stage Context"));
        assert!(content.contains("## Target"));
        assert!(content.contains("Type**: Knowledge"));
        assert!(content.contains("## Assignment"));
        assert!(content.contains("## Acceptance Criteria"));
        assert!(content.contains("## Files to Explore"));
        assert!(content.contains("## Immediate Tasks"));
    }

    #[test]
    fn test_format_knowledge_signal_no_worktree_instructions() {
        let session = Session::new();
        let stage = create_test_stage();
        let repo_root = PathBuf::from("/repo");
        let deps: Vec<DependencyStatus> = vec![];
        let embedded = super::super::types::EmbeddedContext::default();

        let content =
            format_knowledge_signal_content(&session, &stage, &repo_root, &deps, &embedded);

        // Should NOT contain worktree-specific instructions
        assert!(!content.contains("Worktree Context"));
        assert!(!content.contains("isolated git worktree"));
        assert!(!content.contains("git add <specific-files>"));

        // Should contain knowledge-specific instructions
        assert!(content.contains("COMMITS REQUIRED"));
        assert!(content.contains("main repository"));
    }
}
