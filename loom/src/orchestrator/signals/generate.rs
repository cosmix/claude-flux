use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

use crate::fs::memory::format_memory_for_signal;
use crate::fs::work_dir::resolve_context_ceiling_tokens;
use crate::handoff::git_handoff::GitHistory;
use crate::handoff::schema::ParsedHandoff;
use crate::language::{detect_languages_from_files, DetectedLanguage};
use crate::models::session::Session;
use crate::models::stage::{Stage, StageType};
use crate::models::worktree::Worktree;
use crate::plan::schema::CodeReviewConfig;
use crate::skills::{SkillIndex, SkillMatch, SkillMetadata};
use crate::verify::transitions::load_stage;

use super::cache::SignalMetrics;
use super::format::{format_signal_content, format_signal_with_metrics};
use super::types::{DependencyStatus, EmbeddedContext, SandboxSummary};

/// Default maximum number of skill recommendations to include in signals
pub const DEFAULT_MAX_SKILL_RECOMMENDATIONS: usize = 8;

/// Score assigned to skills injected via project language detection — higher
/// than trigger-based scores (1.0 word, 2.0 phrase) so they appear prominently.
const LANGUAGE_DETECTION_SCORE: f32 = 10.0;

pub fn generate_signal(
    session: &Session,
    stage: &Stage,
    worktree: &Worktree,
    dependencies_status: &[DependencyStatus],
    handoff_file: Option<&str>,
    git_history: Option<&GitHistory>,
    work_dir: &Path,
) -> Result<PathBuf> {
    generate_signal_with_skills(
        session,
        stage,
        worktree,
        dependencies_status,
        handoff_file,
        git_history,
        work_dir,
        None, // No skill index - backward compatible
        &[],  // No detected languages - backward compatible
    )
}

/// Generate a signal file with optional skill recommendations
#[allow(clippy::too_many_arguments)]
pub fn generate_signal_with_skills(
    session: &Session,
    stage: &Stage,
    worktree: &Worktree,
    dependencies_status: &[DependencyStatus],
    handoff_file: Option<&str>,
    git_history: Option<&GitHistory>,
    work_dir: &Path,
    skill_index: Option<&SkillIndex>,
    detected_languages: &[DetectedLanguage],
) -> Result<PathBuf> {
    let mut embedded_context = build_signal_context(session, stage, work_dir, handoff_file);

    if let Some(index) = skill_index {
        let text_to_match = build_skill_match_text(stage);
        embedded_context.skill_recommendations =
            index.match_skills(&text_to_match, DEFAULT_MAX_SKILL_RECOMMENDATIONS);

        // Detection is file-scoped (a monorepo's frontend stage gets loom-typescript,
        // backend gets loom-rust); fall back to project-wide languages when the
        // stage declares no files.
        let stage_languages = detect_languages_from_files(&stage.files);
        let languages: &[DetectedLanguage] = if stage_languages.is_empty() {
            detected_languages
        } else {
            &stage_languages
        };

        for lang in languages {
            if let Some(metadata) = resolve_language_skill(index, lang.skill_name()) {
                // Only add if not already in recommendations (dedup by name)
                if !embedded_context
                    .skill_recommendations
                    .iter()
                    .any(|s| s.name == metadata.name)
                {
                    embedded_context.skill_recommendations.push(SkillMatch::new(
                        metadata.name.clone(),
                        metadata.description.clone(),
                        LANGUAGE_DETECTION_SCORE,
                        vec!["project-language".to_string()],
                    ));
                }
            }
        }
    }

    let mut content = format_signal_content(
        session,
        stage,
        worktree,
        dependencies_status,
        handoff_file,
        git_history,
        &embedded_context,
    );

    // Adjudicator feedback (disputed stages only), appended last so it sits
    // where the agent's recitation attention is highest.
    if stage.dispute_count > 0 {
        if let Ok(Some(text)) =
            crate::orchestrator::adjudication::feedback::read_feedback(work_dir, &stage.id)
        {
            content.push_str("\n## Adjudicator Feedback (from your prior dispute)\n\n");
            content.push_str(&text);
            super::helpers::ensure_trailing_newline(&mut content);
        }
    }

    // Surface the stage's code-review dimensions to integration-verify agents.
    if matches!(stage.stage_type, StageType::IntegrationVerify) {
        if let Some(section) = stage
            .code_review
            .as_ref()
            .and_then(render_review_dimensions)
        {
            content.push_str(&section);
            super::helpers::ensure_trailing_newline(&mut content);
        }
    }

    super::helpers::persist_delivery(work_dir, stage, &session.id, &embedded_context);
    super::helpers::write_signal_file(&session.id, &content, work_dir)
}

