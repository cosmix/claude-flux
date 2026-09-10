//! `loom knowledge telemetry` — summarize context delivery and retrieval events.

use std::io::{self, Write};
use std::path::Path;

use anyhow::Result;

use crate::context::untrusted::inline_safe;
use crate::context::ContextPack;
use crate::fs::work_dir::WorkDir;
use crate::telemetry::summary::{self, StageTelemetrySummary};

pub const HUMAN_HEADER: &str = "stage | briefs (items) | prompt briefs emitted / abstained (top reason) | pulls (avg budget, avg items, unmet) | last event";

/// Print telemetry for every stage, or only `stage` when supplied.
pub fn telemetry(stage: Option<String>, json: bool) -> anyhow::Result<()> {
    let work_dir = WorkDir::new(".")?;
    render_telemetry(work_dir.root(), stage.as_deref(), json, &mut io::stdout())
}

fn render_telemetry(
    work_dir: &Path,
    stage: Option<&str>,
    json: bool,
    output: &mut dyn Write,
) -> Result<()> {
    let records = crate::telemetry::read_events(work_dir)?;
    let mut stages = summary::summarize(&records);
    if let Some(stage_id) = stage {
        stages.retain(|summary| summary.stage_id == stage_id);
    }
    if json {
        serde_json::to_writer_pretty(&mut *output, &serde_json::json!({ "stages": stages }))?;
        writeln!(output)?;
    } else {
        render_human(output, &stages)?;
    }
    Ok(())
}

fn render_human(output: &mut dyn Write, stages: &[StageTelemetrySummary]) -> Result<()> {
    if stages.is_empty() {
        writeln!(output, "no telemetry recorded")?;
        return Ok(());
    }
    writeln!(output, "{HUMAN_HEADER}")?;
    for stage in stages {
        let reason = top_reason(stage)
            .map(|(reason, count)| format!("{}: {count}", inline_safe(reason)))
            .unwrap_or_else(|| "-".to_string());
        writeln!(
            output,
            "{} | {} ({}) | {} / {} ({}) | {} ({}, {}, {}) | {}",
            inline_safe(&stage.stage_id),
            stage.spawn_briefs,
            stage.spawn_items,
            stage.prompt_briefs,
            stage.prompt_abstained.total,
            reason,
            stage.pulls,
            stage.pull_avg_budget,
            stage.pull_avg_items,
            stage.pull_unmet,
            stage.last_at.to_rfc3339(),
        )?;
    }
    Ok(())
}

/// Best-effort telemetry for a completed `loom knowledge context` pull.
///
/// Never fails the caller: a retrieval that already succeeded must not be
/// undone by a telemetry write it has no stake in. Skipped silently — same as
/// `commands::hook::user_prompt::emit_no_target`'s identical guard — when the
/// work dir cannot be resolved or does not exist yet: a read-only retrieval in
/// a checkout that was never `loom init`ed must not create a
/// `.loom/work/telemetry/` tree as a side effect of answering one query.
pub(crate) fn emit_context_pulled(
    stage: &Option<String>,
    query_chars: usize,
    budget_tokens: usize,
    context_pack: &ContextPack,
) {
    let Ok(work_dir) = WorkDir::new(".") else {
        return;
    };
    if !work_dir.root().exists() {
        return;
    }
    let session_id = std::env::var("LOOM_SESSION_ID")
        .ok()
        .filter(|id| !id.trim().is_empty());
    let _ = crate::telemetry::emit(
        work_dir.root(),
        &crate::telemetry::TelemetryEvent::ContextPulled {
            stage_id: stage.as_ref().cloned(),
            session_id,
            query_chars,
            budget_tokens,
            items: context_pack.items.len(),
            estimated_tokens: context_pack.estimated_tokens,
            unmet_required: context_pack.unmet_required.len(),
        },
    );
}

fn top_reason(stage: &StageTelemetrySummary) -> Option<(&str, usize)> {
    stage
        .prompt_abstained
        .by_reason
        .iter()
        .max_by(|(left_reason, left_count), (right_reason, right_count)| {
            left_count
                .cmp(right_count)
                .then_with(|| right_reason.cmp(left_reason))
        })
        .map(|(reason, count)| (reason.as_str(), *count))
}

#[cfg(test)]
#[path = "tests_telemetry.rs"]
mod tests;
