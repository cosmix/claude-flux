use crate::fs::knowledge::catalog::{build, CatalogIssue};
use crate::fs::knowledge::frontmatter;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn commit_all(root: &Path, message: &str) {
    git(root, &["add", "."]);
    git(
        root,
        &[
            "-c",
            "user.name=Loom Test",
            "-c",
            "user.email=loom@example.invalid",
            "commit",
            "-m",
            message,
        ],
    );
}

#[test]
fn evidence_changed_fires_only_for_a_source_that_changed_since_verified() {
    let temp = TempDir::new().unwrap();
    let project = temp.path();
    let knowledge = project.join("doc/loom/knowledge");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::create_dir_all(&knowledge).unwrap();
    fs::write(project.join("src/changed.rs"), "pub const VALUE: u8 = 1;\n").unwrap();
    fs::write(project.join("src/stable.rs"), "pub const VALUE: u8 = 1;\n").unwrap();
    git(project, &["init", "-q"]);
    commit_all(project, "initial sources");
    let verified = git(project, &["rev-parse", "HEAD"]);

    let topic = knowledge.join("architecture.md");
    fs::write(&topic, "# Architecture\n\n## Topic\nBody.\n").unwrap();
    frontmatter::update_file(&topic, |metadata| {
        metadata.sources = vec!["src/changed.rs".into(), "src/stable.rs".into()];
        metadata.verified = Some(verified.clone());
        Ok(())
    })
    .unwrap();
    commit_all(project, "record verification");
    fs::write(project.join("src/changed.rs"), "pub const VALUE: u8 = 2;\n").unwrap();
    commit_all(project, "change one source");

    let catalog = build(&knowledge).unwrap();

    let changed: Vec<_> = catalog
        .issues
        .iter()
        .filter_map(|issue| match issue {
            CatalogIssue::EvidenceChanged { source_path, .. } => Some(source_path.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(changed, vec!["src/changed.rs"]);
}

#[test]
fn strict_ignores_review_only_issues() {
    let issues = [
        CatalogIssue::EvidenceChanged {
            file: PathBuf::from("architecture.md"),
            source_path: "src/a.rs".into(),
            verified: "0123456789abcdef".into(),
        },
        CatalogIssue::UnverifiableReference {
            file: PathBuf::from("architecture.md"),
            source_path: "foo.rs".into(),
            kind: "example".into(),
        },
    ];

    assert_eq!(
        issues
            .iter()
            .filter(|issue| !issue.is_review_only())
            .count(),
        0
    );
}
