mod collector;
mod execution_models;
mod heartbeat_facts;
mod sanitize;
mod timing;

pub use collector::{collect_status_data, load_all_sessions};
pub use execution_models::execution_models_for_stage;

use serde::{Deserialize, Serialize};

// Re-export types that consumers will need
pub use crate::models::failure::FailureInfo;
pub use crate::models::session::{SessionBackendKind, SessionType};
pub use crate::models::stage::{StageStatus, StageType};

/// Activity status derived from heartbeat and session state
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum ActivityStatus {
    /// No active session, the session ended without crashing, or the stage
    /// is finished (Completed/Skipped).
    #[default]
    Idle,
    /// Session is actively working (recent heartbeat)
    Working,
    /// Session encountered an error or crashed
    Error,
    /// Session may be hung (no recent heartbeat but PID alive)
    Stale,
    /// Stage status claims Executing but no session record exists for it —
    /// e.g. the daemon was killed and restarted while the file that would
    /// have named the running agent's session was never written or was lost.
    /// Distinct from `Idle`: there is nothing quiet here, the tracking data
    /// itself is missing.
    Orphaned,
}

impl ActivityStatus {
    /// Get Unicode icon for this activity status
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Idle => "\u{23F3}",      // hourglass
            Self::Working => "\u{1F504}",  // arrows counterclockwise
            Self::Error => "\u{274C}",     // cross mark
            Self::Stale => "\u{26A0}",     // warning
            Self::Orphaned => "\u{1F480}", // skull
        }
    }

    /// Get a short label for this status
    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "IDLE",
            Self::Working => "WORKING",
            Self::Error => "ERROR",
            Self::Stale => "STALE",
            Self::Orphaned => "ORPHANED",
        }
    }
}

/// Main struct aggregating all displayable status information
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StatusData {
    pub stages: Vec<StageSummary>,
    pub merge: MergeSummary,
    pub progress: ProgressSummary,
    /// Extracted plan name (first H1 header from the plan file)
    pub plan_name: Option<String>,
    /// Cached provider quota usage, read independently of the poll that
    /// populates it - always present on the wire, `None` per-provider when
    /// no cache entry exists yet.
    #[serde(default)]
    pub quota: crate::quota::QuotaSnapshot,
}

/// Stage display data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageSummary {
    pub id: String,
    pub name: String,
    pub status: StageStatus,
    /// Type of stage (standard, knowledge, integration-verify)
    #[serde(default)]
    pub stage_type: StageType,
    pub dependencies: Vec<String>,
    /// Resident context tokens for the active session.
    pub context_tokens: Option<u32>,
    /// Wall-clock life of the stage: from its first start to completion, or to
    /// now while still open. `None` until the stage has started.
    pub elapsed_secs: Option<i64>,
    /// Accumulated execution time (excludes wait/backoff time). For an
    /// `Executing` stage this includes the attempt currently in flight, not
    /// just the time banked by prior attempts.
    pub execution_secs: Option<i64>,
    pub base_branch: Option<String>,
    pub base_merged_from: Vec<String>,
    pub failure_info: Option<FailureInfo>,
    /// Activity status derived from heartbeat
    pub activity_status: ActivityStatus,
    /// Last tool used (from heartbeat)
    pub last_tool: Option<String>,
    /// Human-readable activity description
    pub last_activity: Option<String>,
    /// Seconds since last heartbeat (for staleness detection)
    pub staleness_secs: Option<u64>,
    /// Resolved context ceiling for the active session.
    pub context_ceiling_tokens: Option<u32>,
    /// Reason the stage was flagged for human review
    pub review_reason: Option<String>,
    /// Whether stage changes have been merged to the merge point
    pub merged: bool,
    /// `merged` was asserted by `--assume-merged`, not set by a merge loom performed.
    #[serde(default)]
    pub merge_assumed: bool,
    /// Why the post-merge cleanup failed, if it did (worktree/branch still on disk)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleanup_warning: Option<String>,
    /// Whether the stage is held
    pub held: bool,
    /// Current retry count
    pub retry_count: u32,
    /// Maximum retries allowed
    pub max_retries: Option<u32>,
    /// Session PID (if executing)
    pub pid: Option<u32>,
    /// Whether the session process is alive
    pub session_alive: bool,
    /// Effective model name for this stage (explicit override or stage-type default)
    pub model: String,
    /// The kind of session named by `stage.session`, if any. An `Executing`
    /// stage whose session is not of its own worker kind (e.g. an
    /// adjudication session) is not describing a working agent — see
    /// `incoherence`.
    pub session_type: Option<SessionType>,
    /// Why an `Executing` stage does not describe a working agent, if it
    /// does not. `None` for every other stage.
    pub incoherence: Option<String>,
    /// Distinct execution-model display names observed for this stage's subagents,
    /// in first-seen order (spawn ledger, then codex ledger). Empty until a
    /// subagent spawns.
    #[serde(default)]
    pub execution_models: Vec<String>,
    /// Disputes filed against this stage's acceptance criteria.
    #[serde(default)]
    pub dispute_count: u32,
    /// Age in seconds of the adjudication session's heartbeat for this stage.
    /// `None` when no judge has written one.
    #[serde(default)]
    pub judge_heartbeat_secs: Option<u64>,
    /// Which terminal backend hosts this stage's session, when one is known.
    #[serde(default)]
    pub session_backend: Option<crate::models::session::SessionBackendKind>,
}

impl StageSummary {
    /// The word a merged stage wears: `assumed` when an operator asserted the
    /// merge with `--assume-merged`, `merged` when loom performed it.
    pub fn merge_label(&self) -> &'static str {
        if self.merge_assumed {
            "assumed"
        } else {
            "merged"
        }
    }
}

/// Session display data (test-only)
#[cfg(test)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub stage_id: Option<String>,
    pub pid: Option<u32>,
    pub context_tokens: u32,
    pub uptime_secs: i64,
    pub is_alive: bool,
}

/// Merge state summary
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MergeSummary {
    pub merged: Vec<String>,
    pub pending: Vec<String>,
    pub conflicts: Vec<String>,
}

/// Progress counts
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProgressSummary {
    pub total: usize,
    pub completed: usize,
    pub executing: usize,
    pub pending: usize,
    pub blocked: usize,
}
