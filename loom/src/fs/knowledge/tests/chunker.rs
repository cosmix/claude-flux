use crate::context::schema::LifecycleState;
use crate::fs::knowledge::chunker::*;
use sha2::{Digest, Sha256};
use std::path::Path;
use tempfile::TempDir;

#[test]
fn rule_1_decodes_lossy_utf8() {
    let temp = TempDir::new().unwrap();
    let chunks = chunk_file(&temp.path().join("notes.md"), b"## Title\nbad\xff\n").unwrap();
    assert_eq!(chunks[0].body, "## Title\nbad�\n");
}

#[test]
fn rule_2_parses_and_strips_frontmatter_or_falls_back() {
    let valid =
        b"---\nid: fixed\naliases: [one]\nstate: draft\nsources: [src/a.rs]\n---\n## Title\nBody\n";
    let valid_chunks = chunk_file(Path::new("notes.md"), valid).unwrap();
    assert_eq!(valid_chunks[0].id, "fixed");
    assert_eq!(valid_chunks[0].body, "## Title\nBody\n");
    assert_eq!(valid_chunks[0].state, LifecycleState::Draft);
    let invalid = b"---\nunknown: value\n---\n## Title\n";
    let invalid_chunks = chunk_file(Path::new("notes.md"), invalid).unwrap();
    assert_eq!(invalid_chunks[0].id, "notes.md#title#0");
    assert!(!invalid_chunks[0].body.contains("unknown:"));
}

#[test]
fn rule_3_ignores_h2_lines_in_backtick_and_tilde_fences() {
    for fence in ["```", "~~~"] {
        let input = format!("## One\n{fence}\n## Not A Split\n{fence}\n## Two\n");
        let chunks = chunk_file(Path::new("notes.md"), input.as_bytes()).unwrap();
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].body.contains("## Not A Split"));
        assert_eq!(chunks[1].heading, "Two");
    }
}

#[test]
fn rule_4_emits_only_nonempty_preamble() {
    let with_preamble = chunk_file(Path::new("notes.md"), b"Intro\n## First\n").unwrap();
    assert_eq!(with_preamble.len(), 2);
    assert_eq!(with_preamble[0].heading, "");
    let without_preamble = chunk_file(Path::new("notes.md"), b" \n\t\n## First\n").unwrap();
    assert_eq!(without_preamble.len(), 1);
}

#[test]
fn rule_5_keeps_heading_and_trims_trailing_blank_lines() {
    let chunks = chunk_file(Path::new("notes.md"), b"## First\nBody\n \n\n## Second\n").unwrap();
    assert_eq!(chunks[0].body, "## First\nBody\n");
    assert_eq!(chunks[1].body, "## Second\n");
}

#[test]
fn rule_6_normalizes_headings() {
    let chunks = chunk_file(
        Path::new("notes.md"),
        b"## Audit Rules -- the Two Checks Disagree\n",
    )
    .unwrap();
    assert_eq!(chunks[0].anchor, "audit-rules-the-two-checks-disagree");
}

#[test]
fn rule_7_counts_identical_normalized_headings() {
    let chunks = chunk_file(Path::new("notes.md"), b"## Locking!\n## locking\n").unwrap();
    assert_eq!(chunks[0].id, "notes.md#locking#0");
    assert_eq!(chunks[1].id, "notes.md#locking#1");
    assert_ne!(chunks[0].id, chunks[1].id);
}

#[test]
fn rule_8_derives_relative_ids_and_applies_explicit_id_once() {
    let input = b"---\nid: supplied\n---\n## First\n## Second\n";
    let chunks = chunk_file(Path::new("architecture/hooks.md"), input).unwrap();
    assert_eq!(chunks[0].id, "supplied");
    assert_eq!(chunks[1].id, "architecture/hooks.md#second#0");
}

#[test]
fn rule_9_preserves_written_heading_and_anchor() {
    let chunks = chunk_file(Path::new("notes.md"), b"##  Written Heading  \n").unwrap();
    assert_eq!(chunks[0].heading, "Written Heading");
    assert_eq!(chunks[0].anchor, "written-heading");
}

#[test]
fn rule_10_hashes_the_chunk_body() {
    let chunks = chunk_file(Path::new("notes.md"), b"## Title\nBody\n").unwrap();
    let expected = format!(
        "sha256:{}",
        hex::encode(Sha256::digest(b"## Title\nBody\n"))
    );
    assert_eq!(chunks[0].content_hash, expected);
}

#[test]
fn rule_11_estimates_tokens_from_body_bytes() {
    let chunks = chunk_file(Path::new("notes.md"), b"## T\n12345678\n").unwrap();
    assert_eq!(chunks[0].estimated_tokens, chunks[0].body.len() / 4);
}

#[test]
fn rule_12_extracts_deduplicated_backticked_source_paths() {
    let chunks = chunk_file(
        Path::new("notes.md"),
        b"## T\n`src/a.rs` `src/a.rs` outside/b.ts `tools/run.sh`\n",
    )
    .unwrap();
    assert_eq!(chunks[0].source_paths, vec!["src/a.rs", "tools/run.sh"]);
}

#[test]
fn rule_13_extracts_deduplicated_backticked_symbols() {
    let chunks = chunk_file(
        Path::new("notes.md"),
        b"## T\n`crate::Thing` `crate::Thing` `not a symbol` `9bad`\n",
    )
    .unwrap();
    assert_eq!(chunks[0].symbols, vec!["crate::Thing"]);
}

