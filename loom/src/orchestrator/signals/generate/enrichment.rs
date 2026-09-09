use crate::models::stage::{Stage, StageType};
use crate::verify::transitions::load_stage;
use std::fs;
use std::path::Path;

/// True when `stage` is an enrichment target: integration-verify or
/// knowledge-distill with at least one dependency to summarize.
pub(super) fn wants_stage_enrichment(stage: &Stage) -> bool {
    matches!(
        stage.stage_type,
        StageType::IntegrationVerify | StageType::KnowledgeDistill
    ) && !stage.dependencies.is_empty()
}

/// Build a cross-stage change summary: aggregates each dependency's file
/// assignments and metadata into a bird's-eye view for integration-verify agents.
pub(super) fn build_cross_stage_summary(work_dir: &Path, stage: &Stage) -> Option<String> {
    if !wants_stage_enrichment(stage) {
        return None;
    }

    let mut summary = String::from("## Cross-Stage Changes\n\n");
    let mut has_content = false;
    let mut all_files: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();

    for dep_id in &stage.dependencies {
        match load_stage(dep_id, work_dir) {
            Ok(dep_stage) => {
                has_content = true;
                summary.push_str(&format!(
                    "### Stage: {} ({})\n",
                    dep_stage.name,
                    format_stage_status(&dep_stage.status)
                ));
                summary.push_str(&format!("Branch: loom/{dep_id}\n"));

                if !dep_stage.files.is_empty() {
                    summary.push_str("Files:\n");
                    for file in &dep_stage.files {
                        summary.push_str(&format!("- {file}\n"));
                        all_files
                            .entry(file.clone())
                            .or_default()
                            .push(dep_id.clone());
                    }
                }
                summary.push('\n');
            }
            Err(_) => {
                // Stage file not found or unreadable - skip gracefully
            }
        }
    }

    if !has_content {
        return None;
    }

    append_file_concerns(&mut summary, &all_files);

    Some(summary)
}

/// Format a stage status for display
fn format_stage_status(status: &crate::models::stage::StageStatus) -> &'static str {
    use crate::models::stage::StageStatus;
    match status {
        StageStatus::Completed => "completed",
        StageStatus::Executing => "executing",
        StageStatus::Queued => "queued",
        StageStatus::WaitingForDeps => "waiting",
        StageStatus::Blocked => "blocked",
        StageStatus::NeedsHandoff => "needs-handoff",
        StageStatus::WaitingForInput => "waiting-for-input",
        StageStatus::MergeConflict => "merge-conflict",
        StageStatus::Skipped => "skipped",
        StageStatus::CompletedWithFailures => "completed-with-failures",
        StageStatus::MergeBlocked => "merge-blocked",
        StageStatus::NeedsHumanReview => "needs-human-review",
        StageStatus::NeedsAdjudication => "needs-adjudication",
    }
}

/// Build a wiring checklist for integration-verify by extracting wiring-related
/// notes from completed dependency stages' memory entries.
pub(super) fn build_wiring_checklist(work_dir: &Path, stage: &Stage) -> Option<String> {
    if !wants_stage_enrichment(stage) {
        return None;
    }

    let mut checklist = String::from("## Downstream Wiring Checklist\n\n");
    let mut has_items = false;

    for dep_id in &stage.dependencies {
        let memory_path = work_dir.join("memory").join(format!("{dep_id}.md"));

        let content = match fs::read_to_string(&memory_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let stage_name = load_stage(dep_id, work_dir)
            .map(|s| s.name)
            .unwrap_or_else(|_| dep_id.clone());

        let stage_items = wiring_items(&content);

        if !stage_items.is_empty() {
            has_items = true;
            checklist.push_str(&format!("From stage '{stage_name}':\n"));
            for item in stage_items {
                checklist.push_str(&format!("- [ ] {item}\n"));
            }
            checklist.push('\n');
        }
    }

    if !has_items {
        return None;
    }

    Some(checklist)
}

fn append_file_concerns(
    summary: &mut String,
    all_files: &std::collections::HashMap<String, Vec<String>>,
) {
    // Identify files touched by multiple stages
    let multi_stage_files: Vec<_> = all_files
        .iter()
        .filter(|(_, stages)| stages.len() > 1)
        .collect();

    let new_file_count: usize = all_files.values().filter(|s| s.len() == 1).count();

    if !multi_stage_files.is_empty() || new_file_count > 0 {
        summary.push_str("### Potential Concerns\n");
        for (file, stages) in &multi_stage_files {
            summary.push_str(&format!(
                "- `{}` modified by {} stages — verify no conflicts\n",
                file,
                stages.len()
            ));
        }
        if new_file_count > 0 {
            summary.push_str(&format!(
                "- {} new file(s) added — verify all are wired\n",
                new_file_count
            ));
        }
        summary.push('\n');
    }
}

fn wiring_items(content: &str) -> Vec<String> {
    // Keywords indicating wiring-relevant notes
    let wiring_keywords = [
        "needs", "wire", "wiring", "register", "mount", "import", "add to", "connect",
    ];
    let mut stage_items: Vec<String> = Vec::new();

    for line in content.lines() {
        let lower = line.to_lowercase();
        if wiring_keywords.iter().any(|kw| lower.contains(kw)) {
            // Strip common markdown prefixes for cleaner display
            let stripped = line
                .trim_start_matches('-')
                .trim_start_matches('*')
                .trim_start_matches('#')
                .trim();
            if !stripped.is_empty() {
                stage_items.push(stripped.to_string());
            }
        }
    }

    stage_items
}
