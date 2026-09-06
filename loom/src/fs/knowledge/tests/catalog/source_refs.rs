//! Focused coverage for Cargo-aware source-reference resolution.

use super::*;

#[test]
fn rule_8a_accepts_module_relative_paths_from_a_declared_workspace_member() {
    let temp = TempDir::new().unwrap();
    let project = temp.path().join("project");
    let root = project.join("doc/loom/knowledge");
    let crate_root = project.join("crates/core");
    fs::create_dir_all(crate_root.join("src/models")).unwrap();
    fs::create_dir_all(&root).unwrap();
    fs::write(
        project.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/core\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    fs::write(
        crate_root.join("Cargo.toml"),
        "[package]\nname = \"core\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        crate_root.join("src/models/constants.rs"),
        "pub const DEFAULT: u8 = 1;\n",
    )
    .unwrap();
    fs::write(
        root.join("notes.md"),
        "## Topic\n`models/constants.rs`\n`models/missing.rs`\n",
    )
    .unwrap();

    let catalog = build(&root).unwrap();

    assert_eq!(
        catalog.issues,
        vec![CatalogIssue::MissingSourceRef {
            file: PathBuf::from("notes.md"),
            source_path: "models/missing.rs".to_string(),
        }],
        "the module-relative form must resolve through its declared source root; a truly missing sibling remains an issue"
    );
}

#[test]
fn rule_8b_reports_ambiguous_module_relative_paths_across_workspace_members() {
    let temp = TempDir::new().unwrap();
    let project = temp.path().join("project");
    let root = project.join("doc/loom/knowledge");
    fs::create_dir_all(&root).unwrap();
    fs::write(
        project.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/alpha\", \"crates/beta\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    for package in ["alpha", "beta"] {
        let crate_root = project.join("crates").join(package);
        fs::create_dir_all(crate_root.join("src/models")).unwrap();
        fs::write(
            crate_root.join("Cargo.toml"),
            format!("[package]\nname = \"{package}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        )
        .unwrap();
        fs::write(
            crate_root.join("src/models/constants.rs"),
            "pub const VALUE: u8 = 1;\n",
        )
        .unwrap();
    }
    fs::write(root.join("notes.md"), "## Topic\n`models/constants.rs`\n").unwrap();

    let catalog = build(&root).unwrap();

    assert_eq!(
        catalog.issues,
        vec![CatalogIssue::MissingSourceRef {
            file: PathBuf::from("notes.md"),
            source_path: "models/constants.rs".to_string(),
        }],
        "an ambiguous module-relative path must not be assigned to either workspace member"
    );
}

#[test]
fn rule_8c_bare_basename_resolves_against_any_project_file() {
    let temp = TempDir::new().unwrap();
    let project = temp.path().join("project");
    let root = project.join("doc/loom/knowledge");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(project.join("hooks")).unwrap();
    fs::write(project.join("hooks/commit-guard.sh"), "#!/bin/sh\n").unwrap();
    fs::write(root.join("notes.md"), "## Topic\n`commit-guard.sh`\n").unwrap();

    let catalog = build(&root).unwrap();

    assert!(catalog.issues.is_empty(), "issues: {:?}", catalog.issues);
}

#[test]
fn rule_8d_bare_basename_absent_anywhere_stays_reported() {
    let temp = TempDir::new().unwrap();
    let project = temp.path().join("project");
    let root = project.join("doc/loom/knowledge");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("notes.md"), "## Topic\n`nowhere.sh`\n").unwrap();

    let catalog = build(&root).unwrap();

    assert_eq!(
        catalog.issues,
        vec![CatalogIssue::MissingSourceRef {
            file: PathBuf::from("notes.md"),
            source_path: "nowhere.sh".to_string(),
        }]
    );
}

#[test]
fn rule_8e_unique_suffix_path_resolves() {
    let temp = TempDir::new().unwrap();
    let project = temp.path().join("project");
    let root = project.join("doc/loom/knowledge");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(project.join("loom/src/context/rank/corpus")).unwrap();
    fs::write(
        project.join("loom/src/context/rank/corpus/stopwords.rs"),
        "// stopwords\n",
    )
    .unwrap();
    fs::write(
        root.join("notes.md"),
        "## Topic\n`rank/corpus/stopwords.rs`\n",
    )
    .unwrap();

    let catalog = build(&root).unwrap();

    assert!(catalog.issues.is_empty(), "issues: {:?}", catalog.issues);
}

#[test]
fn rule_8f_ambiguous_suffix_path_stays_reported() {
    let temp = TempDir::new().unwrap();
    let project = temp.path().join("project");
    let root = project.join("doc/loom/knowledge");
    fs::create_dir_all(&root).unwrap();
    for crate_name in ["alpha", "beta"] {
        let dir = project.join("crates").join(crate_name).join("tests");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("fixture.rs"), "// fixture\n").unwrap();
    }
    fs::write(root.join("notes.md"), "## Topic\n`tests/fixture.rs`\n").unwrap();

    let catalog = build(&root).unwrap();

    assert_eq!(
        catalog.issues,
        vec![CatalogIssue::MissingSourceRef {
            file: PathBuf::from("notes.md"),
            source_path: "tests/fixture.rs".to_string(),
        }],
        "two equally-plausible matches must not be assigned to either crate"
    );
}

#[test]
fn rule_8g_files_under_target_are_never_matched() {
    let temp = TempDir::new().unwrap();
    let project = temp.path().join("project");
    let root = project.join("doc/loom/knowledge");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(project.join("target/debug")).unwrap();
    fs::write(project.join("target/debug/build.rs"), "// build script\n").unwrap();
    fs::write(root.join("notes.md"), "## Topic\n`debug/build.rs`\n").unwrap();

    let catalog = build(&root).unwrap();

    assert_eq!(
        catalog.issues,
        vec![CatalogIssue::MissingSourceRef {
            file: PathBuf::from("notes.md"),
            source_path: "debug/build.rs".to_string(),
        }],
        "build output under target/ must never satisfy a source reference"
    );
}
