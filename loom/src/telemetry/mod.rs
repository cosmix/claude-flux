//! Best-effort orchestration and context telemetry.
//!
//! Events normally append to `.loom/work/telemetry/events.jsonl`. Sandboxed
//! stage sessions cannot write through the worktree's state-root symlink, so a
//! denied direct write falls back to a per-worktree spool which the daemon
//! drains into the canonical event file.

use std::fs::OpenOptions;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

pub mod spool;
pub mod summary;
pub use spool::TELEMETRY_SPOOL_RELPATH;

/// One recorded orchestration fact. Counts are estimates, never savings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum TelemetryEvent {
    ContextDelivered {
        stage_id: String,
        session_id: String,
        context_epoch: String,
        items: usize,
    },
    ContextUnavailable {
        stage_id: String,
        session_id: String,
        reason: String,
    },
    PromptBrief {
        stage_id: Option<String>,
        session_id: Option<String>,
        items: usize,
        estimated_tokens: usize,
        omitted: usize,
    },
    PromptAbstained {
        stage_id: Option<String>,
        session_id: Option<String>,
        reason: String,
    },
    ContextPulled {
        stage_id: Option<String>,
        session_id: Option<String>,
        query_chars: usize,
        budget_tokens: usize,
        items: usize,
        estimated_tokens: usize,
        unmet_required: usize,
    },
}

/// A timestamped event as stored in the telemetry JSON-lines files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetryRecord {
    pub at: DateTime<Utc>,
    #[serde(flatten)]
    pub event: TelemetryEvent,
}

pub(crate) fn events_path(work_dir: &Path) -> PathBuf {
    work_dir.join("telemetry").join("events.jsonl")
}

/// Append `event` to the canonical event file, or its worktree spool when the
/// state root is write-denied.
///
/// Best-effort by contract: telemetry must never fail a caller, so failures on
/// both paths are logged at debug level and returned as success.
pub fn emit(work_dir: &Path, event: &TelemetryEvent) -> Result<()> {
    let record = TelemetryRecord {
        at: Utc::now(),
        event: event.clone(),
    };
    if let Err(error) = append_record(work_dir, &record) {
        if spool::is_write_denied(&error) {
            if let Err(spool_error) = spool_denied_record(&record) {
                tracing::debug!(%error, %spool_error, "failed to spool telemetry event");
            }
        } else {
            tracing::debug!(%error, "failed to record telemetry event");
        }
    }
    Ok(())
}

/// Append one already-timestamped record under an exclusive lock.
pub(crate) fn append_record(work_dir: &Path, record: &TelemetryRecord) -> Result<()> {
    let path = events_path(work_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("Failed to create telemetry directory: {}", parent.display())
        })?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("Failed to open telemetry events: {}", path.display()))?;
    file.lock_exclusive()
        .with_context(|| format!("Failed to lock telemetry events: {}", path.display()))?;
    let line = serde_json::to_string(record).context("Failed to serialize telemetry record")?;
    writeln!(file, "{line}")
        .with_context(|| format!("Failed to append telemetry event: {}", path.display()))?;
    Ok(())
}

fn spool_denied_record(record: &TelemetryRecord) -> Result<()> {
    let cwd = std::env::current_dir().context("Failed to get current directory")?;
    let worktree_root = crate::git::worktree::find_worktree_root_from_cwd(&cwd)
        .context("Telemetry write was denied outside a stage worktree")?;
    spool::append_to_spool(&worktree_root, record)
}

