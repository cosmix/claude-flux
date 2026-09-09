//! Shared per-item rendering for context consumers.

use crate::context::schema::{estimate_tokens, Confidence, ContextItem, ItemKind};
use crate::context::untrusted::inline_safe;

/// One knowledge item's full rendering: its list entry, plus its fenced
/// excerpt block when it carries one, plus a trailing blank line separating
/// it from whatever renders next — another item, or the next section.
pub(crate) fn render_knowledge_item(item: &ContextItem) -> String {
    let mut out = render_knowledge_item_line(item);
    if let Some(excerpt) = &item.excerpt {
        out.push('\n');
        out.push_str(&render_excerpt_block(excerpt));
    }
    out.push('\n');
    out
}

/// One knowledge item's list entry: `` - `<id>` `` plus, only when the
/// rendered pointer differs from the id, `` — `<pointer>` ``, followed by its
/// Reason/state line.
///
/// The id and the pointer are untrusted and go through [`inline_safe`]. The
/// reasons, the confidence and the state do not: `SelectionReason`,
/// `Confidence` and `LifecycleState` are fieldless enums rendered from a fixed
/// set of literals, so none of them can carry caller text.
fn render_knowledge_item_line(item: &ContextItem) -> String {
    let pointer = render_pointer(item);
    let mut line = format!("- `{}`", inline_safe(item.id.as_str()));
    if pointer != item.id.as_str() {
        line.push_str(&format!(" — `{}`", inline_safe(&pointer)));
    }
    line.push('\n');
    line.push_str(&format!(
        "  Reason: {} | state: {}\n",
        render_reasons(item),
        item.state
    ));
    line
}

/// Join an item's [`SelectionReason`]s the way both sections render them,
/// followed by `; <confidence>` when that confidence is below `High`.
///
/// Reasons alone do not tell the reader how much to trust the hit: the packer
/// publishes the WEAKER of the reasons-implied confidence and the rung ceiling
/// (`context::rank::RankedCandidate::confidence`), so a node admitted on
/// rarity alone reads as `exact-symbol` yet is only Medium. Safe to print
/// unescaped — see [`render_knowledge_item_line`]'s doc comment.
///
/// [`SelectionReason`]: crate::context::schema::SelectionReason
pub(crate) fn render_reasons(item: &ContextItem) -> String {
    let reasons = item
        .reasons
        .iter()
        .map(|reason| reason.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    match confidence_word(item.confidence) {
        Some(word) => format!("{reasons}; {word}"),
        None => reasons,
    }
}

/// The word naming a confidence worth flagging, or `None` for `High`.
///
/// High is the common case, and every token spent restating it is a token the
/// brief's excerpts do not get: the rendering there stays byte-identical to
/// what it was before this label existed, and only a demoted item pays for
/// saying so.
pub(crate) fn confidence_word(confidence: Confidence) -> Option<&'static str> {
    match confidence {
        Confidence::High => None,
        Confidence::Medium => Some("medium"),
        Confidence::Low => Some("low"),
    }
}

/// `<path>`, plus the line span and the `#<anchor>` each when present.
fn render_pointer(item: &ContextItem) -> String {
    let mut rendered = item.pointer.path.display().to_string();
    if let Some(span) = render_span(item) {
        rendered.push_str(&span);
    }
    if !item.pointer.anchor.is_empty() {
        rendered.push_str(&format!("#{}", item.pointer.anchor));
    }
    rendered
}

/// `:<line-start>` alone, or `:<line-start>-<line-end>` when both are known.
/// `None` when the item carries no span at all.
fn render_span(item: &ContextItem) -> Option<String> {
    let start = item.pointer.line_start?;
    Some(match item.pointer.line_end {
        Some(end) => format!(":{start}-{end}"),
        None => format!(":{start}"),
    })
}

/// A fenced, escape-proof excerpt block.
pub(crate) fn render_excerpt_block(excerpt: &str) -> String {
    let fence = fence_for(excerpt);
    format!("{fence}text\n{excerpt}\n{fence}\n")
}

/// A backtick fence at least one longer than the longest backtick run already
/// present in `text`, and never shorter than 3.
pub(crate) fn fence_for(text: &str) -> String {
    let mut longest = 0usize;
    let mut current = 0usize;
    for ch in text.chars() {
        if ch == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    "`".repeat((longest + 1).max(3))
}

/// One item's fragment of a grouped source bullet: `` `<name>` <kind>
/// :<span> (<reasons>) ``, or `` `<id>` :<span> (<reasons>) `` when the id
/// does not split into `<path>#<kind>:<scope>`
/// (`context::source_graph::node_id`) — a fallback that renders the whole id
/// rather than inventing a name that could mislead. The parentheses carry a
/// trailing `; medium` or `; low` for a demoted item — see [`render_reasons`].
pub(crate) fn render_source_entry(item: &ContextItem) -> String {
    let mut parts = match parse_source_identity(item.id.as_str()) {
        Some((kind, name)) => vec![format!("`{}`", inline_safe(name)), inline_safe(kind)],
        None => vec![format!("`{}`", inline_safe(item.id.as_str()))],
    };
    parts.extend(render_span(item));
    parts.push(format!("({})", render_reasons(item)));
    parts.join(" ")
}

/// Split a source node id (`<path>#<kind>:<scope>`, see
/// `context::source_graph::node_id`) into `(kind, scope)`. `None` when the id
/// does not carry that shape.
fn parse_source_identity(id: &str) -> Option<(&str, &str)> {
    let (_, suffix) = id.split_once('#')?;
    let (kind, name) = suffix.split_once(':')?;
    (!kind.is_empty() && !name.is_empty()).then_some((kind, name))
}

/// Estimated tokens of the exact text this item renders to inside a brief.
#[allow(dead_code)]
pub(crate) fn rendered_item_tokens(item: &ContextItem) -> usize {
    let rendered = match item.kind {
        ItemKind::KnowledgeChunk => render_knowledge_item(item),
        ItemKind::SourceNode => render_source_entry(item),
    };
    estimate_tokens(&rendered)
}
