use ratatui::{backend::TestBackend, Terminal};

use super::{render, LedgerView};
use crate::commands::status::data::{
    ActivityStatus, MergeSummary, ProgressSummary, StageSummary, StatusData,
};
use crate::commands::status::render::attention_model::attention_entries;
use crate::commands::status::ui::tui::state::TuiActivityLog;
use crate::models::stage::{StageStatus, StageType, StatusBucket};
use crate::plan::graph::levels;

pub(super) fn screen(width: u16, height: u16, view: &LedgerView) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            render(frame, view);
        })
        .unwrap();

    terminal
        .backend()
        .buffer()
        .content
        .chunks(usize::from(width))
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end_matches(' ')
                .to_owned()
        })
        .collect()
}

pub(super) fn contains(rows: &[String], needle: &str) -> bool {
    rows.iter().any(|row| row.contains(needle))
}

pub(super) fn make_stage(id: &str, status: StageStatus) -> StageSummary {
    StageSummary {
        id: id.to_owned(),
        name: id.to_owned(),
        status,
        stage_type: StageType::Standard,
        dependencies: Vec::new(),
        context_tokens: None,
        elapsed_secs: None,
        execution_secs: None,
        base_branch: None,
        base_merged_from: Vec::new(),
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
        execution_models: Vec::new(),
        dispute_count: 0,
        judge_heartbeat_secs: None,
        session_backend: None,
    }
}

/// The plain (undetailed) stage, one per `StageStatus` variant exercised by the ledger tests.
fn plain_stages() -> Vec<StageSummary> {
    [
        ("s-waiting", StageStatus::WaitingForDeps),
        ("s-queued", StageStatus::Queued),
        ("s-executing", StageStatus::Executing),
        ("s-input", StageStatus::WaitingForInput),
        ("s-handoff", StageStatus::NeedsHandoff),
        ("s-completed", StageStatus::Completed),
        ("s-skipped", StageStatus::Skipped),
        ("s-blocked", StageStatus::Blocked),
        ("s-failed", StageStatus::CompletedWithFailures),
        ("s-conflict", StageStatus::MergeConflict),
        ("s-mergeblk", StageStatus::MergeBlocked),
        ("s-review", StageStatus::NeedsHumanReview),
        ("s-adjudicate", StageStatus::NeedsAdjudication),
    ]
    .into_iter()
    .map(|(id, status)| make_stage(id, status))
    .collect()
}

/// Fills in the per-stage detail (models/context, review reason, dispute count, pid)
/// that the render tests assert against.
fn apply_stage_detail(stages: &mut [StageSummary]) {
    let executing = stages
        .iter_mut()
        .find(|stage| stage.id == "s-executing")
        .unwrap();
    executing.model = "opus".to_owned();
    executing.execution_models = vec!["sonnet".to_owned(), "terra".to_owned()];
    executing.context_tokens = Some(487_000);
    executing.context_ceiling_tokens = Some(800_000);
    executing.activity_status = ActivityStatus::Working;
    executing.last_tool = Some("Edit".to_owned());

    stages
        .iter_mut()
        .find(|stage| stage.id == "s-review")
        .unwrap()
        .review_reason = Some("which wins?".to_owned());
    stages
        .iter_mut()
        .find(|stage| stage.id == "s-adjudicate")
        .unwrap()
        .dispute_count = 2;
    stages
        .iter_mut()
        .find(|stage| stage.id == "s-input")
        .unwrap()
        .pid = Some(4242);
}

fn progress_summary(stages: &[StageSummary]) -> ProgressSummary {
    let mut progress = ProgressSummary {
        total: stages.len(),
        ..ProgressSummary::default()
    };
    for stage in stages {
        match stage.status.bucket() {
            StatusBucket::Executing => progress.executing += 1,
            StatusBucket::Pending => progress.pending += 1,
            StatusBucket::Completed => progress.completed += 1,
            StatusBucket::Blocked => progress.blocked += 1,
        }
    }
    progress
}

pub(super) fn fixture() -> StatusData {
    let mut stages = plain_stages();
    apply_stage_detail(&mut stages);
    let progress = progress_summary(&stages);

    StatusData {
        stages,
        merge: MergeSummary {
            merged: vec!["s-completed".to_owned()],
            pending: Vec::new(),
            conflicts: Vec::new(),
        },
        progress,
        plan_name: Some("embed-assets-self-update".to_owned()),
        quota: crate::quota::QuotaSnapshot::default(),
    }
}

pub(super) fn render_view(
    data: &StatusData,
    width: u16,
    height: u16,
    legend_open: bool,
) -> Vec<String> {
    let levels = levels::compute_all_levels(
        &data.stages,
        |stage| stage.id.as_str(),
        |stage| &stage.dependencies,
    );
    let mut ordered: Vec<&StageSummary> = data.stages.iter().collect();
    ordered.sort_by(|left, right| {
        let left_level = levels.get(&left.id).copied().unwrap_or_default();
        let right_level = levels.get(&right.id).copied().unwrap_or_default();
        left_level
            .cmp(&right_level)
            .then_with(|| left.id.cmp(&right.id))
    });
    let attention = attention_entries(&data.stages);
    let activity = TuiActivityLog::new();
    let alerts = Vec::new();
    let view = LedgerView {
        data,
        levels: &levels,
        ordered: &ordered,
        attention: &attention,
        activity: &activity,
        alerts: &alerts,
        spinner: '⠋',
        scroll_y: 0,
        legend_open,
        tick_age_secs: Some(2),
        last_error: None,
        now_epoch: 1_788_523_200,
        scrollable: false,
    };
    screen(width, height, &view)
}

