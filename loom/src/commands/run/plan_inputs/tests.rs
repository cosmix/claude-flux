use super::{mark_plan_in_progress, require_committed_plan};
use crate::fs::{plan_lifecycle, work_dir::WorkDir};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

const PLAN: &str = "doc/plans/PLAN-demo.md";
const ACTIVE: &str = "doc/plans/IN_PROGRESS-PLAN-demo.md";
const BRIEF: &str = "doc/plans/briefs/demo/worker.md";

fn git(root: &Path, args: &[&str]) -> String {
    let result = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", root.join("absent-global"))
        .env("GIT_CONFIG_SYSTEM", root.join("absent-system"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap().trim().to_string()
}

fn write(root: &Path, path: &str, content: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn fixture() -> (TempDir, WorkDir) {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    git(root, &["init", "-b", "main"]);
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
    ] {
        git(root, &["config", key, value]);
    }
    write(root, "seed", "seed");
    git(root, &["add", "seed"]);
    git(root, &["commit", "-m", "seed"]);
    write(root, PLAN, &format!("# Plan\nRead `{BRIEF}`.\n"));
    write(root, BRIEF, "# Implement the stage\n");
    let work = WorkDir::new(root).unwrap();
    fs::create_dir_all(work.root()).unwrap();
    configure(&work, &root.join(PLAN));
    (temp, work)
}

fn configure(work: &WorkDir, plan: &Path) {
    let mut config = toml_edit::DocumentMut::new();
    config["plan"] = toml_edit::table();
    config["plan"]["source_path"] = toml_edit::value(plan.to_str().unwrap());
    config["plan"]["base_branch"] = toml_edit::value("main");
    crate::fs::work_dir::write_config(work.root(), &config).unwrap();
}

fn commit_inputs(root: &Path) {
    git(root, &["add", "doc/plans"]);
    git(root, &["commit", "-m", "docs: add plan and briefs"]);
}

#[test]
fn rejects_untracked_inputs_without_mutating_them() {
    let (temp, work) = fixture();
    let root = temp.path();
    let before = git(root, &["status", "--porcelain"]);
    assert!(!crate::git::has_uncommitted_changes(root).unwrap());
    let error = require_committed_plan(&work).unwrap_err().to_string();
    assert!(error.contains(PLAN) && error.contains(BRIEF), "{error}");
    assert!(error.contains("must be committed"), "{error}");
    assert_eq!(git(root, &["status", "--porcelain"]), before);
    assert!(root.join(PLAN).exists());
    assert!(!root.join(ACTIVE).exists());
}

#[test]
fn rejects_ignored_and_staged_only_briefs() {
    let (temp, work) = fixture();
    let root = temp.path();
    git(root, &["add", PLAN]);
    git(root, &["commit", "-m", "plan"]);
    write(root, ".git/info/exclude", "doc/plans/briefs/\n");
    assert!(require_committed_plan(&work)
        .unwrap_err()
        .to_string()
        .contains(BRIEF));
    git(root, &["add", "-f", BRIEF]);
    assert!(require_committed_plan(&work)
        .unwrap_err()
        .to_string()
        .contains("staged only"));
}

#[test]
fn rejects_modified_and_deleted_inputs() {
    let (temp, work) = fixture();
    let root = temp.path();
    commit_inputs(root);
    write(root, BRIEF, "edited");
    assert!(require_committed_plan(&work)
        .unwrap_err()
        .to_string()
        .contains("uncommitted changes"));
    git(root, &["add", BRIEF]);
    assert!(require_committed_plan(&work).is_err());
    fs::remove_file(root.join(BRIEF)).unwrap();
    assert!(require_committed_plan(&work)
        .unwrap_err()
        .to_string()
        .contains("missing file"));
}

#[test]
fn discovers_bundle_and_transitive_references_but_allows_unrelated_drafts() {
    let (temp, work) = fixture();
    let root = temp.path();
    let nested = "doc/plans/briefs/shared/nested.md";
    write(
        root,
        BRIEF,
        &format!("|worker|[brief]({nested}#details)|\n"),
    );
    write(root, nested, &format!("Read `{BRIEF}`")); // cycle terminates
    commit_inputs(root);
    write(root, "scratch.md", "unrelated");
    write(root, "doc/plans/PLAN-other.md", "unrelated");
    write(root, "doc/plans/briefs/other/unrelated.md", "unrelated");
    require_committed_plan(&work).unwrap();
    let worker = "doc/plans/briefs/demo/extra-worker.md";
    write(root, worker, "bundle member without a direct link");
    let error = require_committed_plan(&work).unwrap_err().to_string();
    assert!(error.contains(worker), "{error}");
    write(root, nested, "modified transitive brief");
    assert!(require_committed_plan(&work)
        .unwrap_err()
        .to_string()
        .contains(nested));
}

#[test]
fn rejects_inputs_committed_only_on_another_branch() {
    let (temp, work) = fixture();
    git(temp.path(), &["switch", "-c", "plan-draft"]);
    commit_inputs(temp.path());
    let error = require_committed_plan(&work).unwrap_err().to_string();
    assert!(error.contains("base branch is 'main'"), "{error}");
}

#[test]
fn supports_relative_config_paths() {
    let (temp, work) = fixture();
    commit_inputs(temp.path());
    configure(&work, Path::new(PLAN));
    require_committed_plan(&work).unwrap();
    mark_plan_in_progress(&work).unwrap();
    assert!(temp.path().join(ACTIVE).is_file());
    require_committed_plan(&work).unwrap();
}

#[test]
fn discovers_glob_references_and_directory_contents() {
    let (temp, work) = fixture();
    let root = temp.path();
    write(
        root,
        PLAN,
        "Read doc/plans/briefs/shared/*.md and doc/plans/briefs/assets/.",
    );
    write(root, "doc/plans/briefs/shared/one.md", "worker");
    write(root, "doc/plans/briefs/assets/diagram.bin", "asset");
    commit_inputs(root);
    require_committed_plan(&work).unwrap();
    let added = "doc/plans/briefs/shared/two.md";
    write(root, added, "another worker");
    let error = require_committed_plan(&work).unwrap_err().to_string();
    assert!(error.contains(added), "{error}");
}

#[test]
fn rejects_edits_hidden_from_git_status() {
    let (temp, work) = fixture();
    let root = temp.path();
    commit_inputs(root);
    git(root, &["update-index", "--assume-unchanged", BRIEF]);
    write(root, BRIEF, "hidden edit");
    assert!(!crate::git::has_uncommitted_changes(root).unwrap());
    let error = require_committed_plan(&work).unwrap_err().to_string();
    assert!(error.contains("differs from the committed file"), "{error}");
}

#[test]
fn committed_rename_reaches_worktrees_and_allows_cleanup_and_restart() {
    let (temp, work) = fixture();
    let root = temp.path();
    commit_inputs(root);
    write(root, "scratch.md", "keep untracked");
    require_committed_plan(&work).unwrap();
    mark_plan_in_progress(&work).unwrap();
    assert_eq!(
        git(root, &["show", &format!("HEAD:{ACTIVE}")]),
        format!("# Plan\nRead `{BRIEF}`.")
    );
    assert!(git(root, &["diff", "HEAD", "--name-only"]).is_empty());
    let head = git(root, &["rev-parse", "HEAD"]);
    require_committed_plan(&work).unwrap();
    mark_plan_in_progress(&work).unwrap();
    assert_eq!(git(root, &["rev-parse", "HEAD"]), head);
    let stage = root.join("stage");
    git(
        root,
        &[
            "worktree",
            "add",
            "-b",
            "loom/stage",
            stage.to_str().unwrap(),
            "main",
        ],
    );
    assert!(stage.join(ACTIVE).is_file() && stage.join(BRIEF).is_file());
    write(&stage, "result", "done");
    git(&stage, &["add", "result"]);
    git(&stage, &["commit", "-m", "stage result"]);
    git(
        root,
        &["merge", "--no-ff", "-m", "merge stage", "loom/stage"],
    );
    git(root, &["worktree", "remove", stage.to_str().unwrap()]);
    assert!(!stage.exists());
    assert_eq!(
        fs::read_to_string(root.join("scratch.md")).unwrap(),
        "keep untracked"
    );
}

#[test]
fn refuses_rename_collision_without_overwriting() {
    let (temp, work) = fixture();
    commit_inputs(temp.path());
    write(temp.path(), ACTIVE, "precious existing file");
    require_committed_plan(&work).unwrap();
    assert!(mark_plan_in_progress(&work)
        .unwrap_err()
        .to_string()
        .contains("already exists"));
    assert_eq!(
        fs::read_to_string(temp.path().join(ACTIVE)).unwrap(),
        "precious existing file"
    );
    assert!(temp.path().join(PLAN).exists());
}

#[test]
fn rename_does_not_commit_unrelated_staged_changes() {
    let (temp, work) = fixture();
    let root = temp.path();
    commit_inputs(root);
    require_committed_plan(&work).unwrap();
    write(root, "seed", "operator edit after preflight");
    git(root, &["add", "seed"]);
    mark_plan_in_progress(&work).unwrap();
    assert_eq!(git(root, &["show", "HEAD:seed"]), "seed");
    assert_eq!(git(root, &["diff", "--cached", "--name-only"]), "seed");
}

#[test]
fn commit_failure_keeps_plan_recoverable_and_stops_startup() {
    let (temp, work) = fixture();
    let root = temp.path();
    commit_inputs(root);
    git(root, &["config", "user.email", ""]);
    git(root, &["config", "user.name", ""]);
    let head = git(root, &["rev-parse", "HEAD"]);
    let error = mark_plan_in_progress(&work).unwrap_err().to_string();
    assert!(error.contains("no stage has started"), "{error}");
    assert_eq!(git(root, &["rev-parse", "HEAD"]), head);
    let active = plan_lifecycle::get_plan_source_path(&work)
        .unwrap()
        .unwrap();
    assert!(active.is_file());
    assert!(require_committed_plan(&work).is_err());
}
