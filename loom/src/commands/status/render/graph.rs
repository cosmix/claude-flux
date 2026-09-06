//! Tree-based execution graph display for status command
//!
//! Renders stages as a vertical tree with connectors, dependency annotations,
//! and inline status details (session, failure, merge info).

use std::collections::HashMap;
use std::io::Write;

use colored::{Color, Colorize};

use crate::commands::common::tree::{compute_connector, format_dep_annotation};
use crate::commands::graph::colors::color_by_index;
use crate::commands::graph::indicators::status_indicator;
use crate::commands::status::data::{ActivityStatus, StageSummary, StatusData};
use crate::models::session::SessionType;
use crate::models::stage::{StageStatus, StageType};
use crate::orchestrator::{context_health, ContextHealth};
use crate::plan::graph::levels;
use crate::utils::format_elapsed;

use super::attention_model::failure_label;
use super::render_orphaned_warning;

/// All `StageStatus` variants in display order for legend generation.
///
/// Ordered by operational significance so operators can scan quickly.
const LEGEND_STATUSES: &[StageStatus] = &[
    StageStatus::Completed,
    StageStatus::Executing,
    StageStatus::Queued,
    StageStatus::WaitingForDeps,
    StageStatus::WaitingForInput,
    StageStatus::Blocked,
    StageStatus::NeedsHandoff,
    StageStatus::Skipped,
    StageStatus::MergeConflict,
    StageStatus::CompletedWithFailures,
    StageStatus::MergeBlocked,
    StageStatus::NeedsHumanReview,
    StageStatus::NeedsAdjudication,
];

/// Compute topological level for each stage (level = max(dep_levels) + 1)
fn compute_stage_levels(stages: &[StageSummary]) -> HashMap<String, usize> {
    levels::compute_all_levels(stages, |s| s.id.as_str(), |s| &s.dependencies)
}

/// Format inline annotations for a stage (session, failure, merge, held)
fn format_stage_annotations(stage: &StageSummary) -> String {
    let mut parts: Vec<String> = Vec::new();

    // Resident context tokens (only when a session is active).
    if matches!(stage.status, StageStatus::Executing) {
        if let (Some(tokens), Some(ceiling)) = (stage.context_tokens, stage.context_ceiling_tokens)
        {
            let ctx_str = format!("[{tokens}/{ceiling}]");
            let color = match context_health(tokens, ceiling) {
                ContextHealth::Green => Color::Green,
                ContextHealth::Yellow => Color::Yellow,
                ContextHealth::Red => Color::Red,
            };
            parts.push(format!("{}", ctx_str.color(color)));
        }

        if let Some(secs) = stage.elapsed_secs {
            parts.push(format!("{}", format_elapsed(secs).dimmed()));
        }

        // Activity icon
        let activity_icon = stage.activity_status.icon();
        parts.push(activity_icon.to_string());

        // Staleness warning
        if let Some(staleness) = stage.staleness_secs {
            if staleness > 300 {
                parts.push(format!("{}", "(stale)".yellow()));
            }
        }

        push_session_annotations(stage, &mut parts);
    }

    // Held indicator
    if stage.held {
        parts.push(format!("{}", "HELD".yellow()));
    }

    // Failure info for blocked stages
    if stage.status == StageStatus::Blocked {
        let max = stage.max_retries.unwrap_or(3);
        let failure_label = stage
            .failure_info
            .as_ref()
            .map(|i| failure_label(&i.failure_type))
            .unwrap_or("error");
        parts.push(format!(
            "{}",
            format!("{failure_label} ({}/{max})", stage.retry_count).red()
        ));
    }

    // Review reason for NeedsHumanReview
    if stage.status == StageStatus::NeedsHumanReview {
        if let Some(ref reason) = stage.review_reason {
            parts.push(format!("{}", reason.yellow()));
        }
    }

    // Merge status for completed stages
    if stage.status == StageStatus::Completed {
        if stage.merged {
            parts.push(format!("{}", stage.merge_label().green().dimmed()));
        } else if !matches!(stage.stage_type, StageType::Knowledge) {
            // Completed but not merged and not a knowledge stage — needs manual merge
            parts.push(format!("{}", "unmerged".yellow()));
        }
        parts.extend(cleanup_annotation(stage));
    }

    if parts.is_empty() {
        String::new()
    } else {
        let sep = format!(" {} ", "·".dimmed());
        format!("  {}", parts.join(&sep))
    }
}

