//! Match-centred, line-aligned excerpts for packed context items.
//!
//! The window obeys three properties, in this order of precedence:
//!
//! 1. **The focus line is always inside the window.** When a line matches the
//!    query, `body[start..end]` contains that whole line. A single line longer
//!    than [`EXCERPT_BYTE_LIMIT`] becomes the window on its own and the excerpt
//!    exceeds the limit: an over-long excerpt is charged honestly by the packer,
//!    an excerpt missing the match it was selected for is a lie.
//! 2. **It centres.** The budget left over after the focus line is spent half
//!    backwards and half forwards, aligned to line boundaries, and whatever one
//!    side cannot use (because the body ends) goes to the other.
//! 3. **Markers mean what they say.** The `[… earlier lines omitted]` prefix
//!    appears exactly when `start > 0`, [`EXCERPT_TRUNCATION_MARKER`] exactly
//!    when `end < body.len()`, and the returned flag is `start > 0 || end <
//!    body.len()`.
//!
//! With no matching term the leading window is used instead, cut at the last
//! whole line inside the limit — or, for a body whose first line already
//! exceeds it, at the last whole character.

use crate::context::lexical::tokenize;
use crate::context::schema::{
    estimate_tokens, BYTES_PER_TOKEN_ESTIMATE, EXCERPT_MAX_TOKENS, EXCERPT_TRUNCATION_MARKER,
};
use std::collections::BTreeSet;
use std::ops::Range;

const EARLIER_LINES_MARKER: &str = "[… earlier lines omitted]";

/// The byte budget a window aims for. Only property 1 above may overrun it.
const EXCERPT_BYTE_LIMIT: usize = EXCERPT_MAX_TOKENS * BYTES_PER_TOKEN_ESTIMATE;

/// Return the bounded representation of `body` and whether any text was omitted.
pub(crate) fn bounded_excerpt(body: &str, terms: &[String]) -> (String, bool) {
    if estimate_tokens(body) <= EXCERPT_MAX_TOKENS {
        return (body.to_string(), false);
    }

    let lines = line_ranges(body);
    let best_line = best_matching_line(body, &lines, terms);
    let window = select_window(body, &lines, best_line);
    let focus_end = best_line
        .map(|index| lines[index].end)
        .unwrap_or(window.start);

    let (opening_fence, repair) = fence_repair(body, &window, focus_end);
    let (window, closing_fence) = match repair {
        FenceRepair::Balanced => (window, None),
        FenceRepair::Trim(fence_start) => (window.start..fence_start, None),
        FenceRepair::Close(fence) => (window, Some(fence)),
    };

    let omitted = window.start > 0 || window.end < body.len();
    (
        assemble(
            body,
            &window,
            opening_fence.as_deref(),
            closing_fence.as_deref(),
        ),
        omitted,
    )
}

/// Join the window to its markers: the omission prefix, the opening fence a
/// window that starts inside a code block owes its reader, the quoted slice,
/// the closing fence a window that ended inside a code block owes its reader,
/// and the truncation marker.
fn assemble(
    body: &str,
    window: &Range<usize>,
    opening_fence: Option<&str>,
    closing_fence: Option<&str>,
) -> String {
    let mut excerpt = String::new();
    if window.start > 0 {
        excerpt.push_str(EARLIER_LINES_MARKER);
        excerpt.push('\n');
    }
    if let Some(fence) = opening_fence {
        excerpt.push_str(fence);
        excerpt.push('\n');
    }
    excerpt.push_str(&body[window.clone()]);
    if let Some(fence) = closing_fence {
        if !excerpt.ends_with('\n') {
            excerpt.push('\n');
        }
        excerpt.push_str(fence);
        excerpt.push('\n');
    }
    if window.end < body.len() {
        if !excerpt.ends_with('\n') {
            excerpt.push('\n');
        }
        excerpt.push_str(EXCERPT_TRUNCATION_MARKER);
    }
    excerpt
}

fn line_ranges(body: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for (index, character) in body.char_indices() {
        if character == '\n' {
            ranges.push(start..index + 1);
            start = index + 1;
        }
    }
    if start < body.len() {
        ranges.push(start..body.len());
    }
    ranges
}

fn best_matching_line(body: &str, lines: &[Range<usize>], terms: &[String]) -> Option<usize> {
    let wanted: BTreeSet<&str> = terms.iter().map(String::as_str).collect();
    if wanted.is_empty() {
        return None;
    }

    let mut best = None;
    let mut best_count = 0;
    for (index, range) in lines.iter().enumerate() {
        let present: BTreeSet<String> = tokenize(&body[range.clone()]).into_iter().collect();
        let count = wanted
            .iter()
            .filter(|term| present.contains(**term))
            .count();
        if count > best_count {
            best = Some(index);
            best_count = count;
        }
    }
    best
}

