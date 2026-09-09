use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use tempfile::TempDir;

use super::{ProjectProfile, ProjectType};

fn write(root: &Path, path: &str, content: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn monorepo() -> TempDir {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    write(
        temp.path(),
        "backend/Cargo.toml",
        "[package]\nname = 'backend'\n",
    );
    write(
        temp.path(),
        "web/package.json",
        r#"{"dependencies":{"react":"19"},"devDependencies":{"typescript":"6"}}"#,
    );
    fs::create_dir(temp.path().join("web/src")).unwrap();
    temp
}

fn kinds(types: &[ProjectType]) -> BTreeSet<&str> {
    types.iter().map(|kind| kind.kind.as_str()).collect()
}

#[test]
fn root_discovers_nested_packages_and_reports_evidence() {
    let repo = monorepo();
    let profile = ProjectProfile::discover(repo.path());
    assert_eq!(
        kinds(&profile.types),
        BTreeSet::from(["rust", "react", "typescript"])
    );
    assert!(profile
        .types
        .iter()
        .any(|kind| kind.kind == "rust" && kind.path == Path::new("backend")));
    assert!(!profile.truncated);
}

#[test]
fn nested_cwd_and_assignments_select_the_nearest_package() {
    let repo = monorepo();
    let cwd = repo.path().join("web/src");
    let profile = ProjectProfile::discover(&cwd);
    assert_eq!(
        kinds(&profile.for_prompt(&cwd, "please continue")),
        BTreeSet::from(["react", "typescript"])
    );
    assert_eq!(
        kinds(&profile.for_files(&["backend/src/new.rs".into()])),
        BTreeSet::from(["rust"])
    );
    assert_eq!(
        kinds(&profile.for_prompt(repo.path(), "edit web/src/new.tsx")),
        BTreeSet::from(["react", "typescript"])
    );
}

#[test]
fn directory_and_glob_assignments_include_descendant_packages() {
    let repo = monorepo();
    let profile = ProjectProfile::discover(repo.path());
    assert_eq!(
        kinds(&profile.for_files(&["web/**/*.tsx".into()])),
        BTreeSet::from(["react", "typescript"])
    );
    assert_eq!(
        kinds(&profile.for_files(&["**/*.rs".into()])),
        BTreeSet::from(["rust"])
    );
    assert_eq!(
        kinds(&profile.for_files(&["web".into()])),
        BTreeSet::from(["react", "typescript"])
    );
}

#[test]
fn plain_javascript_is_not_misclassified_as_typescript() {
    let repo = TempDir::new().unwrap();
    write(
        repo.path(),
        "package.json",
        r#"{"dependencies":{"react":"19"}}"#,
    );
    write(repo.path(), "bun.lock", "{}");
    assert_eq!(
        kinds(&ProjectProfile::discover(repo.path()).types),
        BTreeSet::from(["react"])
    );
}

#[test]
fn discovery_refreshes_after_manifest_changes_and_tolerates_bad_json() {
    let repo = TempDir::new().unwrap();
    write(repo.path(), "package.json", "{ invalid }");
    assert!(ProjectProfile::discover(repo.path()).types.is_empty());
    write(
        repo.path(),
        "package.json",
        r#"{"dependencies":{"react":"19"},"devDependencies":[]}"#,
    );
    assert_eq!(
        kinds(&ProjectProfile::discover(repo.path()).types),
        BTreeSet::from(["react"])
    );
}

#[test]
fn worktree_marker_stops_traversal_before_the_main_checkout() {
    let repo = monorepo();
    let worktree = repo.path().join(".worktrees/stage");
    write(&worktree, ".git", "gitdir: /unneeded/main/git/metadata");
    write(&worktree, "app/pyproject.toml", "[project]");
    let profile = ProjectProfile::discover(&worktree.join("app"));
    assert_eq!(profile.root, worktree.canonicalize().unwrap());
    assert_eq!(kinds(&profile.types), BTreeSet::from(["python"]));
}

#[test]
fn dependencies_and_symlinked_trees_do_not_contribute() {
    let repo = monorepo();
    let external = TempDir::new().unwrap();
    write(external.path(), "go.mod", "module outside");
    write(
        repo.path(),
        "node_modules/example/go.mod",
        "module dependency",
    );
    write(repo.path(), "target/fixture/pyproject.toml", "[project]");
    std::os::unix::fs::symlink(external.path(), repo.path().join("linked")).unwrap();
    assert_eq!(
        kinds(&ProjectProfile::discover(repo.path()).types),
        BTreeSet::from(["react", "rust", "typescript"])
    );
}

#[test]
fn outside_assignments_cannot_pull_in_other_projects() {
    let repo = monorepo();
    let profile = ProjectProfile::discover(repo.path());
    assert!(profile.for_files(&["../external/go.mod".into()]).is_empty());
    assert!(profile.for_files(&["/outside/source.rs".into()]).is_empty());
}

#[test]
fn depth_limit_is_visible_in_the_profile() {
    let repo = TempDir::new().unwrap();
    write(repo.path(), "a/b/c/d/e/f/g/h/i/j/Cargo.toml", "[package]");
    assert!(ProjectProfile::discover(repo.path()).truncated);
}

#[test]
fn unsupported_child_package_does_not_inherit_parent_workspace_language() {
    let repo = TempDir::new().unwrap();
    fs::create_dir(repo.path().join(".git")).unwrap();
    write(repo.path(), "Cargo.toml", "[workspace]");
    write(repo.path(), "tools/package.json", r#"{"name":"plain-js"}"#);
    let profile = ProjectProfile::discover(repo.path());
    assert!(profile.for_files(&["tools/index.js".into()]).is_empty());
}

#[test]
fn unrelated_slashes_in_prose_do_not_disable_project_discovery() {
    let repo = monorepo();
    let profile = ProjectProfile::discover(repo.path());
    assert_eq!(
        kinds(&profile.for_prompt(
            repo.path(),
            "review performance/correctness and https://example.org"
        )),
        kinds(&profile.types),
    );
}

#[test]
fn infrastructure_markers_remain_detectable() {
    let repo = TempDir::new().unwrap();
    write(repo.path(), "infra/kustomization.yaml", "resources: []");
    write(repo.path(), "infra/versions.tf", "terraform {}");
    write(repo.path(), "Dockerfile", "FROM scratch");
    write(repo.path(), ".github/workflows/check.yml", "name: check");
    assert_eq!(
        kinds(&ProjectProfile::discover(repo.path()).types),
        BTreeSet::from(["ci-cd", "docker", "kubernetes", "kustomize", "terraform"])
    );
}
