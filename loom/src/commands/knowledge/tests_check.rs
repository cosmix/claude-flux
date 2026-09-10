//! Tests for `commands/knowledge/check.rs`.
//!
//! The load-bearing assertion here is negative: `check` must never create
//! anything under `.loom/` (that is the whole reason it exists instead of
//! reusing `context::resolve()` — see check.rs's module doc). Every test
//! below builds its own temp project with a state directory PRESENT before
//! calling `check`; without it, `WorkDir::new(".")` searches upward past the
//! temp dir and silently resolves the real repository's state directory
//! instead (`fs/work_dir.rs`'s upward search), which would make these tests
//! check the wrong tree entirely.

use super::*;
use serial_test::serial;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

/// Marks a temp dir as its own project root for `WorkDir::new(".")`, without
/// pulling in the full state-directory initialization `WorkDir::initialize`
/// performs (irrelevant here, and it would just add unused subdirectories).
///
/// The resolver keys on `config.toml`'s presence, not directory existence
/// (see `fs/work_dir.rs`'s `workspace_at`), so an empty `config.toml` is
/// what actually pins resolution to `project_root` — a bare directory alone
/// would let the upward search walk straight past it.
fn seed_work_marker(project_root: &std::path::Path) {
    let work_dir = project_root.join(".loom").join("work");
    fs::create_dir_all(&work_dir).expect("failed to seed state dir marker");
    fs::write(work_dir.join("config.toml"), "").expect("failed to seed config.toml marker");
}

fn seed_clean_knowledge(project_root: &std::path::Path) {
    let root = project_root.join("doc/loom/knowledge");
    fs::create_dir_all(&root).expect("failed to create knowledge root");
    fs::write(
        root.join("patterns.md"),
        "# Patterns\n\n> Reusable patterns.\n\n## One\n\nSome prose.\n",
    )
    .expect("failed to write patterns.md");
}

/// A tier-1 file with the SAME `## ` heading twice — the fixture `catalog::build`
/// reports as `CatalogIssue::DuplicateHeading` (`fs/knowledge/catalog.rs`).
fn seed_knowledge_with_duplicate_heading(project_root: &std::path::Path) {
    let root = project_root.join("doc/loom/knowledge");
    fs::create_dir_all(&root).expect("failed to create knowledge root");
    fs::write(
        root.join("mistakes.md"),
        "# Mistakes\n\n> Lessons learned.\n\n## Foo\n\nFirst copy.\n\n## Foo\n\nSecond copy.\n",
    )
    .expect("failed to write mistakes.md");
}

/// No path under `.loom/` is created anywhere under the project root by a
/// `check` run — the property that keeps this command safe as a stage
/// acceptance criterion (unlike `loom knowledge sync`, see check.rs's doc).
#[test]
#[serial]
fn check_never_creates_a_loom_cache_directory() {
    let temp = TempDir::new().expect("failed to create temp dir");
    let project_root = temp.path();
    seed_work_marker(project_root);
    seed_clean_knowledge(project_root);

    let original_dir = std::env::current_dir().expect("failed to get current dir");
    std::env::set_current_dir(project_root).expect("failed to change dir");
    let result = check(false, false);
    std::env::set_current_dir(original_dir).expect("failed to restore dir");

    result.expect("check must succeed against a clean knowledge tree");
    assert!(
        !project_root.join(".loom").join("cache").exists(),
        "check must not create a .loom/cache directory under the project root"
    );
    assert!(
        no_extra_loom_entries(project_root),
        "check must not create anything under .loom/ beyond the seeded state-directory marker"
    );
}

/// Confirms `.loom/` under `root` holds nothing beyond the [`seed_work_marker`]
/// fixture (`work/config.toml`) -- i.e. `check` itself added no cache or other
/// files there. The state directory and the context cache both now live under
/// `.loom/`, so this can no longer assert `.loom` is absent outright (see
/// `seed_work_marker`'s doc); it asserts the narrower, still load-bearing
/// property that `check` writes nothing alongside the pre-existing marker.
fn no_extra_loom_entries(root: &std::path::Path) -> bool {
    let loom_dir = root.join(".loom");
    let entries = match fs::read_dir(&loom_dir) {
        Ok(entries) => entries,
        Err(_) => return true,
    };
    if !entries
        .filter_map(|entry| entry.ok())
        .all(|entry| entry.file_name() == "work")
    {
        return false;
    }

    let work_entries = match fs::read_dir(loom_dir.join("work")) {
        Ok(entries) => entries,
        Err(_) => return true,
    };
    work_entries
        .filter_map(|entry| entry.ok())
        .all(|entry| entry.file_name() == "config.toml")
}

