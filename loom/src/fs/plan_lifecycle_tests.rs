use super::*;
use tempfile::TempDir;

fn create_test_work_dir(temp_dir: &TempDir) -> WorkDir {
    let work_dir = WorkDir::new(temp_dir.path()).unwrap();
    work_dir.initialize().unwrap();
    work_dir
}

fn create_plan_file(temp_dir: &TempDir, filename: &str) -> PathBuf {
    let plans_dir = temp_dir.path().join("doc/plans");
    fs::create_dir_all(&plans_dir).unwrap();
    let plan_path = plans_dir.join(filename);
    fs::write(&plan_path, "# Test Plan\n\nPlan content").unwrap();
    plan_path
}

fn write_config(work_dir: &WorkDir, plan_path: &std::path::Path) {
    let config_content = format!(
        "[plan]\nsource_path = \"{}\"\nplan_id = \"test\"\nplan_name = \"Test\"\nbase_branch = \"main\"\n",
        plan_path.display()
    );
    fs::write(work_dir.root().join("config.toml"), config_content).unwrap();
}

fn create_stage_file(work_dir: &WorkDir, stage_id: &str, merged: bool) {
    let stages_dir = work_dir.root().join("stages");
    fs::create_dir_all(&stages_dir).unwrap();
    let content = format!(
        "---\nid: {stage_id}\nname: Test Stage\nstatus: Completed\nmerged: {merged}\n---\n# Stage\n"
    );
    fs::write(stages_dir.join(format!("0-{stage_id}.md")), content).unwrap();
}

// Filename operations tests
#[test]
fn test_add_prefix_to_filename() {
    let path = PathBuf::from("doc/plans/PLAN-feature.md");
    let result = add_prefix_to_filename(&path, IN_PROGRESS_PREFIX);
    assert_eq!(
        result,
        PathBuf::from("doc/plans/IN_PROGRESS-PLAN-feature.md")
    );
}

#[test]
fn test_add_prefix_preserves_nested_path() {
    let path = PathBuf::from("/home/user/project/doc/plans/PLAN-auth.md");
    let result = add_prefix_to_filename(&path, DONE_PREFIX);
    assert_eq!(
        result,
        PathBuf::from("/home/user/project/doc/plans/DONE-PLAN-auth.md")
    );
}

#[test]
fn test_remove_prefix_from_filename() {
    let path = PathBuf::from("doc/plans/IN_PROGRESS-PLAN-feature.md");
    let result = remove_prefix_from_filename(&path, IN_PROGRESS_PREFIX);
    assert_eq!(result, PathBuf::from("doc/plans/PLAN-feature.md"));
}

#[test]
fn test_remove_prefix_not_present() {
    let path = PathBuf::from("doc/plans/PLAN-feature.md");
    let result = remove_prefix_from_filename(&path, IN_PROGRESS_PREFIX);
    assert_eq!(result, PathBuf::from("doc/plans/PLAN-feature.md"));
}

#[test]
fn test_has_prefix() {
    assert!(has_prefix(
        Path::new("doc/plans/IN_PROGRESS-PLAN.md"),
        IN_PROGRESS_PREFIX
    ));
    assert!(!has_prefix(
        Path::new("doc/plans/PLAN.md"),
        IN_PROGRESS_PREFIX
    ));
    assert!(has_prefix(Path::new("doc/plans/DONE-PLAN.md"), DONE_PREFIX));
}

// Merge status tests
#[test]
fn test_all_stages_merged_empty_stages_dir() {
    let temp_dir = TempDir::new().unwrap();
    let work_dir = create_test_work_dir(&temp_dir);
    // No stages directory

    let result = all_stages_merged(&work_dir).unwrap();

    assert!(!result); // Empty = not merged
}

#[test]
fn test_all_stages_merged_ignores_non_markdown() {
    let temp_dir = TempDir::new().unwrap();
    let work_dir = create_test_work_dir(&temp_dir);

    let stages_dir = work_dir.root().join("stages");
    fs::create_dir_all(&stages_dir).unwrap();
    fs::write(stages_dir.join("readme.txt"), "Not a stage").unwrap();

    // With only non-markdown files, returns false (no stages found)
    let result = all_stages_merged(&work_dir).unwrap();
    assert!(!result);
}

#[test]
fn test_all_stages_merged_true() {
    let temp_dir = TempDir::new().unwrap();
    let work_dir = create_test_work_dir(&temp_dir);

    create_stage_file(&work_dir, "stage-1", true);
    create_stage_file(&work_dir, "stage-2", true);

    let result = all_stages_merged(&work_dir).unwrap();
    assert!(result);
}

#[test]
fn test_all_stages_merged_false_when_one_not_merged() {
    let temp_dir = TempDir::new().unwrap();
    let work_dir = create_test_work_dir(&temp_dir);

    create_stage_file(&work_dir, "stage-1", true);
    create_stage_file(&work_dir, "stage-2", false);

    let result = all_stages_merged(&work_dir).unwrap();
    assert!(!result);
}

// Plan lifecycle tests
#[test]
fn test_mark_plan_in_progress_renames_file() {
    let temp_dir = TempDir::new().unwrap();
    let work_dir = create_test_work_dir(&temp_dir);
    let plan_path = create_plan_file(&temp_dir, "PLAN-feature.md");
    write_config(&work_dir, &plan_path);

    let result = mark_plan_in_progress(&work_dir).unwrap();

    assert!(result.is_some());
    let new_path = result.unwrap();
    assert!(new_path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("IN_PROGRESS-"));
    assert!(new_path.exists());
    assert!(!plan_path.exists()); // Original file should be gone
}

