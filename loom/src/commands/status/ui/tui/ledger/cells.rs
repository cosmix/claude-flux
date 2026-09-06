use ratatui::{style::Style, text::Span};

use crate::{
    commands::status::{
        data::{ActivityStatus, StageSummary},
        render::attention_model::failure_label,
        ui::theme::Theme,
    },
    models::stage::{StageStatus, StageType},
    utils::format_elapsed,
};

use super::text::{text_width, truncate};
use super::{Column, ColumnKind};

// Re-exported so `rows.rs` can keep importing `padded` from `cells`.
pub(super) use super::text::padded;

pub(super) struct Cell {
    pub(super) text: String,
    pub(super) style: Style,
}

pub(super) fn cell_for(
    stage: &StageSummary,
    level: usize,
    column: &Column,
    all: &[&StageSummary],
) -> Cell {
    match column.kind {
        ColumnKind::State => state_cell(stage, column.width),
        ColumnKind::Stage => stage_cell(stage, level),
        ColumnKind::DependsOn => dependencies_cell(stage, all, column.width),
        ColumnKind::Activity => activity_cell(stage, column.width),
        ColumnKind::Context => context_cell(stage),
        ColumnKind::Time => time_cell(stage),
        ColumnKind::Merge => merge_cell(stage),
        ColumnKind::Models => empty_cell(),
    }
}

pub(super) fn model_spans(stage: &StageSummary, width: u16) -> Vec<Span<'static>> {
    let (model, execution) = model_parts(stage, width);
    let used = text_width(&model) + text_width(&execution);
    vec![
        Span::styled(model, Theme::dimmed()),
        Span::raw(execution),
        Span::raw(" ".repeat(usize::from(width).saturating_sub(used))),
    ]
}

pub(super) fn activity_cell(stage: &StageSummary, width: u16) -> Cell {
    let cell = if matches!(&stage.status, StageStatus::Executing) && stage.incoherence.is_some() {
        Cell {
            text: "incoherent".to_owned(),
            style: Theme::status_blocked(),
        }
    } else {
        match &stage.status {
            StageStatus::Queued => Cell {
                text: "ready".to_owned(),
                style: Theme::status_queued(),
            },
            StageStatus::Executing => executing_activity(stage, width),
            StageStatus::WaitingForInput => Cell {
                text: "awaiting input".to_owned(),
                style: stage.status.tui_style(),
            },
            StageStatus::NeedsHandoff => Cell {
                text: "handing off".to_owned(),
                style: Theme::status_warning(),
            },
            StageStatus::Blocked => retry_activity(stage, blocked_label(stage)),
            StageStatus::CompletedWithFailures => retry_activity(stage, "failed"),
            StageStatus::MergeConflict => Cell {
                text: "conflict".to_owned(),
                style: Theme::status_warning(),
            },
            StageStatus::MergeBlocked => Cell {
                text: "merge error".to_owned(),
                style: Theme::status_blocked(),
            },
            StageStatus::NeedsHumanReview => Cell {
                text: "awaiting you".to_owned(),
                style: stage.status.tui_style(),
            },
            StageStatus::NeedsAdjudication => adjudication_activity(stage),
            _ => empty_cell(),
        }
    };
    held_cell(stage, cell)
}

pub(super) fn context_cell(stage: &StageSummary) -> Cell {
    let applicable = matches!(
        &stage.status,
        StageStatus::Executing | StageStatus::WaitingForInput | StageStatus::NeedsHandoff
    );
    let (Some(tokens), Some(ceiling)) = (stage.context_tokens, stage.context_ceiling_tokens) else {
        return empty_cell();
    };
    if !applicable || ceiling == 0 {
        return empty_cell();
    }
    let ratio = f64::from(tokens) / f64::from(ceiling);
    let filled = (ratio * 5.0).floor().clamp(0.0, 5.0) as usize;
    let percent = (ratio * 100.0).round() as u32;
    Cell {
        text: format!(
            "{}{} {percent:>2}%",
            "━".repeat(filled),
            "╌".repeat(5 - filled)
        ),
        style: Theme::context_style(tokens, ceiling),
    }
}

pub(super) fn model_parts(stage: &StageSummary, width: u16) -> (String, String) {
    if stage.execution_models.is_empty() {
        return (truncate(&stage.model, usize::from(width)), String::new());
    }

    let model = truncate(&stage.model, usize::from(width));
    let remaining = usize::from(width).saturating_sub(text_width(&model));
    if remaining == 0 {
        return (model, String::new());
    }
    let execution = execution_models_text(&stage.execution_models, remaining - 1);
    (model, format!("›{execution}"))
}

fn state_cell(stage: &StageSummary, width: u16) -> Cell {
    let icon = stage.status.icon();
    let icon_width = text_width(icon) as u16;
    let label_width = width.saturating_sub(icon_width + 1);
    Cell {
        text: format!("{icon} {}", padded(stage.status.label(), label_width)),
        style: stage.status.tui_style(),
    }
}

fn stage_cell(stage: &StageSummary, level: usize) -> Cell {
    let style = match &stage.status {
        StageStatus::Executing => Theme::header(),
        StageStatus::WaitingForDeps => Theme::status_pending(),
        StageStatus::Skipped => Theme::dimmed(),
        _ => Style::default(),
    };
    Cell {
        text: format!("{}{}", "  ".repeat(level), stage.id),
        style,
    }
}

