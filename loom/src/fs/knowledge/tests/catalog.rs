use crate::fs::knowledge::catalog::*;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
mod source_refs;
fn section(heading: &str, body_lines: usize) -> String {
    format!("## {heading}\n{}", "detail\n".repeat(body_lines))
}

fn small_sections(count: usize) -> String {
    (0..count)
        .map(|index| section(&format!("Part {index}"), 3))
        .collect()
}

fn write_ordering_fixture(root: &Path) {
    fs::create_dir_all(root.join("topics")).unwrap();
    fs::write(root.join("INDEX.md"), "x".repeat(12_289)).unwrap();
    fs::write(
        root.join("a.md"),
        format!(
            "## Repeat\n## repeat\n[Missing](missing.md)\n{}",
            section("Large", 40)
        ),
    )
    .unwrap();
    fs::write(root.join("b.md"), small_sections(63)).unwrap();
    fs::write(root.join("d.md"), "## Source\n`src/missing.rs`\n").unwrap();
    fs::write(
        root.join("topics/generic.md"),
        "> Topic notes for the topics knowledge area.\n## Topic\n",
    )
    .unwrap();
}

#[test]
fn rule_1_walks_markdown_recursively_and_sorts_while_skipping_generated_files() {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("zdir")).unwrap();
    fs::create_dir_all(temp.path().join(".hidden")).unwrap();
    fs::write(temp.path().join("b.md"), "## B\n").unwrap();
    fs::write(temp.path().join("INDEX.md"), "## Generated\n").unwrap();
    fs::write(temp.path().join("zdir/a.md"), "## A\n").unwrap();
    fs::write(temp.path().join(".hidden/ignored.md"), "## Hidden\n").unwrap();
    let catalog = build(temp.path()).unwrap();
    assert_eq!(
        catalog
            .chunks
            .iter()
            .map(|chunk| chunk.file.clone())
            .collect::<Vec<_>>(),
        vec![PathBuf::from("b.md"), PathBuf::from("zdir/a.md")]
    );
}

#[test]
fn rule_2_passes_relative_forward_slash_paths_to_chunks() {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("architecture")).unwrap();
    fs::write(temp.path().join("architecture/topic.md"), "## Topic\n").unwrap();
    let catalog = build(temp.path()).unwrap();
    assert_eq!(
        catalog.chunks[0].file,
        PathBuf::from("architecture/topic.md")
    );
    assert!(!catalog.chunks[0].file.is_absolute());
}

#[test]
fn rule_3_hashes_sorted_chunk_identity_lines_for_revision() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("z.md"), "## Z\n").unwrap();
    fs::write(temp.path().join("a.md"), "## A\n").unwrap();
    let catalog = build(temp.path()).unwrap();
    let mut lines: Vec<_> = catalog
        .chunks
        .iter()
        .map(|chunk| format!("{}:{}\n", chunk.id, chunk.content_hash))
        .collect();
    lines.sort();
    assert_eq!(
        catalog.revision,
        hex::encode(Sha256::digest(lines.concat().as_bytes()))
    );
}

#[test]
fn rule_4_is_deterministic_for_identical_bytes() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("notes.md"), "## Title\n`src/none.rs`\n").unwrap();
    let first = build(temp.path()).unwrap();
    let second = build(temp.path()).unwrap();
    assert_eq!(first.revision, second.revision);
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
}

#[test]
fn rule_5_reports_duplicate_normalized_headings_per_file() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("notes.md"), "## Locking!\n## locking\n").unwrap();
    let catalog = build(temp.path()).unwrap();
    assert_eq!(
        catalog.issues,
        vec![CatalogIssue::DuplicateHeading {
            file: PathBuf::from("notes.md"),
            heading: "locking".to_string(),
            occurrences: 2,
        }]
    );
}

#[test]
fn rule_6_reports_scaffold_generic_blurbs() {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("architecture")).unwrap();
    let blurb = "Topic notes for the architecture knowledge area.";
    fs::write(
        temp.path().join("architecture/topic.md"),
        format!("> {blurb}\n## Topic\n"),
    )
    .unwrap();
    let catalog = build(temp.path()).unwrap();
    assert_eq!(
        catalog.issues,
        vec![CatalogIssue::GenericBlurb {
            file: PathBuf::from("architecture/topic.md"),
            blurb: blurb.to_string(),
        }]
    );
}