#[test]
fn test_mark_plan_in_progress_updates_config() {
    let temp_dir = TempDir::new().unwrap();
    let work_dir = create_test_work_dir(&temp_dir);
    let plan_path = create_plan_file(&temp_dir, "PLAN-feature.md");
    write_config(&work_dir, &plan_path);

    mark_plan_in_progress(&work_dir).unwrap();

    // Verify config was updated
    let new_source_path = get_plan_source_path(&work_dir).unwrap().unwrap();
    assert!(new_source_path.to_str().unwrap().contains("IN_PROGRESS-"));
}

#[test]
fn test_mark_plan_in_progress_idempotent() {
    let temp_dir = TempDir::new().unwrap();
    let work_dir = create_test_work_dir(&temp_dir);
    let plan_path = create_plan_file(&temp_dir, "IN_PROGRESS-PLAN-feature.md");
    write_config(&work_dir, &plan_path);

    let result = mark_plan_in_progress(&work_dir).unwrap();

    assert!(result.is_none()); // No rename needed
    assert!(plan_path.exists()); // File unchanged
}

#[test]
fn test_mark_plan_in_progress_skips_done_plans() {
    let temp_dir = TempDir::new().unwrap();
    let work_dir = create_test_work_dir(&temp_dir);
    let plan_path = create_plan_file(&temp_dir, "DONE-PLAN-feature.md");
    write_config(&work_dir, &plan_path);

    let result = mark_plan_in_progress(&work_dir).unwrap();

    assert!(result.is_none()); // No rename for DONE plans
    assert!(plan_path.exists()); // File unchanged
}

#[test]
fn test_mark_plan_in_progress_no_config() {
    let temp_dir = TempDir::new().unwrap();
    let work_dir = create_test_work_dir(&temp_dir);
    // No config.toml created

    let result = mark_plan_in_progress(&work_dir).unwrap();

    assert!(result.is_none());
}

#[test]
fn test_mark_plan_done_when_all_merged() {
    let temp_dir = TempDir::new().unwrap();
    let work_dir = create_test_work_dir(&temp_dir);
    let plan_path = create_plan_file(&temp_dir, "IN_PROGRESS-PLAN-feature.md");
    write_config(&work_dir, &plan_path);

    // Create merged stage files
    create_stage_file(&work_dir, "stage-1", true);
    create_stage_file(&work_dir, "stage-2", true);

    let result = mark_plan_done_if_all_merged(&work_dir).unwrap();

    assert!(result.is_some());
    let new_path = result.unwrap();
    assert!(new_path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("DONE-"));
    assert!(!new_path.to_str().unwrap().contains("IN_PROGRESS"));
    assert!(new_path.exists());
}

#[test]
fn test_mark_plan_done_skips_when_not_all_merged() {
    let temp_dir = TempDir::new().unwrap();
    let work_dir = create_test_work_dir(&temp_dir);
    let plan_path = create_plan_file(&temp_dir, "IN_PROGRESS-PLAN-feature.md");
    write_config(&work_dir, &plan_path);

    // Create one merged and one not merged
    create_stage_file(&work_dir, "stage-1", true);
    create_stage_file(&work_dir, "stage-2", false);

    let result = mark_plan_done_if_all_merged(&work_dir).unwrap();

    assert!(result.is_none()); // Should not rename
    assert!(plan_path.exists()); // Original still exists
}

#[test]
fn test_mark_plan_done_only_processes_in_progress() {
    let temp_dir = TempDir::new().unwrap();
    let work_dir = create_test_work_dir(&temp_dir);
    let plan_path = create_plan_file(&temp_dir, "PLAN-feature.md"); // No prefix
    write_config(&work_dir, &plan_path);

    create_stage_file(&work_dir, "stage-1", true);

    let result = mark_plan_done_if_all_merged(&work_dir).unwrap();

    assert!(result.is_none()); // Should not process non-IN_PROGRESS files
}

#[test]
fn test_mark_plan_done_updates_config() {
    let temp_dir = TempDir::new().unwrap();
    let work_dir = create_test_work_dir(&temp_dir);
    let plan_path = create_plan_file(&temp_dir, "IN_PROGRESS-PLAN-feature.md");
    write_config(&work_dir, &plan_path);

    create_stage_file(&work_dir, "stage-1", true);

    mark_plan_done_if_all_merged(&work_dir).unwrap();

    // Verify config was updated
    let new_source_path = get_plan_source_path(&work_dir).unwrap().unwrap();
    assert!(new_source_path.to_str().unwrap().contains("DONE-"));
    assert!(!new_source_path.to_str().unwrap().contains("IN_PROGRESS"));
}

#[test]
fn test_full_lifecycle_plan_to_done() {
    let temp_dir = TempDir::new().unwrap();
    let work_dir = create_test_work_dir(&temp_dir);
    let original_plan = create_plan_file(&temp_dir, "PLAN-my-feature.md");
    write_config(&work_dir, &original_plan);

    // Step 1: Mark as in-progress (simulates loom run start)
    let in_progress = mark_plan_in_progress(&work_dir).unwrap().unwrap();
    assert_eq!(
        in_progress.file_name().unwrap().to_str().unwrap(),
        "IN_PROGRESS-PLAN-my-feature.md"
    );

    // Step 2: Create merged stages (simulates execution completing)
    create_stage_file(&work_dir, "stage-1", true);

    // Step 3: Mark as done (simulates successful completion)
    let done = mark_plan_done_if_all_merged(&work_dir).unwrap().unwrap();
    assert_eq!(
        done.file_name().unwrap().to_str().unwrap(),
        "DONE-PLAN-my-feature.md"
    );

    // Verify final state
    assert!(!original_plan.exists());
    assert!(!in_progress.exists());
    assert!(done.exists());
}
