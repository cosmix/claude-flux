//! `KnowledgeDir`-level regression tests for loom-bugs.txt BUG 2 (H3+
//! `replace-section` targets) and BUG 3 (duplicate-H1 scaffold on a new
//! topic). Split out of `tests_dir.rs` to keep that file under the 400-line
//! maintainability limit.

use super::*;
use tempfile::TempDir;

/// Write `content` directly to `target`'s file, creating its category
/// directory first. Used to seed shapes `append_target` itself would never
/// produce today (e.g. a pre-fix BUG-3 victim file), so remediation can be
/// tested without depending on the old buggy code path.
fn seed_topic_file(knowledge: &KnowledgeDir, target: &KnowledgeTarget, content: &str) {
    let path = knowledge.target_path(target);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
}

// --- BUG 2: replace-section on a heading nested below H2 -------------------

#[test]
fn test_replace_section_targets_nested_h3_heading() {
    let temp = TempDir::new().unwrap();
    let knowledge = KnowledgeDir::new(temp.path());
    knowledge.initialize().unwrap();

    std::fs::write(
        knowledge.file_path(KnowledgeFile::Patterns),
        "## Group heading\n\n### Individual finding\n\nstale text\n",
    )
    .unwrap();

    let outcome = knowledge
        .replace_section(
            KnowledgeFile::Patterns,
            "Individual finding",
            "corrected text",
        )
        .unwrap();

    assert_eq!(outcome, SectionOutcome::Replaced { level: 3 });
    let content = knowledge.read(KnowledgeFile::Patterns).unwrap();
    assert_eq!(
        content,
        "## Group heading\n\n### Individual finding\n\ncorrected text\n"
    );
    assert!(!content.contains("stale text"));
    // Nothing appended at EOF: exactly one occurrence of each heading.
    assert_eq!(content.matches("## Group heading").count(), 1);
    assert_eq!(content.matches("### Individual finding").count(), 1);
}

#[test]
fn test_replace_section_reports_appended_outcome_for_absent_heading() {
    let temp = TempDir::new().unwrap();
    let knowledge = KnowledgeDir::new(temp.path());
    knowledge.initialize().unwrap();

    let outcome = knowledge
        .replace_section(KnowledgeFile::Patterns, "New Heading", "content")
        .unwrap();

    assert_eq!(outcome, SectionOutcome::Appended);
}

#[test]
fn test_replace_section_reports_replaced_outcome_for_h2() {
    let temp = TempDir::new().unwrap();
    let knowledge = KnowledgeDir::new(temp.path());
    knowledge.initialize().unwrap();

    knowledge
        .append(KnowledgeFile::Patterns, "## Section A\n\nOriginal")
        .unwrap();
    let outcome = knowledge
        .replace_section(KnowledgeFile::Patterns, "Section A", "Updated")
        .unwrap();

    assert_eq!(outcome, SectionOutcome::Replaced { level: 2 });
}

// --- BUG 3: new-topic scaffold stubs a duplicate H1 -------------------------

#[test]
fn test_append_target_new_topic_with_own_title_skips_scaffold() {
    let temp = TempDir::new().unwrap();
    let knowledge = KnowledgeDir::new(temp.path());
    knowledge.initialize().unwrap();

    let target = KnowledgeTarget::parse("architecture/admin1-overlay").unwrap();
    knowledge
        .append_target(
            &target,
            "# My Real Title\n\n> My real one-line blurb.\n\nBody detail.",
        )
        .unwrap();

    let content = knowledge.read_target(&target).unwrap();
    assert_eq!(
        content.matches("# ").count(),
        1,
        "exactly one H1: {content}"
    );
    assert_eq!(
        content.matches("> ").count(),
        1,
        "exactly one blurb: {content}"
    );
    assert!(content.starts_with("# My Real Title\n"));
    assert!(content.contains("> My real one-line blurb."));
    assert!(!content.contains("Topic notes for the"));

    let index = knowledge.read_index().unwrap();
    assert!(index.contains("[admin1-overlay](architecture/admin1-overlay.md)"));
    assert!(index.contains("My real one-line blurb."));
}

#[test]
fn test_append_target_self_heals_existing_generic_stub() {
    let temp = TempDir::new().unwrap();
    let knowledge = KnowledgeDir::new(temp.path());
    knowledge.initialize().unwrap();

    let target = KnowledgeTarget::parse("mistakes/admin1-subsystem-traps").unwrap();
    // Seed the REAL BUG-3 victim shape: the generic stub, then the author's
    // own real header, appended together by one (pre-fix) buggy call.
    seed_topic_file(
        &knowledge,
        &target,
        "# Admin1 Subsystem Traps\n\n\
         > Topic notes for the mistakes knowledge area.\n\n\
         # Real Title\n\n\
         > Real blurb.\n\n\
         ## First finding\n\n\
         body one\n",
    );

    // Remediate with corrected header + a genuinely new section.
    knowledge
        .append_target(
            &target,
            "# Real Title\n\n> Real blurb.\n\n## Second finding\n\nbody two",
        )
        .unwrap();

    assert_healed_and_ordered(&knowledge.read_target(&target).unwrap());
}

/// Assertions for the healed-victim-file shape: generic stub gone, exactly
/// one real H1/blurb (no duplicate left behind in the preserved body
/// either), and the pre-existing section still ahead of the newly appended
/// one (`update` is append-only).
fn assert_healed_and_ordered(content: &str) {
    assert!(
        !content.contains("Topic notes for the mistakes knowledge area."),
        "generic stub blurb must be gone: {content}"
    );
    // Line-prefix checks, not substring counts: "## First finding" itself
    // contains the substring "# " (and "> " could in principle appear inside
    // a body line), so only a real H1/blockquote LINE should count.
    assert_eq!(
        content.lines().filter(|l| l.starts_with("# ")).count(),
        1,
        "exactly one H1: {content}"
    );
    assert_eq!(
        content.lines().filter(|l| l.starts_with("> ")).count(),
        1,
        "exactly one blurb: {content}"
    );
    assert!(content.starts_with("# Real Title\n"));
    assert!(
        content.contains("## First finding") && content.contains("body one"),
        "pre-existing section must be preserved: {content}"
    );
    assert!(
        content.contains("## Second finding") && content.contains("body two"),
        "newly appended section must be present: {content}"
    );
    assert!(
        content.find("## First finding").unwrap() < content.find("## Second finding").unwrap(),
        "the pre-existing section must appear BEFORE the newly appended one: {content}"
    );
}