/// Render the "## Review Dimensions" checklist for an integration-verify signal,
/// framed as mandatory when `require_all` is set and advisory otherwise. Returns
/// `None` when no dimensions are configured.
pub(super) fn render_review_dimensions(config: &CodeReviewConfig) -> Option<String> {
    if config.dimensions.is_empty() {
        return None;
    }

    let mut section = String::from("\n## Review Dimensions\n\n");
    if config.require_all {
        section.push_str(
            "Your review MUST explicitly address **every** dimension below before completing \
             this stage (`require_all`). State your findings for each:\n\n",
        );
    } else {
        section.push_str("Address the following review dimensions where applicable:\n\n");
    }
    for dimension in &config.dimensions {
        section.push_str(&format!("- [ ] **{dimension}**\n"));
    }
    Some(section)
}

/// Resolve a detected language to its skill metadata, trying the `loom-<topic>`
/// prefixed name before the bare topic name. Guards against a silent failure
/// mode: a plain `get_by_name("rust")` never matches a skill named `loom-rust`,
/// so language skills would silently never be injected.
fn resolve_language_skill<'a>(index: &'a SkillIndex, base: &str) -> Option<&'a SkillMetadata> {
    index
        .get_by_name(&format!("loom-{base}"))
        .or_else(|| index.get_by_name(base))
}

/// Build text for skill matching from stage metadata
fn build_skill_match_text(stage: &Stage) -> String {
    let mut text = stage.name.clone();
    if let Some(desc) = &stage.description {
        text.push(' ');
        text.push_str(desc);
    }
    for criterion in &stage.acceptance {
        text.push(' ');
        text.push_str(criterion.command());
    }
    text
}

/// Build embedded context for a stage's memory recitation
pub(super) fn build_embedded_context_for_stage(
    work_dir: &Path,
    handoff_file: Option<&str>,
    stage_id: &str,
) -> EmbeddedContext {
    build_embedded_context_with_stage_and_session(work_dir, handoff_file, Some(stage_id))
}

/// Build embedded context with optional stage-specific task state (no session memory)
pub fn build_embedded_context_with_stage(
    work_dir: &Path,
    handoff_file: Option<&str>,
    stage_id: Option<&str>,
) -> EmbeddedContext {
    build_embedded_context_with_stage_and_session(work_dir, handoff_file, stage_id)
}

/// Build embedded context with both stage and session info for full recitation
pub fn build_embedded_context_with_stage_and_session(
    work_dir: &Path,
    handoff_file: Option<&str>,
    stage_id: Option<&str>,
) -> EmbeddedContext {
    // Availability is resolved here, not in the formatters, so the formatting
    // path stays pure and tests can pin both branches deterministically.
    let mut context = EmbeddedContext {
        codex_available: crate::codex::codex_lane_available(),
        ..EmbeddedContext::default()
    };

    if let Some(handoff_name) = handoff_file {
        let handoff_path = work_dir.join("handoffs").join(format!("{handoff_name}.md"));
        if handoff_path.exists() {
            if let Ok(content) = fs::read_to_string(&handoff_path) {
                match ParsedHandoff::parse(&content) {
                    ParsedHandoff::V2(handoff) => {
                        context.parsed_handoff = Some(*handoff);
                        context.handoff_content = Some(content);
                    }
                    ParsedHandoff::V1Fallback(_) => {
                        context.handoff_content = Some(content);
                    }
                }
            }
        }
    }

    if stage_id
        .and_then(|id| load_stage(id, work_dir).ok())
        .and_then(|stage| stage.plan_overview)
        != Some(false)
    {
        context.plan_overview = read_plan_overview(work_dir);
    }

    // Manus pattern: recite the last 10 memory entries to keep stage context
    // in the attention window.
    if let Some(sid) = stage_id {
        context.memory_content = format_memory_for_signal(work_dir, sid, 10);
    }

    context
}