#[test]
fn rule_7_reports_broken_markdown_links() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("notes.md"),
        "## Topic\n[Missing](missing.md)\n",
    )
    .unwrap();
    let catalog = build(temp.path()).unwrap();
    assert_eq!(
        catalog.issues,
        vec![CatalogIssue::BrokenLink {
            file: PathBuf::from("notes.md"),
            target: "missing.md".to_string(),
        }]
    );
}

#[test]
fn rule_8_reports_missing_repository_source_paths_when_project_root_is_known() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("project/doc/loom/knowledge");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("notes.md"), "## Topic\n`src/missing.rs`\n").unwrap();
    let catalog = build(&root).unwrap();
    assert_eq!(
        catalog.issues,
        vec![CatalogIssue::MissingSourceRef {
            file: PathBuf::from("notes.md"),
            source_path: "src/missing.rs".to_string(),
        }]
    );
}

#[test]
fn rule_9_sorts_issues_by_file_kind_and_payload() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("project/doc/loom/knowledge");
    write_ordering_fixture(&root);
    assert_eq!(
        build(&root).unwrap().issues,
        vec![
            CatalogIssue::OversizedIndex { bytes: 12_289 },
            CatalogIssue::DuplicateHeading {
                file: PathBuf::from("a.md"),
                heading: "repeat".to_string(),
                occurrences: 2,
            },
            CatalogIssue::BrokenLink {
                file: PathBuf::from("a.md"),
                target: "missing.md".to_string(),
            },
            CatalogIssue::OversizedSection {
                file: PathBuf::from("a.md"),
                heading: "Large".to_string(),
                lines: 41,
            },
            CatalogIssue::OversizedFile {
                file: PathBuf::from("b.md"),
                lines: 252,
            },
            CatalogIssue::MissingSourceRef {
                file: PathBuf::from("d.md"),
                source_path: "src/missing.rs".to_string(),
            },
            CatalogIssue::GenericBlurb {
                file: PathBuf::from("topics/generic.md"),
                blurb: "Topic notes for the topics knowledge area.".to_string(),
            },
        ]
    );
}

#[test]
fn rule_10_returns_empty_catalog_for_missing_or_empty_root() {
    let temp = TempDir::new().unwrap();
    let empty = build(temp.path()).unwrap();
    let missing = build(&temp.path().join("missing")).unwrap();
    let empty_hash = hex::encode(Sha256::digest(b""));
    assert_eq!(empty.revision, empty_hash);
    assert!(empty.chunks.is_empty() && empty.issues.is_empty());
    assert_eq!(missing, empty);
}

#[test]
fn rule_11_parent_relative_link_from_a_tier2_file_still_resolves() {
    // `../concerns.md` from a tier-2 file legitimately points at a tier-1
    // file one directory up - that must NOT be reported as a broken link.
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("architecture")).unwrap();
    fs::write(temp.path().join("concerns.md"), "## Concerns\n").unwrap();
    fs::write(
        temp.path().join("architecture/topic.md"),
        "## Topic\n[Concerns](../concerns.md)\n",
    )
    .unwrap();
    let catalog = build(temp.path()).unwrap();
    assert!(catalog.issues.is_empty(), "issues: {:?}", catalog.issues);
}

#[test]
fn rule_12_absolute_link_target_is_reported_broken_not_probed() {
    let temp = TempDir::new().unwrap();
    // `elsewhere.md` genuinely exists, right next to the knowledge root:
    // `Path::join` with an absolute second argument DISCARDS the base
    // entirely (this is exactly the S2 vulnerability), so the OLD code
    // would resolve this straight to the real file and never flag it. The
    // fix must reject the absolute target outright, before that join even
    // happens.
    fs::write(temp.path().join("elsewhere.md"), "## Elsewhere\n").unwrap();
    let absolute = temp.path().join("elsewhere.md").display().to_string();
    fs::write(
        temp.path().join("notes.md"),
        format!("## Topic\n[Away]({absolute})\n"),
    )
    .unwrap();
    let catalog = build(temp.path()).unwrap();
    assert_eq!(
        catalog.issues,
        vec![CatalogIssue::BrokenLink {
            file: PathBuf::from("notes.md"),
            target: absolute,
        }]
    );
}

