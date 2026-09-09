use std::fs;
use std::path::{Path, PathBuf};

use super::super::types::EmbeddedContext;
use crate::fs::memory::format_memory_for_signal;
use crate::handoff::schema::ParsedHandoff;
use crate::verify::transitions::load_stage;

/// Build embedded context for a stage's memory recitation
pub(crate) fn build_embedded_context_for_stage(
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
pub(crate) fn extract_plan_overview(plan_content: &str) -> Option<String> {
    extract_plan_overview_from(plan_content, "the plan file")
}

/// `pub(crate)` so `tests_size.rs` can call it directly with a pathological
/// (e.g. multi-KB) `plan_label` and assert the truncation bound still holds
/// on the production path, which passes the plan's real file path rather than
/// the short label the `#[cfg(test)]` wrapper above uses.
pub(crate) fn extract_plan_overview_from(plan_content: &str, plan_label: &str) -> Option<String> {
    let mut overview = String::new();
    let mut in_relevant_section = false;
    let mut current_section = String::new();

    for line in plan_content.lines() {
        if line.starts_with("## ") {
            let section_name = line.trim_start_matches("## ").trim().to_lowercase();

            if in_relevant_section && !current_section.is_empty() {
                flush_section(&mut overview, &mut current_section);
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
                    flush_section(&mut overview, &mut current_section);
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

    finish_overview(&overview, plan_label)
}

fn flush_section(overview: &mut String, section: &mut String) {
    overview.push_str(section);
    overview.push_str("\n\n");
    section.clear();
}

fn finish_overview(overview: &str, plan_label: &str) -> Option<String> {
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
