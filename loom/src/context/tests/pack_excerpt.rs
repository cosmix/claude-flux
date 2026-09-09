//! Windowing tests for [`crate::context::pack::excerpt::bounded_excerpt`].
//!
//! The excerpt exists to show the reader the line that matched. Every test here
//! builds a body over the token cap — so the bounded path is really taken — and
//! then asks the one question that matters: is the match inside what came back,
//! and is what came back honest about the text it dropped?

use crate::context::pack::excerpt::bounded_excerpt;
use crate::context::schema::{estimate_tokens, EXCERPT_MAX_TOKENS, EXCERPT_TRUNCATION_MARKER};

const EARLIER_LINES_MARKER: &str = "[… earlier lines omitted]";

/// A 20-byte line carrying no term any test searches for.
const FILLER: &str = "ordinary filler xyz\n";

fn excerpt(body: &str, terms: &[&str]) -> (String, bool) {
    let terms: Vec<String> = terms.iter().map(|term| (*term).to_string()).collect();
    let (text, omitted) = bounded_excerpt(body, &terms);
    assert!(
        estimate_tokens(body) > EXCERPT_MAX_TOKENS,
        "the fixture must exceed the cap or the bounded path is never taken"
    );
    (text, omitted)
}

/// The quoted body text, with both markers stripped off.
fn quoted(excerpt: &str) -> &str {
    let body = excerpt
        .strip_prefix(EARLIER_LINES_MARKER)
        .map(|rest| rest.strip_prefix('\n').unwrap_or(rest))
        .unwrap_or(excerpt);
    match body.strip_suffix(EXCERPT_TRUNCATION_MARKER) {
        Some(trimmed) => trimmed.strip_suffix('\n').unwrap_or(trimmed),
        None => body,
    }
}

#[test]
fn a_body_under_the_token_cap_is_returned_verbatim() {
    let body = "## Heading\n\nA short section body.\n";
    assert_eq!(
        bounded_excerpt(body, &["heading".to_string()]),
        (body.to_string(), false)
    );
}

/// The matching line spans bytes 1580..1620, straddling the 1600-byte budget.
/// Cutting at the last line end inside the budget stops exactly where the match
/// begins, which is what the old windowing did.
#[test]
fn a_match_straddling_the_byte_limit_is_inside_the_excerpt() {
    let matching = "needle straddles the excerpt byte limit\n";
    let body = format!("{}{matching}{}", FILLER.repeat(79), FILLER.repeat(100));
    assert_eq!(body.find(matching), Some(1_580));

    let (text, omitted) = excerpt(&body, &["needle"]);
    assert!(text.contains(matching.trim_end()), "excerpt: {text}");
    assert!(omitted);
}

/// A single unwrapped paragraph, a minified line, a long URL: the match sits on
/// a line longer than the whole budget. The window is that line, over budget and
/// whole, rather than the short lead-in line that fits.
#[test]
fn a_match_on_a_line_longer_than_the_whole_budget_is_still_returned() {
    let long_line = format!("needle {}\n", "padding word ".repeat(160));
    assert!(long_line.len() > 1_600);
    let body = format!("{}{long_line}{}", FILLER.repeat(100), FILLER.repeat(100));

    let (text, _) = excerpt(&body, &["needle"]);
    assert!(text.contains(long_line.trim_end()), "excerpt: {text}");
}

#[test]
fn the_window_brackets_the_match_on_both_sides() {
    let before = "alpha sentinel ahead of it\n";
    let after = "omega sentinel behind it\n";
    let body = format!(
        "{}{before}{}needle sits in the middle\n{}{after}{}",
        FILLER.repeat(150),
        FILLER.repeat(3),
        FILLER.repeat(3),
        FILLER.repeat(150)
    );

    let (text, _) = excerpt(&body, &["needle"]);
    assert!(
        text.contains("needle sits in the middle"),
        "excerpt: {text}"
    );
    assert!(text.contains(before.trim_end()), "no lead-in: {text}");
    assert!(text.contains(after.trim_end()), "no follow-on: {text}");
    assert!(text.starts_with(EARLIER_LINES_MARKER));
}

/// A match two lines in has nothing behind it to quote, so the backward half of
/// the budget is not simply wasted: it goes forward instead.
#[test]
fn a_match_near_the_start_fills_the_window_forward() {
    let sentinel = "faraway sentinel line\n";
    let body = format!(
        "{}needle sits near the start\n{}{sentinel}{}",
        FILLER.repeat(2),
        FILLER.repeat(68),
        FILLER.repeat(100)
    );

    let (text, _) = excerpt(&body, &["needle"]);
    assert!(!text.starts_with(EARLIER_LINES_MARKER), "excerpt: {text}");
    assert!(text.starts_with(FILLER), "the window starts at byte 0");
    assert!(
        text.contains(sentinel.trim_end()),
        "window cut short: {text}"
    );
}

