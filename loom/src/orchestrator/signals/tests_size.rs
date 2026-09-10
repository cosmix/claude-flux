//! Size ceilings on the agent-facing text surfaces loom controls.
//!
//! `CLAUDE.md.template`, `generate_stable_prefix()`, the standard signal's
//! stable+semi-stable boilerplate floor, and `extract_plan_overview`'s
//! truncation cap were all cut down together (see the sibling doctrine test
//! files for what was cut and why). None of that trimming is enforced by the
//! type system — a future doctrine addition can silently regrow any of these
//! surfaces one paragraph at a time until the residency cost is back where it
//! started. Each test here pins one surface to a byte ceiling so that
//! regrowth fails loudly, in this file, instead of silently compounding
//! across every spawn of every stage in every plan.

use std::fs;

use tempfile::TempDir;

use super::cache::generate_stable_prefix;
use super::generate::{
    extract_plan_overview, extract_plan_overview_from, generate_signal_with_metrics,
};

/// `CLAUDE.md.template` is pasted into every subagent's context on every
/// spawn, across every stage, in every plan.
///
/// Raised alongside the subagent context-ceiling doctrine (BLOCK-D, Rule 3
/// and hard stop 4's ceiling-raise wording): the prior ceiling (26,624) left
/// only ~26 bytes of headroom, too little for the addition. Raised again
/// alongside the memory-events doctrine (receipts, `--evidence`): the prior
/// ceiling (28,672) left only ~18 bytes of headroom by the time that change
/// landed. Actual size is now 28,813 bytes, leaving ~880 bytes of buffer.
/// Trim future doctrine additions rather than spending down that buffer.
const CLAUDE_MD_TEMPLATE_MAX_BYTES: usize = 29_696;

/// The KV-cache-stable prefix pasted into the first message of every fresh
/// session spawned for a standard stage.
///
/// Raised alongside BLOCK-D (see `CLAUDE_MD_TEMPLATE_MAX_BYTES`). The first
/// raise (5,120 -> 6,144) left only ~138 bytes of buffer over the 6,006-byte
/// actual - thin enough that the next one-line doctrine edit would trip it -
/// so it was raised again to 7,168. Actual size is now 6,046 bytes, leaving
/// ~1.1KB of buffer.
const STABLE_PREFIX_MAX_BYTES: usize = 7_168;

/// The stable-prefix + semi-stable-section floor every standard-stage signal
/// pays, independent of the stage's own assignment text.
///
/// This is NOT the whole floor: `format_dynamic_section` (`format/sections.rs`)
/// emits its own stage-invariant prose before `## Assignment` — the "WHERE
/// COMMANDS EXECUTE" box (`sections.rs:348-365`) and the Worktree Isolation
/// ALLOWED/FORBIDDEN lists (`sections.rs:374-403`). That text lands in
/// `metrics.dynamic_bytes` mixed with genuinely per-stage content (the
/// assignment, dependencies, acceptance criteria, files to modify), so it
/// cannot be pinned to a fixed ceiling without either splitting it out of
/// `format_dynamic_section` first or asserting on its literal text (fragile).
/// A future stage tightening this ratchet should do that split and add a
/// fifth ceiling over the invariant part; until then it can regrow unchecked.
///
/// Unchanged by BLOCK-D (see `STABLE_PREFIX_MAX_BYTES`): stable_prefix_bytes
/// grows by ~889 bytes for a standard stage, but actual floor after that
/// addition is 7,624 bytes (stable 6,034, semi-stable 1,590) - comfortably
/// under this ceiling already, so it stays at its original value.
const STANDARD_SIGNAL_BOILERPLATE_FLOOR_MAX_BYTES: usize = 10_240;

/// Mirrors `generate::MAX_PLAN_OVERVIEW_BYTES` (private to that module); an
/// unbounded plan overview is re-embedded in every signal generated for the
/// stage, so it is paid on every fresh session spawned for it.
const PLAN_OVERVIEW_MAX_BYTES: usize = 4_096;

const CLAUDE_MD_TEMPLATE: &str = include_str!("../../../../CLAUDE.md.template");

