//! Heartbeat-derived facts folded into a stage's `StageSummary`.

use chrono::Utc;

use crate::fs::work_dir::WorkDir;
use crate::models::constants::STALENESS_THRESHOLD_SECS;
use crate::models::session::{Session, SessionStatus};
use crate::models::stage::{Stage, StageStatus, StatusBucket};
use crate::orchestrator::monitor::heartbeat::{judge_heartbeat_path, read_heartbeat, Heartbeat};

use super::sanitize::valid_stage_id;
use super::{execution_models_for_stage, ActivityStatus};

/// Read one of a stage's heartbeat files from the heartbeat directory.
///
/// Both callers join `stage_id` into the path, and the id comes from a stage
/// file's frontmatter, so it is validated here: an id carrying `../` would
/// otherwise make the daemon read an arbitrary `*.json` and put its strings in
/// the payload every subscriber renders.
fn read_stage_heartbeat(stage_id: &str, path: &std::path::Path) -> Option<Heartbeat> {
    if !valid_stage_id(stage_id) || !path.exists() {
        return None;
    }
    read_heartbeat(path).ok()
}

fn read_heartbeat_for_stage(stage_id: &str, work_dir: &WorkDir) -> Option<Heartbeat> {
    let path = work_dir
        .root()
        .join("heartbeat")
        .join(format!("{stage_id}.json"));
    read_stage_heartbeat(stage_id, &path)
}

fn read_judge_heartbeat_for_stage(stage_id: &str, work_dir: &WorkDir) -> Option<Heartbeat> {
    read_stage_heartbeat(stage_id, &judge_heartbeat_path(work_dir.root(), stage_id))
}

/// Calculate activity status from session state, heartbeat staleness, and the
/// stage's own status. `stage_status` matters for the no-session case: no
/// session while the stage sits somewhere idle is unremarkable, but no
/// session while the stage claims `Executing` means the tracking data itself
/// is missing (killed daemon, lost session file) — that is `Orphaned`, not
/// `Idle`, and the dashboard must not render it as a quiet agent. It also
/// matters once the stage is finished: a `Completed`/`Skipped` stage has no
/// agent working it regardless of what its last session file says. Likewise a
/// session that ended without crashing (`SessionStatus::is_terminal` and not
/// `Crashed`) is neither working nor hung, whatever the stage's own status.
fn determine_activity_status(
    session: Option<&Session>,
    staleness_secs: Option<u64>,
    stage_status: &StageStatus,
) -> ActivityStatus {
    match (session, staleness_secs) {
        // No session, but the stage claims to be running - the session
        // record is missing, not merely quiet.
        (None, _) if *stage_status == StageStatus::Executing => ActivityStatus::Orphaned,
        // No session and the stage isn't claiming to run - idle.
        (None, _) => ActivityStatus::Idle,
        // Session crashed
        (Some(s), _) if s.status == SessionStatus::Crashed => ActivityStatus::Error,
        // A finished stage has no agent: whatever its last session file says,
        // the work is over.
        (_, _) if stage_status.bucket() == StatusBucket::Completed => ActivityStatus::Idle,
        // A session that ended without crashing is not working and not hung.
        (Some(s), _) if s.status.is_terminal() => ActivityStatus::Idle,
        // Session running but stale heartbeat (> 5 minutes)
        (Some(_), Some(secs)) if secs > STALENESS_THRESHOLD_SECS => ActivityStatus::Stale,
        // Session running with recent heartbeat
        (Some(_), _) => ActivityStatus::Working,
    }
}

/// Heartbeat-derived facts for a stage's [`StageSummary`]: staleness, current
/// activity, and the last recorded tool/activity strings. Extracted from
/// `build_stage_summary` to keep that function within the line limit.
pub(super) struct HeartbeatFacts {
    pub(super) staleness_secs: Option<u64>,
    pub(super) activity_status: ActivityStatus,
    pub(super) last_tool: Option<String>,
    pub(super) last_activity: Option<String>,
}

pub(super) struct StageExtras {
    pub(super) execution_models: Vec<String>,
    pub(super) judge_heartbeat_secs: Option<u64>,
}

pub(super) fn stage_extras(stage: &Stage, work_dir: &WorkDir) -> StageExtras {
    let judge_heartbeat_secs = read_judge_heartbeat_for_stage(&stage.id, work_dir).map(|hb| {
        Utc::now()
            .signed_duration_since(hb.timestamp)
            .num_seconds()
            .max(0) as u64
    });
    StageExtras {
        execution_models: execution_models_for_stage(work_dir, &stage.id),
        judge_heartbeat_secs,
    }
}

pub(super) fn heartbeat_facts(
    stage: &Stage,
    session: Option<&Session>,
    work_dir: &WorkDir,
) -> HeartbeatFacts {
    let heartbeat = read_heartbeat_for_stage(&stage.id, work_dir);

    // Calculate staleness (seconds since last heartbeat)
    let staleness_secs = heartbeat.as_ref().map(|hb| {
        let age = Utc::now().signed_duration_since(hb.timestamp);
        age.num_seconds().max(0) as u64
    });

    // Determine activity status based on session, heartbeat, and stage status
    let activity_status = determine_activity_status(session, staleness_secs, &stage.status);

    let last_tool = heartbeat.as_ref().and_then(|hb| hb.last_tool.clone());
    let last_activity = heartbeat.as_ref().and_then(|hb| hb.activity.clone());

    HeartbeatFacts {
        staleness_secs,
        activity_status,
        last_tool,
        last_activity,
    }
}
