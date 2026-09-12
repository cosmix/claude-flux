//! Read final persisted states, including stages completed outside active sessions.
use super::OrchestratorResult;
use crate::verify::transitions::load_stage;
use crate::{models::stage::StageStatus, plan::ExecutionGraph};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::path::Path;

pub(super) fn collect_result(
    work_dir: &Path,
    graph: &ExecutionGraph,
    spawned: usize,
    started_at: DateTime<Utc>,
) -> Result<OrchestratorResult> {
    let mut result = OrchestratorResult {
        completed_stages: Vec::new(),
        failed_stages: Vec::new(),
        needs_handoff: Vec::new(),
        total_sessions_spawned: spawned,
        started_at,
        completed_at: Utc::now(),
    };
    for node in graph.all_nodes() {
        let stage = load_stage(&node.id, work_dir)
            .with_context(|| format!("Failed to read final state of stage '{}'", node.id))?;
        match stage.status {
            StageStatus::Completed => result.completed_stages.push(stage.id),
            StageStatus::Skipped => {}
            StageStatus::NeedsHandoff => result.needs_handoff.push(stage.id),
            _ => result.failed_stages.push(stage.id),
        }
    }
    result.completed_stages.sort();
    result.failed_stages.sort();
    result.needs_handoff.sort();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::stage::Stage;
    use crate::verify::transitions::serialize_stage_to_markdown;
    use std::fs;
    use tempfile::TempDir;

    fn fixture() -> (TempDir, ExecutionGraph) {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("stages")).unwrap();
        let definition = serde_yaml::from_str("id: test\nname: Test\nworking_dir: .\n").unwrap();
        (dir, ExecutionGraph::build(vec![definition]).unwrap())
    }

    fn write_status(root: &Path, status: StageStatus) {
        let mut stage = Stage::new("Test".into(), None);
        stage.id = "test".into();
        stage.status = status;
        fs::write(
            root.join("stages/test.md"),
            serialize_stage_to_markdown(&stage).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn recovered_block_and_handoff_do_not_poison_final_result() {
        let (dir, graph) = fixture();
        for status in [StageStatus::Blocked, StageStatus::NeedsHandoff] {
            write_status(dir.path(), status);
            assert!(!collect_result(dir.path(), &graph, 1, Utc::now())
                .unwrap()
                .is_success());
            write_status(dir.path(), StageStatus::Completed);
            let result = collect_result(dir.path(), &graph, 2, Utc::now()).unwrap();
            assert!(result.is_success());
            assert_eq!(result.completed_stages, ["test"]);
        }
    }

    #[test]
    fn unfinished_or_unreadable_stage_cannot_report_success() {
        let (dir, graph) = fixture();
        write_status(dir.path(), StageStatus::Queued);
        assert!(!collect_result(dir.path(), &graph, 0, Utc::now())
            .unwrap()
            .is_success());
        fs::write(dir.path().join("stages/test.md"), "broken").unwrap();
        assert!(collect_result(dir.path(), &graph, 0, Utc::now()).is_err());
    }
}
