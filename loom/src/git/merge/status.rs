//! Merge status detection utilities for stages.
//!
//! This module provides utilities for detecting the merge state of completed stages,
//! including whether their work has been merged to the merge point, if there are
//! conflicts, or if branches are missing.

use anyhow::Result;
use std::path::Path;

use crate::git::branch::{branch_exists, branch_name_for_stage, is_ancestor_of};
use crate::models::stage::{Stage, StageType};

/// The merge state of a completed stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeState {
    /// Stage work has been merged to the merge point
    Merged,
    /// Stage needs merge (work not yet in merge point)
    Pending,
    /// Stage has unresolved merge conflicts
    Conflict,
    /// Stage branch is missing (deleted without being marked as merged)
    BranchMissing,
    /// Cannot determine merge state (no completed_commit - legacy stage)
    Unknown,
}

impl std::fmt::Display for MergeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MergeState::Merged => write!(f, "Merged"),
            MergeState::Pending => write!(f, "Pending"),
            MergeState::Conflict => write!(f, "Conflict"),
            MergeState::BranchMissing => write!(f, "BranchMissing"),
            MergeState::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Check the merge state of a stage.
///
/// Determines whether a stage's work has been merged to the merge point
/// by examining git ancestry (primary) and falling back to metadata.
///
/// IMPORTANT: This function always verifies via git ancestry when possible,
/// rather than trusting the `merged` flag. This prevents "phantom merges"
/// where the flag was set but code never landed.
///
/// When git ancestry is unavailable (no `completed_commit` or git command failure),
/// knowledge stages with `merged: true` return [`MergeState::Merged`] because they
/// legitimately have no branch by design. Non-knowledge stages in the same situation
/// return [`MergeState::Unknown`] to force callers to escalate rather than trusting
/// potentially stale or incorrect metadata.
///
/// # Arguments
/// * `stage` - The stage to check
/// * `merge_point` - The branch/commit to check against (e.g., "main" or merge point SHA)
/// * `repo_root` - Path to the git repository root
///
/// # Returns
/// The merge state of the stage.
pub fn check_merge_state(stage: &Stage, merge_point: &str, repo_root: &Path) -> Result<MergeState> {
    // PRIORITY 1: Git ancestry check via completed_commit (authoritative source of truth).
    //
    // This MUST run before checking the merge_conflict flag because the flag is
    // metadata that can become stale. After a merge resolution agent resolves
    // conflicts and commits, the git state is correct but the merge_conflict flag
    // in the stage file may still be true (only cleared by `loom stage merge --resolved`).
    // Without this ordering, the orchestrator cannot detect successful resolutions,
    // causing infinite re-spawning of merge sessions.
    if let Some(ref completed_commit) = stage.completed_commit {
        match is_ancestor_of(completed_commit, merge_point, repo_root) {
            Ok(true) => return Ok(MergeState::Merged),
            Ok(false) => {
                // Commit exists but not in target - check if branch still exists
                let branch_name = branch_name_for_stage(&stage.id);
                if !branch_exists(&branch_name, repo_root)? {
                    // Branch gone but commit not merged - suspicious state
                    return Ok(MergeState::BranchMissing);
                }
                // Branch exists, commit not merged yet
                if stage.merge_conflict {
                    return Ok(MergeState::Conflict);
                }
                return Ok(MergeState::Pending);
            }
            Err(_) => {
                // Git command failed - fall back to metadata below
            }
        }
    }

    // PRIORITY 2: Metadata flags (when git check unavailable or failed).
    if stage.merge_conflict {
        return Ok(MergeState::Conflict);
    }

    if stage.merged {
        // Knowledge stages legitimately have no completed_commit (no branch by design).
        if stage.stage_type == StageType::Knowledge {
            return Ok(MergeState::Merged);
        }
        // Non-knowledge stage marked merged but no verifiable commit — phantom risk.
        // Return Unknown so callers escalate rather than trusting bad metadata.
        return Ok(MergeState::Unknown);
    }

    // No completed_commit, not marked merged - we have no way to verify
    Ok(MergeState::Unknown)
}

/// Summary report of merge status across multiple stages.
#[derive(Debug, Default)]
pub struct MergeStatusReport {
    /// Stage IDs that have been merged
    pub merged: Vec<String>,
    /// Stage IDs pending merge
    pub pending: Vec<String>,
    /// Stage IDs with merge conflicts
    pub conflicts: Vec<String>,
    /// Warning messages (e.g., missing branches, unknown states)
    pub warnings: Vec<String>,
}

impl MergeStatusReport {
    /// Create a new empty report
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if there are any issues requiring attention
    pub fn has_issues(&self) -> bool {
        !self.conflicts.is_empty() || !self.warnings.is_empty()
    }

    /// Check if all tracked stages have been merged
    pub fn all_merged(&self) -> bool {
        self.pending.is_empty() && self.conflicts.is_empty()
    }

    /// Total number of stages tracked
    pub fn total(&self) -> usize {
        self.merged.len() + self.pending.len() + self.conflicts.len()
    }
}