#[test]
fn renders_all_thirteen_states_at_full_width() {
    let data = fixture();
    let rows = render_view(&data, 120, 40, false);
    for status in [
        StageStatus::WaitingForDeps,
        StageStatus::Queued,
        StageStatus::Executing,
        StageStatus::WaitingForInput,
        StageStatus::NeedsHandoff,
        StageStatus::Completed,
        StageStatus::Skipped,
        StageStatus::Blocked,
        StageStatus::CompletedWithFailures,
        StageStatus::MergeConflict,
        StageStatus::MergeBlocked,
        StageStatus::NeedsHumanReview,
        StageStatus::NeedsAdjudication,
    ] {
        assert!(
            contains(&rows, status.label()),
            "missing {}",
            status.label()
        );
    }
    assert!(contains(&rows, "MODELS"));
    assert!(contains(&rows, "CONTEXT"));
    assert!(contains(&rows, "TIME"));
    assert!(rows
        .iter()
        .any(|row| row.contains("opus›sonnet+1") && row.contains(" 61%")));
    assert!(contains(&rows, "NEEDS ATTENTION"));
    // The attention panel is capped at 10 rows; only these titles fit for this
    // fixture at 120x40 (NEEDS REVIEW and ADJUDICATING fall below the cut).
    assert!(contains(&rows, "NEEDS INPUT"));
    assert!(contains(&rows, "BLOCKED"));
    assert!(contains(&rows, "ACCEPTANCE FAILED"));
    assert!(contains(&rows, "MERGE CONFLICT"));
    assert!(contains(&rows, "MERGE ERROR"));
}

#[test]
fn drops_columns_in_priority_order() {
    let data = fixture();
    let at_110 = render_view(&data, 110, 40, false);
    assert!(!contains(&at_110, "TIME"));
    assert!(!contains(&at_110, "MODELS"));
    assert!(contains(&at_110, "DEPENDS ON"));

    let at_74 = render_view(&data, 74, 40, false);
    assert!(!contains(&at_74, "CONTEXT"));
    assert!(contains(&at_74, "MERGE"));
}

#[test]
fn shows_notice_below_minimum_size() {
    for (width, height) in [(60, 20), (120, 12)] {
        let data = fixture();
        let rows = render_view(&data, width, height, false);
        assert!(contains(&rows, "at least 64×16"));
        assert!(contains(&rows, "q quit"));
        assert!(!contains(&rows, "s-waiting"));
    }
}

#[test]
fn legend_overlay_lists_every_state() {
    let data = fixture();
    let rows = render_view(&data, 120, 40, true);
    assert!(contains(&rows, "Stage states"));
    for status in [
        StageStatus::WaitingForDeps,
        StageStatus::Queued,
        StageStatus::Executing,
        StageStatus::WaitingForInput,
        StageStatus::NeedsHandoff,
        StageStatus::Completed,
        StageStatus::Skipped,
        StageStatus::Blocked,
        StageStatus::CompletedWithFailures,
        StageStatus::MergeConflict,
        StageStatus::MergeBlocked,
        StageStatus::NeedsHumanReview,
        StageStatus::NeedsAdjudication,
    ] {
        assert!(
            contains(&rows, status.label()),
            "missing {}",
            status.label()
        );
    }
}

#[test]
fn footer_lists_only_present_states() {
    let mut data = fixture();
    data.stages.retain(|stage| {
        matches!(
            stage.status,
            StageStatus::Executing | StageStatus::Completed
        )
    });
    let rows = render_view(&data, 120, 40, false);
    let footer = rows.last().unwrap();
    assert!(footer.contains("executing"));
    assert!(footer.contains("done"));
    assert!(!footer.contains("queued"));
}

#[test]
fn header_counts_match_progress() {
    let data = fixture();
    let executing = data
        .stages
        .iter()
        .filter(|stage| stage.status.bucket() == StatusBucket::Executing)
        .count();
    let queued = data
        .stages
        .iter()
        .filter(|stage| stage.status == StageStatus::Queued)
        .count();
    let waiting = data
        .stages
        .iter()
        .filter(|stage| stage.status == StageStatus::WaitingForDeps)
        .count();
    let done = data
        .stages
        .iter()
        .filter(|stage| stage.status.bucket() == StatusBucket::Completed)
        .count();
    let attention_count = attention_entries(&data.stages).len();
    let rows = render_view(&data, 120, 40, false);
    let header = &rows[2];
    assert!(header.contains(&format!("{executing} executing")));
    assert!(header.contains(&format!("{queued} queued")));
    assert!(header.contains(&format!("{waiting} waiting")));
    assert!(header.contains(&format!("{attention_count} need attention")));
    assert!(header.contains(&format!("{done} done")));
}

#[test]
fn wide_terminal_widens_stage_column() {
    let data = fixture();
    let at_120 = render_view(&data, 120, 40, false);
    let at_140 = render_view(&data, 140, 40, false);
    let header_120 = at_120
        .iter()
        .find(|row| row.contains("DEPENDS ON"))
        .unwrap();
    let header_140 = at_140
        .iter()
        .find(|row| row.contains("DEPENDS ON"))
        .unwrap();
    let start_120 = header_120.find("DEPENDS ON").unwrap();
    let start_140 = header_140.find("DEPENDS ON").unwrap();
    assert!(start_140 > start_120);
}