/// Read the plan overview from the plan file referenced in config.toml
fn read_plan_overview(work_dir: &Path) -> Option<String> {
    let config_path = work_dir.join("config.toml");
    if !config_path.exists() {
        return None;
    }

    let config_content = fs::read_to_string(&config_path).ok()?;
    let config: toml::Value = config_content.parse().ok()?;

    let source_path = config.get("plan")?.get("source_path")?.as_str()?;

    let plan_path = PathBuf::from(source_path);
    if !plan_path.exists() {
        return None;
    }

    let plan_content = fs::read_to_string(&plan_path).ok()?;
    extract_plan_overview_from(&plan_content, source_path)
}

/// Hard cap, in bytes, on the overview text embedded in a signal — unconditional,
/// since a plan's Overview section can run to any length and would otherwise be
/// paid for on every fresh session spawned for the stage.
const MAX_PLAN_OVERVIEW_BYTES: usize = 4096;

/// Truncate `text` to at most `max_bytes` bytes: prefers a line-boundary cut
/// when it keeps most of the budget, else a mid-line (never mid-codepoint)
/// cut, then appends a suffix naming `plan_label`. The suffix is clamped to
/// `max_bytes` first, so the result is always `<= max_bytes` even when
/// `plan_label` alone would blow the budget.
fn truncate_overview(text: &str, max_bytes: usize, plan_label: &str) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }

    let full_suffix =
        format!("\n\n_Overview truncated at {max_bytes} bytes — full text is in {plan_label}._");
    let mut suffix_cut = full_suffix.len().min(max_bytes);
    while suffix_cut > 0 && !full_suffix.is_char_boundary(suffix_cut) {
        suffix_cut -= 1;
    }
    let suffix = &full_suffix[..suffix_cut];

    let budget = max_bytes - suffix.len();
    let mut cut = budget.min(text.len());
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    // An unconditional line-boundary cut can land right after the heading (a
    // one-paragraph Overview) and drop the rest; only take it if it keeps half the budget.
    let line_cut = text[..cut].rfind('\n').unwrap_or(0);
    cut = if line_cut * 2 >= cut { line_cut } else { cut };

    let mut truncated = text[..cut].trim_end().to_string();
    truncated.push_str(suffix);
    truncated
}

/// Extract overview and proposed changes sections from plan markdown.
///
/// Test-only: production code calls [`extract_plan_overview_from`] directly so
/// it can pass the real plan path as the truncation label; this unlabeled form
/// exists so tests don't need to care about that label.
#[cfg(test)]
pub(super) fn extract_plan_overview(plan_content: &str) -> Option<String> {
    extract_plan_overview_from(plan_content, "the plan file")
}