#[test]
fn rule_14_collects_markdown_links() {
    let chunks = chunk_file(
        Path::new("notes.md"),
        b"## T\n[One](other.md) [Two](nested/two.md) [No](other.txt)\n",
    )
    .unwrap();
    assert_eq!(
        chunks[0].links,
        vec![
            ("One".to_string(), "other.md".to_string()),
            ("Two".to_string(), "nested/two.md".to_string())
        ]
    );
}

#[test]
fn rule_15_derives_category_from_parent_directory() {
    let tier_one = chunk_file(Path::new("architecture.md"), b"## T\n").unwrap();
    let topic = chunk_file(Path::new("architecture/topic.md"), b"## T\n").unwrap();
    assert_eq!(tier_one[0].category, None);
    assert_eq!(topic[0].category.as_deref(), Some("architecture"));
}

/// Regression: `Frontmatter` used to `deny_unknown_fields`, so a single
/// unrecognized key (like `title`, which this subsystem does not read) failed
/// the whole frontmatter parse and silently discarded every other field.
#[test]
fn unrecognized_frontmatter_keys_are_ignored_not_fatal() {
    let input =
        b"---\ntitle: Verification Harness\nstate: deprecated\naliases: [harness]\n---\n## Notes\n";
    let chunks = chunk_file(Path::new("notes.md"), input).unwrap();
    assert_eq!(chunks[0].state, LifecycleState::Deprecated);
    assert_eq!(chunks[0].aliases, vec!["harness"]);
}

/// Regression: `SOURCE_PATH_REGEX` had no trailing boundary, so it matched a
/// known extension as a prefix of a longer, unrelated one — `foo.rsx` yielded
/// a phantom `foo.rs` source reference. The regex crate has no lookahead, so
/// the fix rejects a match followed by another identifier character instead.
/// `notes.markdown` is a control, not a second regression case: "markdown"
/// has no adjacent `m`,`d` pair, so it never matched `\.md`, before or after
/// this fix — it is here only to confirm the fix does not start matching it.
#[test]
fn source_path_regex_does_not_match_a_known_extension_as_a_prefix() {
    let chunks = chunk_file(
        Path::new("notes.md"),
        b"## T\n`notes.markdown` `foo.rsx` `src/context/pack.rs`\n",
    )
    .unwrap();
    assert_eq!(chunks[0].source_paths, vec!["src/context/pack.rs"]);
}

#[test]
fn rule_16_applies_aliases_sources_and_state_as_specified() {
    let input = b"---\naliases: [first-alias]\nstate: deprecated\nsources: [src/a.rs]\n---\n## One\n`src/a.rs` `src/b.rs`\n## Two\n`src/c.rs`\n";
    let chunks = chunk_file(Path::new("notes.md"), input).unwrap();
    assert_eq!(chunks[0].aliases, vec!["first-alias"]);
    assert!(chunks[1].aliases.is_empty());
    assert_eq!(chunks[0].state, LifecycleState::Deprecated);
    assert_eq!(chunks[1].state, LifecycleState::Deprecated);
    assert_eq!(chunks[0].source_paths, vec!["src/a.rs", "src/b.rs"]);
    assert_eq!(chunks[1].source_paths, vec!["src/c.rs"]);
}

#[test]
fn a_live_reference_is_classified_as_live() {
    use crate::fs::knowledge::chunker::references::{classify_reference, EvidenceKind};

    assert_eq!(
        classify_reference(
            "loom/src/context/pack.rs",
            "See `loom/src/context/pack.rs` for packing.",
        ),
        EvidenceKind::Live
    );
}

#[test]
fn example_references_are_classified_as_examples() {
    use crate::fs::knowledge::chunker::references::{classify_reference, EvidenceKind};

    assert_eq!(
        classify_reference("category/slug.md", "Use `category/slug.md` as an example."),
        EvidenceKind::Example
    );
    assert_eq!(
        classify_reference("doc/plans/PLAN-foo.md", "Write `doc/plans/PLAN-foo.md`."),
        EvidenceKind::Example
    );
}

#[test]
fn a_runtime_reference_is_classified_as_runtime() {
    use crate::fs::knowledge::chunker::references::{classify_reference, EvidenceKind};

    assert_eq!(
        classify_reference(
            ".work/config.toml",
            "Runtime state is in `.work/config.toml`."
        ),
        EvidenceKind::Runtime
    );
}

#[test]
fn an_external_reference_is_classified_as_external() {
    use crate::fs::knowledge::chunker::references::{classify_reference, EvidenceKind};

    assert_eq!(
        classify_reference(
            "codex-rs/linux-sandbox/src/bwrap.rs",
            "OpenAI's codex-rs uses `codex-rs/linux-sandbox/src/bwrap.rs`."
        ),
        EvidenceKind::External
    );
}

#[test]
fn a_historical_reference_is_classified_as_historical() {
    use crate::fs::knowledge::chunker::references::{classify_reference, EvidenceKind};

    assert_eq!(
        classify_reference("gc.rs", "There is no `gc.rs` in this version."),
        EvidenceKind::Historical
    );
}

#[test]
fn there_is_no_followed_by_other_words_stays_live() {
    use crate::fs::knowledge::chunker::references::{classify_reference, EvidenceKind};

    assert_eq!(
        classify_reference(
            "loom/src/x.rs",
            "There is no reason to change `loom/src/x.rs`.",
        ),
        EvidenceKind::Live
    );
}

#[test]
fn only_live_references_become_source_paths() {
    let chunks = chunk_file(
        Path::new("notes.md"),
        b"## T\nUse `category/slug.md` as an example. Runtime is `.work/config.toml`. The live code is `loom/src/context/pack.rs`.\n",
    )
    .unwrap();

    assert_eq!(chunks[0].source_paths, vec!["loom/src/context/pack.rs"]);
}