#[test]
fn rule_13_link_target_escaping_the_knowledge_root_is_reported_broken_not_probed() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("root");
    fs::create_dir_all(root.join("architecture")).unwrap();
    // A real file OUTSIDE the knowledge root, reachable via `..` if the OS
    // resolved the joined path rather than the target being contained
    // lexically first - `root/architecture/../../outside.md` normalizes to
    // this file, which is exactly why the OLD code (plain `Path::join`,
    // then `fs::metadata`) would have resolved it and never flagged it.
    fs::write(temp.path().join("outside.md"), "## Outside\n").unwrap();
    fs::write(
        root.join("architecture/topic.md"),
        "## Topic\n[Escape](../../outside.md)\n",
    )
    .unwrap();
    let catalog = build(&root).unwrap();
    assert_eq!(
        catalog.issues,
        vec![CatalogIssue::BrokenLink {
            file: PathBuf::from("architecture/topic.md"),
            target: "../../outside.md".to_string(),
        }]
    );
}

#[test]
fn rule_14_reports_tier1_sections_over_the_spill_threshold() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("notes.md"), section("Topic", 40)).unwrap();
    assert_eq!(
        build(temp.path()).unwrap().issues,
        vec![CatalogIssue::OversizedSection {
            file: PathBuf::from("notes.md"),
            heading: "Topic".to_string(),
            lines: 41,
        }]
    );
}

#[test]
fn rule_15_does_not_report_tier1_sections_at_or_below_the_spill_threshold() {
    let temp = TempDir::new().unwrap();
    fs::write(
        temp.path().join("notes.md"),
        format!("{}{}", section("First", 39), section("Second", 39)),
    )
    .unwrap();
    let catalog = build(temp.path()).unwrap();
    assert!(!catalog
        .issues
        .iter()
        .any(|issue| matches!(issue, CatalogIssue::OversizedSection { .. })));
}

#[test]
fn rule_16_reports_tier1_files_over_the_line_limit() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("notes.md"), small_sections(63)).unwrap();
    assert_eq!(
        build(temp.path()).unwrap().issues,
        vec![CatalogIssue::OversizedFile {
            file: PathBuf::from("notes.md"),
            lines: 252,
        }]
    );
}

#[test]
fn rule_17_does_not_report_oversized_tier2_topics() {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("architecture")).unwrap();
    let content = format!("{}{}", section("Large", 40), small_sections(53));
    fs::write(temp.path().join("architecture/topic.md"), content).unwrap();
    let catalog = build(temp.path()).unwrap();
    assert!(!catalog.issues.iter().any(|issue| {
        matches!(
            issue,
            CatalogIssue::OversizedSection { .. } | CatalogIssue::OversizedFile { .. }
        )
    }));
}

#[test]
fn rule_18_reports_only_indexes_over_the_byte_limit() {
    let oversized = TempDir::new().unwrap();
    fs::write(oversized.path().join("INDEX.md"), "x".repeat(12_289)).unwrap();
    assert_eq!(
        build(oversized.path()).unwrap().issues,
        vec![CatalogIssue::OversizedIndex { bytes: 12_289 }]
    );

    let small = TempDir::new().unwrap();
    fs::write(small.path().join("INDEX.md"), "small index").unwrap();
    assert!(!build(small.path())
        .unwrap()
        .issues
        .iter()
        .any(|issue| matches!(issue, CatalogIssue::OversizedIndex { .. })));
}

/// Write an `INDEX.md` of exactly `bytes` bytes, asserting the size actually
/// landed so the boundary test cannot rot into testing a different size.
fn index_of_exactly(root: &Path, bytes: usize) {
    let path = root.join("INDEX.md");
    fs::write(&path, "x".repeat(bytes)).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().len(), bytes as u64);
}

#[test]
fn rule_19_reports_index_only_strictly_over_the_byte_boundary() {
    let at = TempDir::new().unwrap();
    index_of_exactly(at.path(), 12_288);
    assert!(build(at.path()).unwrap().issues.is_empty());

    let over = TempDir::new().unwrap();
    index_of_exactly(over.path(), 12_289);
    assert_eq!(
        build(over.path()).unwrap().issues,
        vec![CatalogIssue::OversizedIndex { bytes: 12_289 }]
    );
}

#[test]
fn rule_20_reports_tier1_file_only_strictly_over_the_line_boundary() {
    let at = TempDir::new().unwrap();
    assert_eq!("line\n".repeat(250).lines().count(), 250);
    fs::write(at.path().join("notes.md"), "line\n".repeat(250)).unwrap();
    assert!(build(at.path()).unwrap().issues.is_empty());

    let over = TempDir::new().unwrap();
    assert_eq!("line\n".repeat(251).lines().count(), 251);
    fs::write(over.path().join("notes.md"), "line\n".repeat(251)).unwrap();
    assert_eq!(
        build(over.path()).unwrap().issues,
        vec![CatalogIssue::OversizedFile {
            file: PathBuf::from("notes.md"),
            lines: 251,
        }]
    );
}