/// `pub(super)` so `tests_size.rs` can call it directly with a pathological
/// (e.g. multi-KB) `plan_label` and assert the truncation bound still holds
/// on the production path, which passes the plan's real file path rather than
/// the short label the `#[cfg(test)]` wrapper above uses.
pub(super) fn extract_plan_overview_from(plan_content: &str, plan_label: &str) -> Option<String> {
    let mut overview = String::new();
    let mut in_relevant_section = false;
    let mut current_section = String::new();

    for line in plan_content.lines() {
        if line.starts_with("## ") {
            let section_name = line.trim_start_matches("## ").trim().to_lowercase();

            if in_relevant_section && !current_section.is_empty() {
                overview.push_str(&current_section);
                overview.push_str("\n\n");
                current_section.clear();
            }

            in_relevant_section = section_name.contains("overview")
                || section_name.contains("proposed changes")
                || section_name.contains("summary")
                || section_name.contains("current state");

            if in_relevant_section {
                current_section.push_str(line);
                current_section.push('\n');
            }
        } else if line.starts_with("# ") && overview.is_empty() {
            overview.push_str(line);
            overview.push_str("\n\n");
        } else if in_relevant_section {
            // Stop at next major section (Stages, metadata, etc.)
            let trimmed = line.trim().to_lowercase();
            if trimmed.starts_with("## stages")
                || trimmed.starts_with("```yaml")
                || trimmed.starts_with("<!-- loom")
            {
                in_relevant_section = false;
                if !current_section.is_empty() {
                    overview.push_str(&current_section);
                    overview.push_str("\n\n");
                    current_section.clear();
                }
            } else {
                current_section.push_str(line);
                current_section.push('\n');
            }
        }
    }

    if in_relevant_section && !current_section.is_empty() {
        overview.push_str(&current_section);
    }

    let trimmed = overview.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(truncate_overview(
            trimmed,
            MAX_PLAN_OVERVIEW_BYTES,
            plan_label,
        ))
    }
}

/// Generate a signal file with metrics about section sizes, for debugging
/// KV-cache efficiency and token usage.
pub fn generate_signal_with_metrics(
    session: &Session,
    stage: &Stage,
    worktree: &Worktree,
    dependencies_status: &[DependencyStatus],
    handoff_file: Option<&str>,
    git_history: Option<&GitHistory>,
    work_dir: &Path,
) -> Result<(PathBuf, SignalMetrics)> {
    let embedded_context = build_signal_context(session, stage, work_dir, handoff_file);

    let formatted = format_signal_with_metrics(
        session,
        stage,
        worktree,
        dependencies_status,
        handoff_file,
        git_history,
        &embedded_context,
    );

    super::helpers::persist_delivery(work_dir, stage, &session.id, &embedded_context);
    let signal_path = super::helpers::write_signal_file(&session.id, &formatted.content, work_dir)?;

    Ok((signal_path, formatted.metrics))
}

/// True when `stage` is an enrichment target: integration-verify or
/// knowledge-distill with at least one dependency to summarize.
fn wants_stage_enrichment(stage: &Stage) -> bool {
    matches!(
        stage.stage_type,
        StageType::IntegrationVerify | StageType::KnowledgeDistill
    ) && !stage.dependencies.is_empty()
}

/// Build a cross-stage change summary: aggregates each dependency's file
/// assignments and metadata into a bird's-eye view for integration-verify agents.
fn build_cross_stage_summary(work_dir: &Path, stage: &Stage) -> Option<String> {
    if !wants_stage_enrichment(stage) {
        return None;
    }

    let mut summary = String::from("## Cross-Stage Changes\n\n");
    let mut has_content = false;
    let mut all_files: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();

    for dep_id in &stage.dependencies {
        match load_stage(dep_id, work_dir) {
            Ok(dep_stage) => {
                has_content = true;
                summary.push_str(&format!(
                    "### Stage: {} ({})\n",
                    dep_stage.name,
                    format_stage_status(&dep_stage.status)
                ));
                summary.push_str(&format!("Branch: loom/{dep_id}\n"));

                if !dep_stage.files.is_empty() {
                    summary.push_str("Files:\n");
                    for file in &dep_stage.files {
                        summary.push_str(&format!("- {file}\n"));
                        all_files
                            .entry(file.clone())
                            .or_default()
                            .push(dep_id.clone());
                    }
                }
                summary.push('\n');
            }
            Err(_) => {
                // Stage file not found or unreadable - skip gracefully
            }
        }
    }

    if !has_content {
        return None;
    }

    // Identify files touched by multiple stages
    let multi_stage_files: Vec<_> = all_files
        .iter()
        .filter(|(_, stages)| stages.len() > 1)
        .collect();

    let new_file_count: usize = all_files.values().filter(|s| s.len() == 1).count();

    if !multi_stage_files.is_empty() || new_file_count > 0 {
        summary.push_str("### Potential Concerns\n");
        for (file, stages) in &multi_stage_files {
            summary.push_str(&format!(
                "- `{}` modified by {} stages — verify no conflicts\n",
                file,
                stages.len()
            ));
        }
        if new_file_count > 0 {
            summary.push_str(&format!(
                "- {} new file(s) added — verify all are wired\n",
                new_file_count
            ));
        }
        summary.push('\n');
    }

    Some(summary)
}