#[test]
fn an_excerpt_never_splits_a_multibyte_character() {
    let body = "→".repeat(700);
    let (text, _) = excerpt(&body, &["absent"]);
    assert!(body.starts_with(quoted(&text)), "excerpt: {text}");
    assert!(quoted(&text).chars().all(|character| character == '→'));

    let line = "πλαίσιο γραμμής με → και ✓\n";
    let body = format!(
        "{}needle μέσα σε πολύγλωσσο κείμενο\n{}",
        line.repeat(40),
        line.repeat(40)
    );
    let (text, _) = excerpt(&body, &["needle"]);
    assert!(body.contains(quoted(&text)), "excerpt: {text}");
    assert!(text.contains("needle μέσα σε πολύγλωσσο κείμενο"));
}

/// The window ends deep inside a fenced block it cannot back out of without
/// dropping the match, so the excerpt closes the fence itself. `--json` emits
/// the excerpt raw, with no outer fence to hide a dangling one.
#[test]
fn an_unclosed_fence_is_never_emitted() {
    let code = "let value = compute();\n";
    let body = format!(
        "{}```rust\n{}let needle = value + 1;\n{}```\n{}",
        FILLER.repeat(30),
        code.repeat(5),
        code.repeat(200),
        FILLER.repeat(50)
    );

    let (text, _) = excerpt(&body, &["needle"]);
    assert!(text.contains("let needle = value + 1;"), "excerpt: {text}");
    let fences = text.matches("```").count();
    assert_eq!(fences % 2, 0, "unbalanced fences ({fences}): {text}");
    assert_eq!(fences, 2, "excerpt: {text}");
}

/// The other half of the fence contract, unchanged: when the block opens after
/// the match, the window stops before it rather than quoting a headless block.
#[test]
fn a_fence_opening_after_the_match_is_left_out_of_the_window() {
    let code = "let value = compute();\n";
    let body = format!(
        "needle line at the top\n{}```rust\n{}",
        FILLER.repeat(20),
        code.repeat(300)
    );

    let (text, _) = excerpt(&body, &["needle"]);
    assert!(text.contains("needle line at the top"), "excerpt: {text}");
    assert!(!text.contains("```"), "quoted a headless block: {text}");
}

/// The window's best-matching line sits well inside a fenced block that opened
/// long before the window starts. Without an opening fence of its own, the
/// excerpt's only fence is the block's closer, which reads as opening a new
/// block in any markdown renderer.
#[test]
fn a_window_starting_inside_a_fenced_block_opens_it() {
    let code = "let value = compute();\n";
    let body = format!(
        "{}```rust\n{}let needle = value + 1;\n{}```\n{}",
        FILLER.repeat(200),
        code.repeat(200),
        code.repeat(5),
        FILLER.repeat(30)
    );

    let (text, omitted) = excerpt(&body, &["needle"]);
    assert!(omitted);
    assert!(text.contains("let needle = value + 1;"), "excerpt: {text}");
    let fences = text.matches("```").count();
    assert_eq!(fences % 2, 0, "unbalanced fences ({fences}): {text}");
    assert_eq!(fences, 2, "excerpt: {text}");
    let quoted = quoted(&text);
    assert!(
        quoted.starts_with("```"),
        "first fence must open, not close: {text}"
    );
}

/// The compose case: the window both starts and ends inside the same fenced
/// block, which never closes anywhere in the body. The excerpt owes its reader
/// both an opening fence at the top and a closing fence at the bottom.
#[test]
fn a_window_inside_a_fence_that_never_closes_gets_both_fences() {
    let code = "let value = compute();\n";
    let body = format!(
        "{}```rust\n{}let needle = value + 1;\n{}",
        FILLER.repeat(200),
        code.repeat(200),
        code.repeat(200)
    );

    let (text, omitted) = excerpt(&body, &["needle"]);
    assert!(omitted);
    assert!(text.contains("let needle = value + 1;"), "excerpt: {text}");
    let fences = text.matches("```").count();
    assert_eq!(fences % 2, 0, "unbalanced fences ({fences}): {text}");
    assert_eq!(fences, 2, "excerpt: {text}");
    let quoted = quoted(&text);
    assert!(
        quoted.starts_with("```"),
        "first fence must open, not close: {text}"
    );
    assert!(
        quoted.trim_end().ends_with("```"),
        "last fence must close: {text}"
    );
}