/// Read every well-formed event record, skipping malformed lines.
pub fn read_events(work_dir: &Path) -> Result<Vec<TelemetryRecord>> {
    let path = events_path(work_dir);
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to open telemetry events: {}", path.display()))
        }
    };
    file.lock_shared()
        .with_context(|| format!("Failed to lock telemetry events: {}", path.display()))?;
    let mut content = String::new();
    (&file)
        .read_to_string(&mut content)
        .with_context(|| format!("Failed to read telemetry events: {}", path.display()))?;
    Ok(content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;
    use tempfile::TempDir;

    fn delivered(stage_id: &str) -> TelemetryEvent {
        TelemetryEvent::ContextDelivered {
            stage_id: stage_id.to_string(),
            session_id: "session-1".to_string(),
            context_epoch: "abc123".to_string(),
            items: 3,
        }
    }

    fn record(at: i64, event: TelemetryEvent) -> TelemetryRecord {
        TelemetryRecord {
            at: Utc.timestamp_opt(at, 0).single().unwrap(),
            event,
        }
    }

    fn prompt_brief() -> TelemetryEvent {
        TelemetryEvent::PromptBrief {
            stage_id: Some("stage-a".to_string()),
            session_id: Some("session-1".to_string()),
            items: 2,
            estimated_tokens: 100,
            omitted: 1,
        }
    }

    fn prompt_abstained() -> TelemetryEvent {
        TelemetryEvent::PromptAbstained {
            stage_id: Some("stage-a".to_string()),
            session_id: Some("session-1".to_string()),
            reason: "floor".to_string(),
        }
    }

    fn context_pulled() -> TelemetryEvent {
        TelemetryEvent::ContextPulled {
            stage_id: Some("stage-a".to_string()),
            session_id: Some("session-1".to_string()),
            query_chars: 12,
            budget_tokens: 600,
            items: 4,
            estimated_tokens: 300,
            unmet_required: 1,
        }
    }

    #[test]
    fn events_round_trip_with_timestamps() {
        let temp = TempDir::new().unwrap();
        let expected = record(1_700_000_000, delivered("stage-a"));

        append_record(temp.path(), &expected).unwrap();

        let events = read_events(temp.path()).unwrap();
        assert_eq!(events, vec![expected]);
    }

    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn emit_falls_back_to_the_spool_when_the_state_root_is_read_only() {
        use std::os::unix::fs::PermissionsExt as _;

        let temp = TempDir::new().unwrap();
        let worktree = temp.path().join(".worktrees").join("stage-a");
        std::fs::create_dir_all(&worktree).unwrap();
        let work_dir = temp.path().join("state");
        let telemetry_dir = work_dir.join("telemetry");
        std::fs::create_dir_all(&telemetry_dir).unwrap();
        std::fs::set_permissions(&telemetry_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let original_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&worktree).unwrap();

        let result = emit(&work_dir, &delivered("stage-a"));

        std::env::set_current_dir(original_cwd).unwrap();
        std::fs::set_permissions(&telemetry_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_ok());
        let pending = spool::read_pending(&worktree).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].event, delivered("stage-a"));
    }

    #[test]
    fn drain_moves_spooled_lines_and_truncates() {
        let worktree = TempDir::new().unwrap();
        let work_dir = TempDir::new().unwrap();
        let expected = record(10, delivered("stage-a"));
        spool::append_to_spool(worktree.path(), &expected).unwrap();
        let path = spool::spool_path(worktree.path());
        writeln!(
            OpenOptions::new().append(true).open(&path).unwrap(),
            "not-json"
        )
        .unwrap();

        let outcome = spool::drain_into_events(work_dir.path(), worktree.path()).unwrap();

        assert_eq!(outcome.drained, 1);
        assert_eq!(outcome.skipped_malformed, 1);
        assert_eq!(read_events(work_dir.path()).unwrap(), vec![expected]);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "");
    }

    const VICTIM: &str = "outside the worktree; a drain must never read or truncate this\n";

    #[test]
    fn drain_refuses_a_spool_symlinked_outside_the_worktree() {
        let worktree = TempDir::new().unwrap();
        let work_dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let victim = outside.path().join("victim.txt");
        std::fs::write(&victim, VICTIM).unwrap();
        std::fs::create_dir_all(worktree.path().join(".loom")).unwrap();
        std::os::unix::fs::symlink(&victim, spool::spool_path(worktree.path())).unwrap();

        let error = spool::drain_into_events(work_dir.path(), worktree.path()).unwrap_err();

        assert!(
            format!("{error:#}").contains("was not drained"),
            "{error:#}"
        );
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), VICTIM);
    }

    #[test]
    fn drain_refuses_a_spool_under_a_symlinked_loom_directory() {
        let worktree = TempDir::new().unwrap();
        let work_dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let victim = outside.path().join("telemetry-spool.jsonl");
        std::fs::write(&victim, VICTIM).unwrap();
        std::os::unix::fs::symlink(outside.path(), worktree.path().join(".loom")).unwrap();

        assert!(spool::drain_into_events(work_dir.path(), worktree.path()).is_err());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), VICTIM);
    }

    #[test]
    fn summary_counts_briefs_abstentions_and_pulls_per_stage() {
        let events = vec![
            record(1, delivered("stage-a")),
            record(2, prompt_brief()),
            record(3, prompt_abstained()),
            record(4, context_pulled()),
        ];

        let summaries = summary::summarize(&events);

        assert_eq!(summaries.len(), 1);
        let summary = &summaries[0];
        assert_eq!(summary.stage_id, "stage-a");
        assert_eq!((summary.spawn_briefs, summary.spawn_items), (1, 3));
        assert_eq!(summary.prompt_briefs, 1);
        assert_eq!(summary.prompt_abstained.total, 1);
        assert_eq!(summary.prompt_abstained.by_reason["floor"], 1);
        assert_eq!(summary.pulls, 1);
        assert_eq!((summary.pull_avg_budget, summary.pull_avg_items), (600, 4));
        assert_eq!(summary.pull_unmet, 1);
        assert_eq!(summary.last_at, Utc.timestamp_opt(4, 0).single().unwrap());
    }
}