/// Format a stage status for display
fn format_stage_status(status: &crate::models::stage::StageStatus) -> &'static str {
    use crate::models::stage::StageStatus;
    match status {
        StageStatus::Completed => "completed",
        StageStatus::Executing => "executing",
        StageStatus::Queued => "queued",
        StageStatus::WaitingForDeps => "waiting",
        StageStatus::Blocked => "blocked",
        StageStatus::NeedsHandoff => "needs-handoff",
        StageStatus::WaitingForInput => "waiting-for-input",
        StageStatus::MergeConflict => "merge-conflict",
        StageStatus::Skipped => "skipped",
        StageStatus::CompletedWithFailures => "completed-with-failures",
        StageStatus::MergeBlocked => "merge-blocked",
        StageStatus::NeedsHumanReview => "needs-human-review",
        StageStatus::NeedsAdjudication => "needs-adjudication",
    }
}

/// Build a wiring checklist for integration-verify by extracting wiring-related
/// notes from completed dependency stages' memory entries.
fn build_wiring_checklist(work_dir: &Path, stage: &Stage) -> Option<String> {
    if !wants_stage_enrichment(stage) {
        return None;
    }

    // Keywords indicating wiring-relevant notes
    let wiring_keywords = [
        "needs", "wire", "wiring", "register", "mount", "import", "add to", "connect",
    ];

    let mut checklist = String::from("## Downstream Wiring Checklist\n\n");
    let mut has_items = false;

    for dep_id in &stage.dependencies {
        let memory_path = work_dir.join("memory").join(format!("{dep_id}.md"));

        let content = match fs::read_to_string(&memory_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let stage_name = load_stage(dep_id, work_dir)
            .map(|s| s.name)
            .unwrap_or_else(|_| dep_id.clone());

        let mut stage_items: Vec<String> = Vec::new();

        for line in content.lines() {
            let lower = line.to_lowercase();
            if wiring_keywords.iter().any(|kw| lower.contains(kw)) {
                // Strip common markdown prefixes for cleaner display
                let stripped = line
                    .trim_start_matches('-')
                    .trim_start_matches('*')
                    .trim_start_matches('#')
                    .trim();
                if !stripped.is_empty() {
                    stage_items.push(stripped.to_string());
                }
            }
        }

        if !stage_items.is_empty() {
            has_items = true;
            checklist.push_str(&format!("From stage '{stage_name}':\n"));
            for item in stage_items {
                checklist.push_str(&format!("- [ ] {item}\n"));
            }
            checklist.push('\n');
        }
    }

    if !has_items {
        return None;
    }

    Some(checklist)
}

/// Build signal context with all shared setup logic, consolidating context,
/// budget, usage, and sandbox setup duplicated across the two signal generators.
fn build_signal_context(
    session: &Session,
    stage: &Stage,
    work_dir: &Path,
    handoff_file: Option<&str>,
) -> EmbeddedContext {
    let mut embedded_context = build_embedded_context_for_stage(work_dir, handoff_file, &stage.id);

    // Full three-tier resolution, not the stage's own value alone:
    // the signal must quote the SAME ceiling the hook governs against and the
    // daemon backstops on, `[context] ceiling_tokens` tier included, or the
    // agent reads itself as nearer its limit than it is and hands off early.
    embedded_context.context_ceiling_tokens = Some(resolve_context_ceiling_tokens(
        work_dir,
        stage.context_ceiling_tokens,
    ));
    embedded_context.context_tokens = Some(session.context_tokens);

    embedded_context.sandbox_summary = Some(build_sandbox_summary(stage));

    // Ultracode license and implementer lanes gate the semi-stable section.
    embedded_context.ultracode = stage.ultracode;
    embedded_context.implementers = stage.implementers.clone();

    // Only set when the plan made a deliberate choice: the orchestrator always
    // measures against `effective_subagent_timeout_secs()`, but the signal tells
    // the agent about it only when there's a value worth acting on.
    embedded_context.subagent_timeout_secs = stage.subagent_timeout_secs;

    if wants_stage_enrichment(stage) {
        embedded_context.cross_stage_summary = build_cross_stage_summary(work_dir, stage);
        embedded_context.wiring_checklist = build_wiring_checklist(work_dir, stage);
    }

    embedded_context.context_pack = super::helpers::retrieve_stage_pack(work_dir, stage);
    embedded_context.knowledge_tree_empty = super::helpers::knowledge_tree_is_empty(work_dir);
    embedded_context
}

/// Build sandbox summary from stage configuration
fn build_sandbox_summary(stage: &Stage) -> SandboxSummary {
    // For now, use stage.sandbox directly; later, merge plan-level defaults via sandbox::merge_config.
    SandboxSummary {
        enabled: stage.sandbox.enabled.unwrap_or(true),
        deny_read: stage
            .sandbox
            .filesystem
            .as_ref()
            .map(|f| f.deny_read.clone())
            .unwrap_or_default(),
        deny_write: stage
            .sandbox
            .filesystem
            .as_ref()
            .map(|f| f.deny_write.clone())
            .unwrap_or_default(),
        allow_write: stage
            .sandbox
            .filesystem
            .as_ref()
            .map(|f| f.allow_write.clone())
            .unwrap_or_default(),
        allowed_domains: stage
            .sandbox
            .network
            .as_ref()
            .map(|n| {
                let mut domains = n.allowed_domains.clone();
                domains.extend(n.additional_domains.clone());
                domains
            })
            .unwrap_or_default(),
        excluded_commands: stage.sandbox.excluded_commands.clone(),
    }
}

#[cfg(test)]
mod resolve_skill_tests {
    use super::resolve_language_skill;
    use crate::language::DetectedLanguage;
    use crate::skills::SkillIndex;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    /// Build a skill index from a temp dir containing one skill named `name`.
    fn index_with_skill(name: &str) -> (TempDir, SkillIndex) {
        let temp = TempDir::new().unwrap();
        let skill_dir = temp.path().join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        let mut f = fs::File::create(skill_dir.join("SKILL.md")).unwrap();
        writeln!(f, "---").unwrap();
        writeln!(f, "name: {name}").unwrap();
        writeln!(f, "description: Test skill").unwrap();
        writeln!(f, "---").unwrap();
        let index = SkillIndex::load_from_directory(temp.path()).unwrap();
        (temp, index)
    }

    #[test]
    fn resolves_loom_prefixed_skill_from_bare_language_name() {
        // The real failure mode: skills are installed as `loom-rust` but the
        // language reports the bare name `rust`. The resolver must bridge that.
        let (_temp, index) = index_with_skill("loom-rust");
        let base = DetectedLanguage::Rust.skill_name();
        assert_eq!(base, "rust");
        let resolved = resolve_language_skill(&index, base).expect("loom-rust must resolve");
        assert_eq!(resolved.name, "loom-rust");
    }

    #[test]
    fn resolves_bare_skill_when_unprefixed() {
        let (_temp, index) = index_with_skill("python");
        let resolved = resolve_language_skill(&index, DetectedLanguage::Python.skill_name())
            .expect("bare python must resolve");
        assert_eq!(resolved.name, "python");
    }

    #[test]
    fn returns_none_when_absent() {
        let (_temp, index) = index_with_skill("loom-rust");
        assert!(resolve_language_skill(&index, "golang").is_none());
    }
}