/// A real defect in the tree (a duplicate `## ` heading) is what `check`
/// exists to surface. Asserted against [`human_report`] - the SAME rendering
/// `check`'s `print_human` calls - rather than `catalog::build` alone, so an
/// emptied or broken `human_report` (or a `print_human` no longer wired to
/// `issue_line`) fails this test instead of going unnoticed.
#[test]
#[serial]
fn check_surfaces_a_duplicate_heading_and_still_returns_ok_without_strict() {
    let temp = TempDir::new().expect("failed to create temp dir");
    let project_root = temp.path();
    seed_work_marker(project_root);
    seed_knowledge_with_duplicate_heading(project_root);

    let original_dir = std::env::current_dir().expect("failed to get current dir");
    std::env::set_current_dir(project_root).expect("failed to change dir");
    let result = check(false, false);
    std::env::set_current_dir(original_dir).expect("failed to restore dir");

    result.expect("check without --strict must still return Ok even with issues present");

    let knowledge_root = project_root.join("doc/loom/knowledge");
    let catalog = crate::fs::knowledge::catalog::build(&knowledge_root)
        .expect("catalog::build must succeed against the seeded tree");
    let report = human_report(&knowledge_root, &catalog);
    assert!(
        report.contains("foo") && report.contains("repeated 2 times"),
        "check's human report must surface the duplicate \"foo\" heading, got: {report}"
    );
}

/// A missing knowledge root is not an error - `check` reports it and returns
/// `Ok`, since a project that has never run a knowledge stage has nothing to
/// diagnose yet.
#[test]
#[serial]
fn check_on_a_missing_knowledge_root_returns_ok() {
    let temp = TempDir::new().expect("failed to create temp dir");
    let project_root = temp.path();
    seed_work_marker(project_root);

    let original_dir = std::env::current_dir().expect("failed to get current dir");
    std::env::set_current_dir(project_root).expect("failed to change dir");
    let result = check(false, false);
    std::env::set_current_dir(original_dir).expect("failed to restore dir");

    result.expect("check must return Ok when the knowledge directory does not exist");
    assert!(
        !project_root.join("doc").exists(),
        "check must not create doc/loom/knowledge when it was absent"
    );
}

// --- `issue_line` coverage: one test per `CatalogIssue` variant, plus the
// control-character flattening property that all of them share. ---

#[test]
fn issue_line_names_the_file_and_heading_for_a_duplicate_heading() {
    let issue = CatalogIssue::DuplicateHeading {
        file: PathBuf::from("mistakes.md"),
        heading: "foo".to_string(),
        occurrences: 2,
    };
    let line = issue_line(&issue);
    assert!(line.contains("mistakes.md"), "line: {line}");
    assert!(line.contains("foo"), "line: {line}");
    assert!(line.contains("repeated 2 times"), "line: {line}");
}

#[test]
fn issue_line_names_the_file_and_blurb_for_a_generic_blurb() {
    let issue = CatalogIssue::GenericBlurb {
        file: PathBuf::from("patterns.md"),
        blurb: "Reusable patterns.".to_string(),
    };
    let line = issue_line(&issue);
    assert!(line.contains("patterns.md"), "line: {line}");
    assert!(line.contains("Reusable patterns."), "line: {line}");
}

#[test]
fn issue_line_names_the_file_and_target_for_a_broken_link() {
    let issue = CatalogIssue::BrokenLink {
        file: PathBuf::from("architecture.md"),
        target: "missing.md".to_string(),
    };
    let line = issue_line(&issue);
    assert!(line.contains("architecture.md"), "line: {line}");
    assert!(line.contains("missing.md"), "line: {line}");
}

#[test]
fn issue_line_names_the_file_and_source_path_for_a_missing_source_ref() {
    let issue = CatalogIssue::MissingSourceRef {
        file: PathBuf::from("stack.md"),
        source_path: "loom/src/gone.rs".to_string(),
    };
    let line = issue_line(&issue);
    assert!(line.contains("stack.md"), "line: {line}");
    assert!(line.contains("loom/src/gone.rs"), "line: {line}");
}

#[test]
fn issue_line_formats_evidence_changed_as_review() {
    let issue = CatalogIssue::EvidenceChanged {
        file: PathBuf::from("architecture/topic.md"),
        source_path: "loom/src/context/pack.rs".to_string(),
        verified: "0123456789abcdef".to_string(),
    };

    let line = issue_line(&issue);

    assert!(
        line.starts_with("review: architecture/topic.md:"),
        "line: {line}"
    );
    assert!(line.contains("changed since 01234567"), "line: {line}");
    assert!(line.contains("--verified HEAD"), "line: {line}");
}