#[test]
fn claude_md_template_stays_under_its_size_ceiling() {
    let actual = CLAUDE_MD_TEMPLATE.len();
    assert!(
        actual <= CLAUDE_MD_TEMPLATE_MAX_BYTES,
        "CLAUDE.md.template regrew to {actual} bytes (ceiling {CLAUDE_MD_TEMPLATE_MAX_BYTES}). \
         This file is pasted into every subagent's context on every spawn across every stage in \
         every plan, so a doctrine paragraph re-added here is a cost paid forever, not once. \
         Trim the regrowth instead of raising the ceiling."
    );
}

#[test]
fn generate_stable_prefix_stays_under_its_size_ceiling() {
    let prefix = generate_stable_prefix();
    let actual = prefix.len();
    assert!(
        actual <= STABLE_PREFIX_MAX_BYTES,
        "generate_stable_prefix() regrew to {actual} bytes (ceiling {STABLE_PREFIX_MAX_BYTES}). \
         This text is the KV-cache-stable prefix pasted into the first message of every fresh \
         session spawned for a standard stage, so growth here is paid on every spawn, not once. \
         Trim the regrowth instead of raising the ceiling."
    );
}

/// Measures the stable-prefix + semi-stable-section floor a standard stage
/// pays, i.e. `metrics.stable_prefix_bytes + metrics.semi_stable_bytes`.
///
/// This does NOT cover everything a stage pays before its own assignment
/// text: `format_dynamic_section` emits stage-invariant prose of its own
/// ahead of `## Assignment` (see the ceiling constant's doc comment for
/// where), and that text is counted in `metrics.dynamic_bytes` instead,
/// which this test does not touch. `stage.description` only ever reaches the
/// dynamic and recitation sections, so it plays no part in what this test
/// measures — a real stage description is used rather than clearing it.
#[test]
fn standard_signal_stable_plus_semi_stable_boilerplate_floor_stays_under_its_size_ceiling() {
    let temp_dir = TempDir::new().unwrap();
    let work_dir = temp_dir.path().join(".loom").join("work");
    fs::create_dir_all(&work_dir).unwrap();

    let session = super::tests::create_test_session();
    let worktree = super::tests::create_test_worktree();
    let stage = super::tests::create_test_stage();

    let (_signal_path, metrics) =
        generate_signal_with_metrics(&session, &stage, &worktree, &[], None, None, &work_dir)
            .unwrap();

    let floor = metrics.stable_prefix_bytes + metrics.semi_stable_bytes;
    assert!(
        floor <= STANDARD_SIGNAL_BOILERPLATE_FLOOR_MAX_BYTES,
        "the standard signal's stable+semi-stable boilerplate floor regrew to {floor} bytes \
         (stable {}, semi-stable {}, ceiling {STANDARD_SIGNAL_BOILERPLATE_FLOOR_MAX_BYTES}). \
         This is the stable+semi-stable boilerplate every stage pays, so growing it here taxes \
         every stage in every plan. Trim the regrowth instead of raising the ceiling. (Note: this \
         is not the FULL floor before the assignment text — see the ceiling constant's doc comment \
         for the dynamic-section prose this test does not cover.)",
        metrics.stable_prefix_bytes,
        metrics.semi_stable_bytes,
    );
}

#[test]
fn extract_plan_overview_never_exceeds_max_bytes() {
    let line = "x".repeat(200);
    let mut body = String::new();
    for _ in 0..300 {
        body.push_str(&line);
        body.push('\n');
    }
    // ~60,300 bytes of Overview body, far larger than the 4,096-byte cap.
    let plan = format!("# Plan\n\n## Overview\n\n{body}\n## Stages\n\nsomething\n");

    let result = extract_plan_overview(&plan).expect("overview section must be present");
    assert!(
        result.len() <= PLAN_OVERVIEW_MAX_BYTES,
        "extract_plan_overview returned {} bytes for a {}-byte Overview section \
         (ceiling {PLAN_OVERVIEW_MAX_BYTES}). This text is embedded in every signal generated for \
         the stage, so an unbounded overview is paid on every fresh session spawned for it.",
        result.len(),
        body.len(),
    );
}