/// Session-identity annotations for an `Executing` stage: the PID (or
/// `orphaned`), a tag when the speaking session is not of the stage's own
/// worker kind, and the incoherence verdict when one applies.
fn push_session_annotations(stage: &StageSummary, parts: &mut Vec<String>) {
    // Session PID or orphaned
    if let Some(pid) = stage.pid {
        if stage.session_alive {
            parts.push(format!("{}", format!("PID {pid}").dimmed()));
        } else {
            parts.push(format!("{}", "orphaned".red()));
        }
    }

    // The session speaking for this stage is not of its own worker kind
    // (e.g. an adjudication session adopted into the worker slot) —
    // surface it before the stronger incoherence verdict below.
    if let Some(session_type) = stage.session_type {
        let worker_type = if matches!(stage.stage_type, StageType::Knowledge) {
            SessionType::Knowledge
        } else {
            SessionType::Stage
        };
        if session_type != worker_type {
            parts.push(format!(
                "{}",
                format!("{session_type} session").yellow().bold()
            ));
        }
    }
    if let Some(reason) = &stage.incoherence {
        parts.push(format!("{}", format!("INCOHERENT: {reason}").red().bold()));
    }
}

/// The `cleanup failed` annotation for a completed stage whose deferred
/// worktree/branch cleanup was refused or errored; `None` otherwise.
fn cleanup_annotation(stage: &StageSummary) -> Option<String> {
    stage
        .cleanup_warning
        .is_some()
        .then(|| format!("{}", "cleanup failed".yellow()))
}

/// 3-space indent applied to every dashboard row so the tree visually aligns
/// with the surrounding header / progress / legend sections.
const ROW_INDENT: &str = "   ";

/// Render execution graph with tree display
pub fn render_graph<W: Write>(w: &mut W, data: &StatusData) -> std::io::Result<()> {
    if data.stages.is_empty() {
        writeln!(w, "{ROW_INDENT}{}", "(no stages found)".dimmed())?;
        return Ok(());
    }

    let levels = compute_stage_levels(&data.stages);

    // Sort stages by level ASC, then id ASC
    let mut sorted_stages: Vec<&StageSummary> = data.stages.iter().collect();
    sorted_stages.sort_by(|a, b| {
        let level_a = levels.get(&a.id).copied().unwrap_or(0);
        let level_b = levels.get(&b.id).copied().unwrap_or(0);
        level_a.cmp(&level_b).then_with(|| a.id.cmp(&b.id))
    });

    // Create position-based color map so adjacent stages have different colors
    let color_map: HashMap<&str, Color> = sorted_stages
        .iter()
        .enumerate()
        .map(|(i, stage)| (stage.id.as_str(), color_by_index(i)))
        .collect();

    // Count stages per level for connector logic (last stage at each level
    // gets `└─`; others get `├─`).
    let mut level_counts: HashMap<usize, usize> = HashMap::new();
    let mut level_indices: HashMap<usize, usize> = HashMap::new();
    for stage in &sorted_stages {
        let level = levels.get(&stage.id).copied().unwrap_or(0);
        *level_counts.entry(level).or_insert(0) += 1;
    }

    for (global_index, stage) in sorted_stages.iter().enumerate() {
        let level = levels.get(&stage.id).copied().unwrap_or(0);
        let index_in_level = *level_indices.entry(level).or_insert(0);
        let level_size = level_counts.get(&level).copied().unwrap_or(1);

        let connector = compute_connector(level, index_in_level, level_size);
        let indicator = status_indicator(&stage.status);
        let deps = format_dep_annotation(&stage.dependencies, &color_map);
        let color = color_by_index(global_index);
        let colored_id = stage.id.color(color);
        let model_tag = format!(" {}", format!("[{}]", stage.model).dimmed());
        let annotations = format_stage_annotations(stage);

        // Layout: <indent> <connector> <indicator>  <id> <model> <deps> <annotations>
        // Two spaces between indicator and id give room to breathe; deps and
        // annotations sit inline (no fragile column padding).
        writeln!(
            w,
            "{ROW_INDENT}{connector}{indicator}  {colored_id}{model_tag}{deps}{annotations}"
        )?;

        write_orphaned_hint(w, stage, &connector)?;
        write_merge_hint(w, stage, &connector)?;
        write_cleanup_hint(w, stage, &connector)?;

        // Increment index for this level
        *level_indices.get_mut(&level).unwrap() += 1;
    }

    writeln!(w)?;
    render_legend(w)?;

    Ok(())
}