#[test]
fn issue_line_formats_unverifiable_reference_as_note() {
    let issue = CatalogIssue::UnverifiableReference {
        file: PathBuf::from("patterns.md"),
        source_path: "category/slug.md".to_string(),
        kind: "example".to_string(),
    };

    let line = issue_line(&issue);

    assert!(line.starts_with("note: patterns.md:"), "line: {line}");
    assert!(line.contains("example"), "line: {line}");
    assert!(line.contains("category/slug.md"), "line: {line}");
}

#[test]
fn issue_line_names_the_file_heading_and_line_count_for_an_oversized_section() {
    let issue = CatalogIssue::OversizedSection {
        file: PathBuf::from("mistakes.md"),
        heading: "phantom merges".to_string(),
        lines: 55,
    };
    let line = issue_line(&issue);
    assert!(line.contains("mistakes.md"), "line: {line}");
    assert!(line.contains("phantom merges"), "line: {line}");
    assert!(line.contains("55 lines"), "line: {line}");
}

#[test]
fn issue_line_names_the_file_and_line_count_for_an_oversized_file() {
    let issue = CatalogIssue::OversizedFile {
        file: PathBuf::from("architecture.md"),
        lines: 300,
    };
    let line = issue_line(&issue);
    assert!(line.contains("architecture.md"), "line: {line}");
    assert!(line.contains("300 lines"), "line: {line}");
}

/// The real threshold, not a copy of the literal - this is what keeps the
/// message honest if `MAX_INDEX_BYTES` ever changes (review finding 2).
#[test]
fn issue_line_for_an_oversized_index_names_the_real_max_index_bytes_const() {
    let issue = CatalogIssue::OversizedIndex { bytes: 9_000 };
    let line = issue_line(&issue);
    assert!(line.contains(INDEX_FILENAME), "line: {line}");
    assert!(line.contains("9000"), "line: {line}");
    assert!(
        line.contains(&MAX_INDEX_BYTES.to_string()),
        "line must name the real MAX_INDEX_BYTES budget, got: {line}"
    );
}

/// A heading carrying a newline and an ESC control character must still
/// render as one line with no control characters surviving - the property
/// review finding 1 exists to fix.
#[test]
fn issue_line_flattens_control_characters_and_newlines_in_untrusted_fields() {
    let issue = CatalogIssue::DuplicateHeading {
        file: PathBuf::from("mistakes.md"),
        heading: "Broken\u{1b}[2J\nheading".to_string(),
        occurrences: 2,
    };
    let line = issue_line(&issue);
    assert!(
        !line.contains('\n'),
        "issue_line must render as a single line, got: {line:?}"
    );
    assert!(
        !line.chars().any(|ch| ch.is_control()),
        "issue_line must contain no control characters, got: {line:?}"
    );
}

// --- JSON payload coverage. ---

/// `count` must always equal `issues.len()`, and `issues` must serialize as
/// an array of that same length - the property a swapped field would break.
#[test]
fn json_payload_count_matches_the_issues_array_length() {
    let root = PathBuf::from("doc/loom/knowledge");
    let catalog = Catalog {
        revision: "deadbeef".to_string(),
        chunks: Vec::new(),
        issues: vec![
            CatalogIssue::OversizedIndex { bytes: 9_000 },
            CatalogIssue::GenericBlurb {
                file: PathBuf::from("patterns.md"),
                blurb: "Reusable patterns.".to_string(),
            },
        ],
    };

    let payload = json_payload(&root, &catalog);

    assert_eq!(payload["count"], serde_json::json!(2));
    let issues = payload["issues"]
        .as_array()
        .expect("issues must serialize as a JSON array");
    assert_eq!(issues.len(), 2);
    assert_eq!(payload["review"], serde_json::json!([]));
}

#[test]
fn json_payload_separates_review_only_diagnostics() {
    let catalog = Catalog {
        revision: String::new(),
        chunks: Vec::new(),
        issues: vec![CatalogIssue::UnverifiableReference {
            file: PathBuf::from("patterns.md"),
            source_path: "foo.rs".to_string(),
            kind: "example".to_string(),
        }],
    };

    let payload = json_payload(Path::new("doc/loom/knowledge"), &catalog);

    assert_eq!(payload["count"], 0);
    assert_eq!(payload["issues"], serde_json::json!([]));
    assert_eq!(payload["review"].as_array().unwrap().len(), 1);
}

#[test]
fn strict_ignores_review_only_issues() {
    let issues = vec![CatalogIssue::EvidenceChanged {
        file: PathBuf::from("patterns.md"),
        source_path: "src/a.rs".to_string(),
        verified: "0123456789abcdef".to_string(),
    }];

    assert_eq!(strict_issue_count(&issues), 0);
}
