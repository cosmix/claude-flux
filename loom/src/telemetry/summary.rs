//! Pure aggregation of telemetry records for CLI reporting.

use std::collections::{btree_map::Entry, BTreeMap};

use chrono::{DateTime, Utc};
use serde::Serialize;

use super::{TelemetryEvent, TelemetryRecord};

/// Stable bucket for prompt/pull events emitted outside a stage session.
pub const CHECKOUT_KEY: &str = "checkout";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AbstentionSummary {
    pub total: usize,
    pub by_reason: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StageTelemetrySummary {
    pub stage_id: String,
    pub spawn_briefs: usize,
    pub spawn_items: usize,
    pub prompt_briefs: usize,
    pub prompt_abstained: AbstentionSummary,
    pub pulls: usize,
    pub pull_avg_budget: usize,
    pub pull_avg_items: usize,
    pub pull_unmet: usize,
    pub last_at: DateTime<Utc>,
}

#[derive(Default)]
struct Totals {
    spawn_briefs: usize,
    spawn_items: usize,
    prompt_briefs: usize,
    prompt_abstained: AbstentionSummary,
    pulls: usize,
    pull_budget: usize,
    pull_items: usize,
    pull_unmet: usize,
    last_at: Option<DateTime<Utc>>,
}

/// Aggregate records by stage (or [`CHECKOUT_KEY`]), sorted by the bucket key.
pub fn summarize(records: &[TelemetryRecord]) -> Vec<StageTelemetrySummary> {
    let mut stages: BTreeMap<String, Totals> = BTreeMap::new();
    for record in records {
        let totals = stages.entry(stage_key(&record.event)).or_default();
        totals.last_at = Some(match totals.last_at {
            Some(last) => last.max(record.at),
            None => record.at,
        });
        add_event(totals, &record.event);
    }
    stages
        .into_iter()
        .filter_map(|(stage_id, totals)| finish(stage_id, totals))
        .collect()
}

fn stage_key(event: &TelemetryEvent) -> String {
    match event {
        TelemetryEvent::ContextDelivered { stage_id, .. }
        | TelemetryEvent::ContextUnavailable { stage_id, .. } => stage_id.clone(),
        TelemetryEvent::PromptBrief { stage_id, .. }
        | TelemetryEvent::PromptAbstained { stage_id, .. }
        | TelemetryEvent::ContextPulled { stage_id, .. } => {
            stage_id.as_deref().unwrap_or(CHECKOUT_KEY).to_string()
        }
    }
}

fn add_event(totals: &mut Totals, event: &TelemetryEvent) {
    match event {
        TelemetryEvent::ContextDelivered { items, .. } => {
            totals.spawn_briefs = totals.spawn_briefs.saturating_add(1);
            totals.spawn_items = totals.spawn_items.saturating_add(*items);
        }
        TelemetryEvent::ContextUnavailable { .. } => {}
        TelemetryEvent::PromptBrief { .. } => {
            totals.prompt_briefs = totals.prompt_briefs.saturating_add(1);
        }
        TelemetryEvent::PromptAbstained { reason, .. } => {
            totals.prompt_abstained.total = totals.prompt_abstained.total.saturating_add(1);
            match totals.prompt_abstained.by_reason.entry(reason.clone()) {
                Entry::Occupied(mut count) => {
                    let value = count.get_mut();
                    *value = (*value).saturating_add(1);
                }
                Entry::Vacant(count) => {
                    count.insert(1);
                }
            }
        }
        TelemetryEvent::ContextPulled {
            budget_tokens,
            items,
            unmet_required,
            ..
        } => {
            totals.pulls = totals.pulls.saturating_add(1);
            totals.pull_budget = totals.pull_budget.saturating_add(*budget_tokens);
            totals.pull_items = totals.pull_items.saturating_add(*items);
            totals.pull_unmet = totals.pull_unmet.saturating_add(*unmet_required);
        }
    }
}

fn finish(stage_id: String, totals: Totals) -> Option<StageTelemetrySummary> {
    let last_at = totals.last_at?;
    let pull_avg_budget = totals.pull_budget.checked_div(totals.pulls).unwrap_or(0);
    let pull_avg_items = totals.pull_items.checked_div(totals.pulls).unwrap_or(0);
    Some(StageTelemetrySummary {
        stage_id,
        spawn_briefs: totals.spawn_briefs,
        spawn_items: totals.spawn_items,
        prompt_briefs: totals.prompt_briefs,
        prompt_abstained: totals.prompt_abstained,
        pulls: totals.pulls,
        pull_avg_budget,
        pull_avg_items,
        pull_unmet: totals.pull_unmet,
        last_at,
    })
}
