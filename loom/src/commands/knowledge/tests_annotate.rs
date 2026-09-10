use super::annotate::{annotate_path, parse_state, Annotation};
use crate::context::schema::LifecycleState;
use crate::fs::knowledge::frontmatter;
use std::fs;
use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn annotate_writes_frontmatter_without_touching_the_body() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("topic.md");
    let body = "# Topic\n\n## Details\nThe body stays byte-identical.\n";
    fs::write(
        &path,
        format!("---\nowner: docs\nsources: [src/old.rs]\n---\n{body}"),
    )
    .unwrap();
    let annotation = Annotation {
        state: Some(LifecycleState::Draft),
        sources: vec!["loom/src/context/pack.rs".into()],
        clear_sources: true,
        aliases: vec!["packing".into()],
        ..Default::default()
    };

    annotate_path(&path, &annotation).unwrap();

    let updated = fs::read_to_string(&path).unwrap();
    assert!(updated.contains("owner: docs\n"));
    assert!(updated.ends_with(body));
    let metadata = frontmatter::file_frontmatter(updated.as_bytes());
    assert_eq!(metadata.state, Some(LifecycleState::Draft));
    assert_eq!(metadata.sources, vec!["loom/src/context/pack.rs"]);
    assert_eq!(metadata.aliases, vec!["packing"]);
}

#[test]
fn annotate_verified_head_resolves_the_full_revision() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let path = root.join("topic.md");
    fs::write(&path, "# Topic\n\nBody.\n").unwrap();
    git(root, &["init", "-q"]);
    git(root, &["add", "topic.md"]);
    git(
        root,
        &[
            "-c",
            "user.name=Loom Test",
            "-c",
            "user.email=loom@example.invalid",
            "commit",
            "-m",
            "initial",
        ],
    );
    let head = git(root, &["rev-parse", "HEAD"]);
    let annotation = Annotation {
        verified: Some(super::annotate::resolve_revision(root, "HEAD").unwrap()),
        ..Default::default()
    };

    annotate_path(&path, &annotation).unwrap();

    let updated = fs::read(&path).unwrap();
    assert_eq!(frontmatter::file_frontmatter(&updated).verified, Some(head));
}

#[test]
fn annotate_with_only_a_blurb_does_not_introduce_a_frontmatter_block() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("topic.md");
    fs::write(
        &path,
        "# Topic\n\n> Old description.\n\n## Details\nBody.\n",
    )
    .unwrap();
    let annotation = Annotation {
        blurb: Some("Concise current description.".into()),
        ..Default::default()
    };

    annotate_path(&path, &annotation).unwrap();

    let updated = fs::read_to_string(&path).unwrap();
    assert!(!updated.starts_with("---"));
    assert_eq!(updated.lines().next(), Some("# Topic"));
}

#[test]
fn annotate_rejects_an_unknown_state() {
    let error = parse_state("forgotten").unwrap_err();

    assert!(error
        .to_string()
        .contains("Unknown knowledge state 'forgotten'"));
}

#[test]
fn annotate_blurb_rewrites_the_index_description_line() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("topic.md");
    fs::write(
        &path,
        "# Topic\n\n> Old description.\n\n## Details\nBody.\n",
    )
    .unwrap();
    let annotation = Annotation {
        blurb: Some("Concise current description.".into()),
        ..Default::default()
    };

    annotate_path(&path, &annotation).unwrap();

    let updated = fs::read_to_string(path).unwrap();
    assert!(updated.contains("\n> Concise current description.\n"));
    assert!(!updated.contains("Old description."));
}
