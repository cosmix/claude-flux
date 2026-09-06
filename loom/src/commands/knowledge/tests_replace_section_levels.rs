//! CLI-level regression tests for loom-bugs.txt BUG 2 (`replace-section`
//! could not target a heading nested below H2) and BUG 3 (`update` on a new
//! topic stubbed a duplicate H1). Split out of `tests.rs` to keep that file
//! under the 400-line maintainability limit.

use super::tests::setup_test_env;
use super::*;
use serial_test::serial;
use std::fs;

#[test]
#[serial]
fn test_replace_section_corrects_nested_h3_in_place() {
    let (_temp_dir, test_dir) = setup_test_env();
    let original_dir = std::env::current_dir().expect("Failed to get current dir");
    std::env::set_current_dir(&test_dir).expect("Failed to change dir");

    KnowledgeDir::new(".")
        .initialize()
        .expect("Failed to initialize knowledge");

    update(
        "patterns".to_string(),
        Some("## Group heading\n\n### Individual finding\n\nStale claim".to_string()),
    )
    .expect("seed append failed");

    let result = replace_section(
        "patterns".to_string(),
        "### Individual finding".to_string(),
        Some("### Individual finding\n\nCorrected claim".to_string()),
    );
    assert!(result.is_ok(), "replace_section() failed: {result:?}");

    let content = fs::read_to_string(test_dir.join("doc/loom/knowledge/patterns.md")).unwrap();
    assert!(content.contains("Corrected claim"));
    assert!(
        !content.contains("Stale claim"),
        "stale text must be gone, not left behind with a disconnected duplicate appended"
    );
    assert_eq!(
        content.matches("### Individual finding").count(),
        1,
        "the H3 heading must not be duplicated: {content}"
    );
    assert_eq!(
        content.matches("## Group heading").count(),
        1,
        "the parent H2 group heading must be untouched: {content}"
    );

    std::env::set_current_dir(original_dir).expect("Failed to restore dir");
}

#[test]
#[serial]
fn test_update_new_topic_with_own_title_shows_real_summary_in_index() {
    let (_temp_dir, test_dir) = setup_test_env();
    let original_dir = std::env::current_dir().expect("Failed to get current dir");
    std::env::set_current_dir(&test_dir).expect("Failed to change dir");

    KnowledgeDir::new(".")
        .initialize()
        .expect("Failed to initialize knowledge");

    let result = update(
        "architecture/admin1-overlay".to_string(),
        Some("# Admin1 Overlay And Card\n\n> How the admin1 overlay renders its card.".to_string()),
    );
    assert!(result.is_ok(), "update() failed: {result:?}");

    let topic_path = test_dir.join("doc/loom/knowledge/architecture/admin1-overlay.md");
    let content = fs::read_to_string(&topic_path).unwrap();
    assert_eq!(
        content.matches("# ").count(),
        1,
        "exactly one H1: {content}"
    );
    assert!(!content.contains("Topic notes for the"));

    let index = fs::read_to_string(test_dir.join("doc/loom/knowledge/INDEX.md")).unwrap();
    assert!(index.contains("[admin1-overlay](architecture/admin1-overlay.md)"));
    assert!(index.contains("How the admin1 overlay renders its card."));
    assert!(!index.contains("Topic notes for the architecture knowledge area."));

    std::env::set_current_dir(original_dir).expect("Failed to restore dir");
}