fn select_window(body: &str, lines: &[Range<usize>], best_line: Option<usize>) -> Range<usize> {
    match best_line {
        Some(index) => centred_window(lines, index),
        None => 0..leading_end(body, lines),
    }
}

/// The window around `lines[index]`, always containing that line in full.
fn centred_window(lines: &[Range<usize>], index: usize) -> Range<usize> {
    let focus = lines[index].clone();
    let Some(spare) = EXCERPT_BYTE_LIMIT.checked_sub(focus.end - focus.start) else {
        return focus;
    };

    let backwards = expand_back(lines, index, focus.start, spare / 2);
    let end = expand_forward(lines, index, focus.end, spare - (focus.start - backwards));
    // Whatever the forward side left unspent — because the body ended — is
    // handed back to the backward side, so a match near the end still fills the
    // window rather than stopping half short.
    let start = expand_back(lines, index, focus.start, spare - (end - focus.end));
    start..end
}

/// The earliest line start within `budget` bytes of `focus_start`.
fn expand_back(lines: &[Range<usize>], index: usize, focus_start: usize, budget: usize) -> usize {
    let mut start = focus_start;
    for line in lines[..index].iter().rev() {
        if focus_start - line.start > budget {
            break;
        }
        start = line.start;
    }
    start
}

/// The latest line end within `budget` bytes of `focus_end`.
fn expand_forward(lines: &[Range<usize>], index: usize, focus_end: usize, budget: usize) -> usize {
    let mut end = focus_end;
    for line in &lines[index + 1..] {
        if line.end - focus_end > budget {
            break;
        }
        end = line.end;
    }
    end
}

/// Where a leading window stops: the last whole line inside the byte limit, or
/// the last whole character when the first line alone overruns it.
fn leading_end(body: &str, lines: &[Range<usize>]) -> usize {
    let raw_end = char_boundary_at_or_before(body, EXCERPT_BYTE_LIMIT);
    if raw_end == body.len() {
        return raw_end;
    }
    lines
        .iter()
        .filter(|range| range.end <= raw_end)
        .map(|range| range.end)
        .next_back()
        .unwrap_or(raw_end)
}

fn char_boundary_at_or_before(body: &str, requested: usize) -> usize {
    let mut end = requested.min(body.len());
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// What a window that stops inside an unclosed fenced block owes its reader.
/// Decided together with, but independently of, whether the window also
/// *starts* inside one (see [`fence_repair`]): a window can owe an opening
/// fence, a closing fence, both, or neither.
enum FenceRepair {
    /// Every fence opened inside the window is closed inside it.
    Balanced,
    /// Stop before the dangling fence instead: the focus line precedes it, so
    /// dropping the partial block costs nothing. Never chosen for a fence the
    /// window inherited already open at `window.start` — there is nothing
    /// before it to trim back to.
    Trim(usize),
    /// The focus line is inside the block, so the block has to stay. Emit this
    /// closing fence after it.
    Close(String),
}

/// The repair a window needs at both ends: an opening fence to emit if the
/// window starts inside a block opened earlier in `body`, and a
/// [`FenceRepair`] for how the window's own tail should be closed. The two
/// compose from a single fence scan seeded with the state inherited at
/// `window.start`, rather than each assuming the window starts balanced.
fn fence_repair(
    body: &str,
    window: &Range<usize>,
    focus_end: usize,
) -> (Option<String>, FenceRepair) {
    let inherited = open_fence(&body[..window.start], 0, None);
    let opening = inherited.map(|(marker, width, _)| marker.to_string().repeat(width));

    let repair = match open_fence(&body[window.clone()], window.start, inherited) {
        None => FenceRepair::Balanced,
        Some((_, _, fence_start)) if fence_start > window.start && focus_end <= fence_start => {
            FenceRepair::Trim(fence_start)
        }
        Some((marker, width, _)) => FenceRepair::Close(marker.to_string().repeat(width)),
    };
    (opening, repair)
}

/// The marker, width, and body offset of the fence left open at the end of
/// `text`, whose first byte sits at `base` in the body. `open` seeds the scan
/// with the fence state already in effect at `base`, so a block opened before
/// `text` starts can be tracked across the boundary.
fn open_fence(
    text: &str,
    base: usize,
    mut open: Option<(char, usize, usize)>,
) -> Option<(char, usize, usize)> {
    let mut offset = base;
    for line in text.split_inclusive('\n') {
        if let Some((marker, width)) = fence_marker(line) {
            match open {
                Some((open_marker, open_width, _))
                    if marker == open_marker && width >= open_width =>
                {
                    open = None;
                }
                None => open = Some((marker, width, offset)),
                _ => {}
            }
        }
        offset += line.len();
    }
    open
}

fn fence_marker(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start();
    let marker = trimmed.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let width = trimmed
        .chars()
        .take_while(|character| *character == marker)
        .count();
    (width >= 3).then_some((marker, width))
}