fn dependencies_cell(stage: &StageSummary, all: &[&StageSummary], width: u16) -> Cell {
    let complete = stage.dependencies.iter().all(|dependency| {
        all.iter().any(|sibling| {
            sibling.id == *dependency && matches!(&sibling.status, StageStatus::Completed)
        })
    });
    let style = if complete {
        Theme::dimmed()
    } else {
        Theme::status_pending()
    };
    Cell {
        text: dependency_text(&stage.dependencies, width),
        style,
    }
}

fn dependency_text(dependencies: &[String], width: u16) -> String {
    match dependencies {
        [] => String::new(),
        [dependency] => dependency.clone(),
        _ => {
            let suffix = format!(" +{}", dependencies.len() - 1);
            let first_width = usize::from(width).saturating_sub(text_width(&suffix));
            format!("{}{}", truncate(&dependencies[0], first_width), suffix)
        }
    }
}

fn execution_models_text(models: &[String], width: usize) -> String {
    let mut shown = String::new();
    let mut count = 0;
    for model in models {
        let candidate = if shown.is_empty() {
            model.clone()
        } else {
            format!("{shown},{model}")
        };
        let hidden = models.len() - count - 1;
        let suffix = if hidden > 0 {
            format!("+{hidden}")
        } else {
            String::new()
        };
        if text_width(&candidate) + text_width(&suffix) > width {
            break;
        }
        shown = candidate;
        count += 1;
    }
    let hidden = models.len() - count;
    let text = if hidden == 0 {
        shown
    } else if shown.is_empty() {
        format!("+{hidden}")
    } else {
        format!("{shown}+{hidden}")
    };
    truncate(&text, width)
}

fn executing_activity(stage: &StageSummary, width: u16) -> Cell {
    match stage.activity_status {
        ActivityStatus::Working => {
            let text = match &stage.last_tool {
                Some(tool) => {
                    let prefix = "working · ";
                    format!(
                        "{prefix}{}",
                        truncate(tool, usize::from(width).saturating_sub(text_width(prefix)))
                    )
                }
                None => "working".to_owned(),
            };
            Cell {
                text,
                style: Theme::status_completed(),
            }
        }
        ActivityStatus::Idle => staleness_activity("idle", stage.staleness_secs, Theme::dimmed()),
        ActivityStatus::Stale => {
            staleness_activity("stale", stage.staleness_secs, Theme::status_warning())
        }
        ActivityStatus::Orphaned => Cell {
            text: "orphaned".to_owned(),
            style: Theme::status_blocked(),
        },
        ActivityStatus::Error => Cell {
            text: "crashed".to_owned(),
            style: Theme::status_blocked(),
        },
    }
}

fn staleness_activity(prefix: &str, seconds: Option<u64>, style: Style) -> Cell {
    let text = seconds.map_or_else(
        || prefix.to_owned(),
        |seconds| {
            format!(
                "{prefix} {}",
                format_elapsed(seconds.try_into().unwrap_or(i64::MAX))
            )
        },
    );
    Cell { text, style }
}

fn retry_activity(stage: &StageSummary, label: &str) -> Cell {
    let maximum = stage.max_retries.unwrap_or(3);
    Cell {
        text: format!("{label} {}/{maximum}", stage.retry_count),
        style: Theme::status_blocked(),
    }
}

fn blocked_label(stage: &StageSummary) -> &'static str {
    stage
        .failure_info
        .as_ref()
        .map(|failure| failure_label(&failure.failure_type))
        .unwrap_or("error")
}

fn adjudication_activity(stage: &StageSummary) -> Cell {
    let state = match stage.judge_heartbeat_secs {
        None => "none",
        Some(seconds) if seconds <= 300 => "working",
        Some(_) => "stale",
    };
    Cell {
        text: format!("dispute {} · judge {state}", stage.dispute_count),
        style: Theme::status_warning(),
    }
}

fn held_cell(stage: &StageSummary, cell: Cell) -> Cell {
    if stage.held && !cell.text.is_empty() {
        Cell {
            text: format!("held · {}", cell.text),
            style: Theme::status_warning(),
        }
    } else {
        cell
    }
}

fn time_cell(stage: &StageSummary) -> Cell {
    if matches!(
        &stage.status,
        StageStatus::WaitingForDeps | StageStatus::Queued
    ) {
        return empty_cell();
    }
    Cell {
        text: stage
            .execution_secs
            .or(stage.elapsed_secs)
            .map(format_elapsed)
            .unwrap_or_default(),
        style: Theme::dimmed(),
    }
}

fn merge_cell(stage: &StageSummary) -> Cell {
    match &stage.status {
        StageStatus::Completed if stage.cleanup_warning.is_some() => Cell {
            text: "cleanup!".to_owned(),
            style: Theme::status_warning(),
        },
        StageStatus::Completed if stage.stage_type != StageType::Knowledge && stage.merged => {
            Cell {
                text: stage.merge_label().to_owned(),
                style: Theme::status_merged(),
            }
        }
        StageStatus::Completed if stage.stage_type != StageType::Knowledge => Cell {
            text: "unmerged".to_owned(),
            style: Theme::status_warning(),
        },
        StageStatus::MergeConflict => Cell {
            text: "conflict".to_owned(),
            style: Theme::status_warning(),
        },
        StageStatus::MergeBlocked => Cell {
            text: "error".to_owned(),
            style: Theme::status_blocked(),
        },
        _ => empty_cell(),
    }
}

fn empty_cell() -> Cell {
    Cell {
        text: String::new(),
        style: Style::default(),
    }
}