/// "日" and friends are 3-byte UTF-8 sequences; packed densely with only
/// occasional newlines, the byte offset `truncate_overview` computes from
/// `MAX_PLAN_OVERVIEW_BYTES` is very likely to land mid-character. The
/// function must walk back to a char boundary before slicing — a naive
/// `&text[..cut]` panics instead of failing an assertion.
#[test]
fn extract_plan_overview_truncates_multibyte_content_without_panicking() {
    let chunk = "日本語のテキストです。".repeat(40);
    let mut body = String::new();
    for _ in 0..50 {
        body.push_str(&chunk);
        body.push('\n');
    }
    let plan = format!("# Plan\n\n## Overview\n\n{body}\n## Stages\n\nsomething\n");

    let result = extract_plan_overview(&plan).expect("overview section must be present");
    assert!(
        result.len() <= PLAN_OVERVIEW_MAX_BYTES,
        "extract_plan_overview returned {} bytes for a multi-byte Overview section (ceiling \
         {PLAN_OVERVIEW_MAX_BYTES})",
        result.len(),
    );
}

/// Regression test: a plan whose `## Overview` section is one long unwrapped
/// paragraph (no interior newlines) used to degrade to "heading lines only".
/// `truncate_overview` used to take a line-boundary cut unconditionally,
/// however early it landed; for this input shape the last newline inside the
/// truncation window sits right after the `## Overview` heading, so the old
/// code returned two heading lines plus the truncation suffix (~110 bytes)
/// and silently discarded the ~6,000-byte paragraph. A bound-only assertion
/// (`<= max_bytes`) cannot catch this — two heading lines easily satisfy any
/// upper bound — so this test pins a CONTENT floor too. Do not "simplify"
/// this back to a bound-only check; that reintroduces the bug undetected.
#[test]
fn extract_plan_overview_keeps_most_of_an_unwrapped_paragraph() {
    let long_line = "x".repeat(6_000);
    let plan = format!("# Plan\n\n## Overview\n\n{long_line}\n## Stages\n\nsomething\n");

    let result = extract_plan_overview(&plan).expect("overview section must be present");

    assert!(
        result.len() <= PLAN_OVERVIEW_MAX_BYTES,
        "extract_plan_overview returned {} bytes for a single-line Overview (ceiling \
         {PLAN_OVERVIEW_MAX_BYTES})",
        result.len(),
    );
    // The old code returned ~110 bytes here (two heading lines + suffix). The
    // fix keeps most of the budget instead of cutting right after the heading.
    assert!(
        result.len() >= PLAN_OVERVIEW_MAX_BYTES / 2,
        "extract_plan_overview returned only {} bytes for a single-line Overview far longer than \
         the cap (ceiling {PLAN_OVERVIEW_MAX_BYTES}); an early line-boundary cut is throwing away \
         the paragraph instead of falling back to the char-boundary cut",
        result.len(),
    );
}

/// The production path (`read_plan_overview`) calls `extract_plan_overview_from`
/// with the plan's real file path as `plan_label`, which can be far longer than
/// the short `"the plan file"` label the `#[cfg(test)]` wrapper above uses. The
/// ≤4,096-byte cap must hold there too, even when the label alone would blow it.
#[test]
fn extract_plan_overview_from_bounds_result_even_with_a_pathological_label() {
    let line = "y".repeat(200);
    let mut body = String::new();
    for _ in 0..300 {
        body.push_str(&line);
        body.push('\n');
    }
    let plan = format!("# Plan\n\n## Overview\n\n{body}\n## Stages\n\nsomething\n");

    let pathological_label = "z".repeat(5_000);
    let result = extract_plan_overview_from(&plan, &pathological_label)
        .expect("overview section must be present");
    assert!(
        result.len() <= PLAN_OVERVIEW_MAX_BYTES,
        "extract_plan_overview_from returned {} bytes for a {}-byte plan_label (ceiling \
         {PLAN_OVERVIEW_MAX_BYTES}); the production path passes the plan's real file path, which \
         can be arbitrarily long, so the cap must hold even when the label alone would blow it",
        result.len(),
        pathological_label.len(),
    );
}