/// Build a merge status report for a collection of stages.
///
/// # Arguments
/// * `stages` - The stages to check
/// * `merge_point` - The branch/commit to check against
/// * `repo_root` - Path to the git repository root
///
/// # Returns
/// A report summarizing the merge status of all provided stages.
pub fn build_merge_report(
    stages: &[Stage],
    merge_point: &str,
    repo_root: &Path,
) -> Result<MergeStatusReport> {
    let mut report = MergeStatusReport::new();

    for stage in stages {
        // Only check completed stages
        if stage.status != crate::models::stage::StageStatus::Completed {
            continue;
        }

        match check_merge_state(stage, merge_point, repo_root) {
            Ok(MergeState::Merged) => {
                report.merged.push(stage.id.clone());
            }
            Ok(MergeState::Pending) => {
                report.pending.push(stage.id.clone());
            }
            Ok(MergeState::Conflict) => {
                report.conflicts.push(stage.id.clone());
            }
            Ok(MergeState::BranchMissing) => {
                report.warnings.push(format!(
                    "Stage '{}' branch missing but not marked as merged",
                    stage.id
                ));
            }
            Ok(MergeState::Unknown) => {
                report.warnings.push(format!(
                    "Stage '{}' has no completed_commit - cannot determine merge state",
                    stage.id
                ));
            }
            Err(e) => {
                report.warnings.push(format!(
                    "Failed to check merge state for '{}': {}",
                    stage.id, e
                ));
            }
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::stage::{Stage, StageStatus, StageType};
    use chrono::Utc;

    fn make_test_stage(id: &str) -> Stage {
        Stage {
            id: id.to_string(),
            name: id.to_string(),
            status: StageStatus::Completed,
            completed_at: Some(Utc::now()),
            ..Stage::default()
        }
    }

    #[test]
    fn test_merge_state_conflict_flag() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().unwrap();

        let mut stage = make_test_stage("test-stage");
        stage.merge_conflict = true;

        let result = check_merge_state(&stage, "main", temp_dir.path()).unwrap();
        assert_eq!(result, MergeState::Conflict);
    }

    #[test]
    fn test_merge_state_merged_flag_standard_stage_returns_unknown() {
        // A non-knowledge stage with merged=true but no completed_commit cannot be
        // verified via git ancestry. Return Unknown to force callers to escalate
        // rather than trusting potentially stale metadata (phantom-merge guard).
        use tempfile::TempDir;
        let temp_dir = TempDir::new().unwrap();

        let mut stage = make_test_stage("test-stage");
        stage.merged = true;
        // stage_type defaults to Standard

        let result = check_merge_state(&stage, "main", temp_dir.path()).unwrap();
        assert_eq!(result, MergeState::Unknown);
    }

    #[test]
    fn test_merge_state_merged_flag_knowledge_stage_returns_merged() {
        // Knowledge stages have no branch by design, so merged=true with no
        // completed_commit is their legitimate terminal state.
        use tempfile::TempDir;
        let temp_dir = TempDir::new().unwrap();

        let mut stage = make_test_stage("test-stage");
        stage.merged = true;
        stage.stage_type = StageType::Knowledge;

        let result = check_merge_state(&stage, "main", temp_dir.path()).unwrap();
        assert_eq!(result, MergeState::Merged);
    }

    #[test]
    fn test_merge_state_no_completed_commit() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().unwrap();

        let stage = make_test_stage("test-stage");
        // completed_commit is None by default

        let result = check_merge_state(&stage, "main", temp_dir.path()).unwrap();
        assert_eq!(result, MergeState::Unknown);
    }

    #[test]
    fn test_merge_status_report_new() {
        let report = MergeStatusReport::new();
        assert!(report.merged.is_empty());
        assert!(report.pending.is_empty());
        assert!(report.conflicts.is_empty());
        assert!(report.warnings.is_empty());
        assert!(!report.has_issues());
        assert!(report.all_merged());
        assert_eq!(report.total(), 0);
    }

    #[test]
    fn test_merge_status_report_has_issues() {
        let mut report = MergeStatusReport::new();
        report.conflicts.push("stage-1".to_string());
        assert!(report.has_issues());
        assert!(!report.all_merged());
    }

    #[test]
    fn test_merge_status_report_all_merged() {
        let mut report = MergeStatusReport::new();
        report.merged.push("stage-1".to_string());
        report.merged.push("stage-2".to_string());
        assert!(report.all_merged());
        assert_eq!(report.total(), 2);
    }

    #[test]
    fn test_merge_state_display() {
        assert_eq!(format!("{}", MergeState::Merged), "Merged");
        assert_eq!(format!("{}", MergeState::Pending), "Pending");
        assert_eq!(format!("{}", MergeState::Conflict), "Conflict");
        assert_eq!(format!("{}", MergeState::BranchMissing), "BranchMissing");
        assert_eq!(format!("{}", MergeState::Unknown), "Unknown");
    }
}