/// For a stage whose activity status is `Orphaned` — it claims to be
/// executing but no session record exists for it at all — show the one-line
/// explanation and the two ways out. No-op for any other stage.
fn write_orphaned_hint<W: Write>(
    w: &mut W,
    stage: &StageSummary,
    connector: &str,
) -> std::io::Result<()> {
    if stage.activity_status != ActivityStatus::Orphaned {
        return Ok(());
    }
    let hint_indent = " ".repeat(connector.chars().count() + 4);
    let hint = render_orphaned_warning(&stage.id);
    writeln!(w, "{ROW_INDENT}{hint_indent}{}", hint.trim_start().red())
}

/// For a completed-but-not-merged non-knowledge stage, show a merge hint.
/// No-op for any other stage.
fn write_merge_hint<W: Write>(
    w: &mut W,
    stage: &StageSummary,
    connector: &str,
) -> std::io::Result<()> {
    if stage.status != StageStatus::Completed
        || stage.merged
        || matches!(stage.stage_type, StageType::Knowledge)
    {
        return Ok(());
    }
    // Indent to align under the stage id (connector width + icon + 2 spaces)
    let hint_indent = " ".repeat(connector.chars().count() + 4);
    let hint = format!("→ run: loom stage merge {}", stage.id);
    writeln!(w, "{ROW_INDENT}{hint_indent}{}", hint.yellow().dimmed())
}

/// For a completed stage whose post-merge cleanup failed or was refused, show
/// why and how to retry it manually. No-op for any other stage.
fn write_cleanup_hint<W: Write>(
    w: &mut W,
    stage: &StageSummary,
    connector: &str,
) -> std::io::Result<()> {
    if stage.status != StageStatus::Completed {
        return Ok(());
    }
    let Some(warning) = &stage.cleanup_warning else {
        return Ok(());
    };
    let hint_indent = " ".repeat(connector.chars().count() + 4);
    let first_line = warning.lines().next().unwrap_or_default();
    let hint = format!(
        "↳ cleanup failed: {first_line} — {}",
        format!("loom worktree remove {}", stage.id).cyan()
    );
    writeln!(w, "{ROW_INDENT}{hint_indent}{}", hint.yellow())
}

/// Render the legend explaining status indicators.
///
/// Generated from `LEGEND_STATUSES` so no variant is ever omitted and icons /
/// colors stay in sync with the canonical `StageStatus` methods automatically.
/// Items separated by a dimmed middle dot, indented to match the dashboard.
fn render_legend<W: Write>(w: &mut W) -> std::io::Result<()> {
    let dot = format!(" {} ", "·".dimmed());
    let parts: Vec<String> = LEGEND_STATUSES
        .iter()
        .map(|s| format!("{} {}", status_indicator(s), s.label()))
        .collect();
    writeln!(w, "{ROW_INDENT}{}", parts.join(&dot))?;
    Ok(())
}

#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;
