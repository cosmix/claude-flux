//! Source-item rendering tests: grouping, `<path>#<kind>:<scope>` id parsing,
//! and the containment case specific to a source bullet's name/kind.
//!
//! Split out of `brief_tests.rs` (which stayed near its own line budget) into
//! a sibling wired the same way `brief.rs` wires `brief_tests.rs` itself:
//! `#[path = "brief_tests_source.rs"] mod source_tests;`.

use super::super::format_knowledge_brief;
use super::{pack, rank_source_item, source_item};

#[test]
fn a_source_item_renders_its_name_kind_span_and_reasons() {
    let pack = pack(vec![rank_source_item(Some(41), Some(58))], 0);
    let rendered = format_knowledge_brief(&pack, Some("stage-1"), "q");

    assert!(
        rendered.contains(
            "- `loom/src/context/rank.rs` — `rank` function :41-58 (lexical, exact-path)"
        ),
        "a source item reaches the brief with its name, kind, span and reasons: {rendered}"
    );
    // Source items get no fence and no per-item Reason line.
    assert!(!rendered.contains("```text"), "{rendered}");
    assert!(!rendered.contains("Reason:"), "{rendered}");
}

#[test]
fn a_span_with_no_end_renders_its_start_alone() {
    let open_ended = pack(vec![rank_source_item(Some(41), None)], 0);
    let rendered = format_knowledge_brief(&open_ended, Some("stage-1"), "q");
    assert!(
        rendered.contains("— `rank` function :41 (lexical, exact-path)"),
        "{rendered}"
    );
}

#[test]
fn a_source_id_that_does_not_parse_falls_back_to_rendering_the_whole_id() {
    let malformed = source_item("no-hash-in-this-id", "some/file.rs", Some(1), Some(2));
    let rendered = format_knowledge_brief(&pack(vec![malformed], 0), Some("stage-1"), "q");

    assert!(
        rendered.contains("- `some/file.rs` — `no-hash-in-this-id` :1-2 (lexical, exact-path)"),
        "{rendered}"
    );
}

#[test]
fn three_source_items_from_one_file_collapse_onto_one_bullet() {
    let items = vec![
        source_item(
            "loom/src/context/rank.rs#function:rank",
            "loom/src/context/rank.rs",
            Some(10),
            Some(20),
        ),
        source_item(
            "loom/src/context/rank.rs#function:score",
            "loom/src/context/rank.rs",
            Some(30),
            Some(40),
        ),
        source_item(
            "loom/src/context/rank.rs#type:RankedCandidate",
            "loom/src/context/rank.rs",
            Some(1),
            Some(5),
        ),
    ];
    let rendered = format_knowledge_brief(&pack(items, 0), Some("stage-1"), "q");

    let bullets: Vec<&str> = rendered
        .lines()
        .filter(|line| line.starts_with("- `loom/src/context/rank.rs`"))
        .collect();
    assert_eq!(bullets.len(), 1, "{rendered}");
    let bullet = bullets[0];
    assert!(bullet.contains("`rank` function :10-20"), "{bullet}");
    assert!(bullet.contains("`score` function :30-40"), "{bullet}");
    assert!(bullet.contains("`RankedCandidate` type :1-5"), "{bullet}");
}

#[test]
fn adjacency_only_grouping_does_not_merge_across_a_different_path() {
    let items = vec![
        source_item("a.rs#function:one", "a.rs", Some(1), Some(2)),
        source_item("b.rs#function:two", "b.rs", Some(3), Some(4)),
        source_item("a.rs#function:three", "a.rs", Some(5), Some(6)),
    ];
    let rendered = format_knowledge_brief(&pack(items, 0), Some("stage-1"), "q");

    let bullets: Vec<&str> = rendered
        .lines()
        .filter(|line| line.starts_with("- `"))
        .collect();
    assert_eq!(
        bullets.len(),
        3,
        "pack order, not path, decides adjacency: {rendered}"
    );
}

#[test]
fn a_source_name_carrying_a_backtick_cannot_close_its_span() {
    let hostile = source_item("f.rs#function:foo`bar", "f.rs", Some(1), Some(2));
    let rendered = format_knowledge_brief(&pack(vec![hostile], 0), Some("stage-1"), "q");

    // The raw embedded backtick, which would otherwise close the wrapping
    // span early, must not survive — only the neutralised form may appear,
    // inside its own legitimate pair of delimiters.
    assert!(!rendered.contains("foo`bar"), "{rendered}");
    assert!(rendered.contains("`fooˋbar`"), "{rendered}");
}
