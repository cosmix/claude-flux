use ratatui::text::{Line, Span};

use crate::commands::status::data::StageSummary;

use super::{
    cells::{cell_for, model_spans, padded},
    columns::gap_before,
    Column, ColumnKind,
};

/// Render one padded ledger row for `stage`.
pub fn stage_row(
    stage: &StageSummary,
    level: usize,
    cols: &[Column],
    all: &[&StageSummary],
) -> Line<'static> {
    let mut spans = Vec::new();
    for column in cols {
        spans.push(Span::raw(" ".repeat(usize::from(gap_before(column.kind)))));
        if column.kind == ColumnKind::Models {
            spans.extend(model_spans(stage, column.width));
        } else {
            let cell = cell_for(stage, level, column, all);
            spans.push(Span::styled(padded(&cell.text, column.width), cell.style));
        }
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::super::cells::{activity_cell, context_cell, model_parts};
    use super::*;
    use crate::{
        commands::status::{data::ActivityStatus, ui::tui::ledger::columns::columns_for_width},
        models::stage::{StageStatus, StageType},
    };

    fn summary(status: StageStatus) -> StageSummary {
        StageSummary {
            id: "stage".to_owned(),
            name: "stage".to_owned(),
            status,
            stage_type: StageType::Standard,
            dependencies: vec![],
            context_tokens: None,
            elapsed_secs: None,
            execution_secs: None,
            base_branch: None,
            base_merged_from: vec![],
            failure_info: None,
            activity_status: ActivityStatus::Idle,
            last_tool: None,
            last_activity: None,
            staleness_secs: None,
            context_ceiling_tokens: None,
            review_reason: None,
            merged: false,
            merge_assumed: false,
            cleanup_warning: None,
            held: false,
            retry_count: 0,
            max_retries: None,
            pid: None,
            session_alive: false,
            model: "opus".to_owned(),
            session_type: None,
            incoherence: None,
            execution_models: vec![],
            dispute_count: 0,
            judge_heartbeat_secs: None,
            session_backend: None,
        }
    }

    macro_rules! activity_case {
        ($name:ident, $status:expr, $activity:expr, $expected:expr) => {
            #[test]
            fn $name() {
                let mut stage = summary($status);
                stage.activity_status = $activity;
                assert_eq!(activity_cell(&stage, 40).text, $expected);
            }
        };
        ($name:ident, $status:expr, $activity:expr, $expected:expr, judge: $judge:expr) => {
            #[test]
            fn $name() {
                let mut stage = summary($status);
                stage.activity_status = $activity;
                stage.judge_heartbeat_secs = $judge;
                assert_eq!(activity_cell(&stage, 40).text, $expected);
            }
        };
    }

    activity_case!(
        queued_activity,
        StageStatus::Queued,
        ActivityStatus::Idle,
        "ready"
    );
    activity_case!(
        working_activity,
        StageStatus::Executing,
        ActivityStatus::Working,
        "working"
    );
    activity_case!(
        idle_activity,
        StageStatus::Executing,
        ActivityStatus::Idle,
        "idle"
    );
    activity_case!(
        stale_activity,
        StageStatus::Executing,
        ActivityStatus::Stale,
        "stale"
    );
    activity_case!(
        orphaned_activity,
        StageStatus::Executing,
        ActivityStatus::Orphaned,
        "orphaned"
    );
    activity_case!(
        error_activity,
        StageStatus::Executing,
        ActivityStatus::Error,
        "crashed"
    );
    activity_case!(
        input_activity,
        StageStatus::WaitingForInput,
        ActivityStatus::Idle,
        "awaiting input"
    );
    activity_case!(
        handoff_activity,
        StageStatus::NeedsHandoff,
        ActivityStatus::Idle,
        "handing off"
    );
    activity_case!(
        blocked_activity,
        StageStatus::Blocked,
        ActivityStatus::Idle,
        "error 0/3"
    );
    activity_case!(
        failed_activity,
        StageStatus::CompletedWithFailures,
        ActivityStatus::Idle,
        "failed 0/3"
    );
    activity_case!(
        conflict_activity,
        StageStatus::MergeConflict,
        ActivityStatus::Idle,
        "conflict"
    );
    activity_case!(
        merge_error_activity,
        StageStatus::MergeBlocked,
        ActivityStatus::Idle,
        "merge error"
    );
    activity_case!(
        review_activity,
        StageStatus::NeedsHumanReview,
        ActivityStatus::Idle,
        "awaiting you"
    );
    activity_case!(
        adjudication_activity,
        StageStatus::NeedsAdjudication,
        ActivityStatus::Idle,
        "dispute 0 · judge none"
    );
    activity_case!(
        adjudication_activity_judge_working,
        StageStatus::NeedsAdjudication,
        ActivityStatus::Idle,
        "dispute 0 · judge working",
        judge: Some(300)
    );
    activity_case!(
        adjudication_activity_judge_stale,
        StageStatus::NeedsAdjudication,
        ActivityStatus::Idle,
        "dispute 0 · judge stale",
        judge: Some(301)
    );

    #[test]
    fn activity_formats_tools_and_staleness() {
        let mut stage = summary(StageStatus::Executing);
        stage.activity_status = ActivityStatus::Working;
        stage.last_tool = Some("git status".to_owned());
        assert_eq!(activity_cell(&stage, 40).text, "working · git status");
        stage.activity_status = ActivityStatus::Idle;
        stage.staleness_secs = Some(61);
        assert_eq!(activity_cell(&stage, 40).text, "idle 1m1s");
    }

    #[test]
    fn models_keep_execution_names_that_fit() {
        let mut stage = summary(StageStatus::Executing);
        stage.execution_models = vec!["sonnet".to_owned(), "terra".to_owned()];
        let (model, execution) = model_parts(&stage, 17);
        assert_eq!(format!("{model}{execution}"), "opus›sonnet,terra");
    }

    #[test]
    fn context_cell_uses_five_cell_bar_and_rounded_percent() {
        let mut stage = summary(StageStatus::Executing);
        stage.context_tokens = Some(487_000);
        stage.context_ceiling_tokens = Some(800_000);
        assert_eq!(context_cell(&stage).text, "━━━╌╌ 61%");
    }

    #[test]
    fn long_content_row_fills_full_width() {
        let mut stage = summary(StageStatus::Executing);
        stage.id = "a-very-long-stage-identifier".repeat(3);
        stage.dependencies = vec!["a-long-dependency".repeat(3), "second".to_owned()];
        stage.execution_models = vec!["a-very-long-model".repeat(3), "terra".to_owned()];
        stage.activity_status = ActivityStatus::Working;
        stage.last_tool = Some("a-very-long-tool-name".repeat(3));
        stage.context_tokens = Some(487_000);
        stage.context_ceiling_tokens = Some(800_000);
        assert_eq!(
            stage_row(&stage, 2, &columns_for_width(120), &[]).width(),
            120
        );
    }

    #[test]
    fn empty_cells_row_fills_full_width() {
        let stage = summary(StageStatus::Completed);
        assert_eq!(
            stage_row(&stage, 0, &columns_for_width(120), &[]).width(),
            120
        );
    }
}
