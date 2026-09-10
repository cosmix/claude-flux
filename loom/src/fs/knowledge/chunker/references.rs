//! Typed evidence extracted from backticked markdown spans.

use regex::Regex;
use std::collections::BTreeSet;
use std::sync::LazyLock;

static SOURCE_PATH_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z0-9_$~<>./-]+\.(rs|tsx|ts|py|go|sh|md|toml|yaml|yml)")
        .unwrap_or_else(|error| panic!("source path regex must be valid: {error}"))
});
static SYMBOL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*(::[A-Za-z_][A-Za-z0-9_]*)*$")
        .unwrap_or_else(|error| panic!("symbol regex must be valid: {error}"))
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum EvidenceKind {
    Live,
    Example,
    Runtime,
    External,
    Historical,
}

impl EvidenceKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Example => "example",
            Self::Runtime => "runtime",
            Self::External => "external",
            Self::Historical => "historical",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct EvidenceReference {
    pub(crate) source_path: String,
    pub(crate) kind: EvidenceKind,
}

/// Classify a backticked path using its sentence-local role.
pub(crate) fn classify_reference(span: &str, sentence: &str) -> EvidenceKind {
    let path = span.trim().to_lowercase();
    let sentence = sentence.to_lowercase();
    if is_runtime_path(&path) {
        return EvidenceKind::Runtime;
    }
    if contains_any(&path, EXAMPLE_MARKERS) || contains_any(&sentence, EXAMPLE_MARKERS) {
        return EvidenceKind::Example;
    }
    if is_there_is_no_this_path(&sentence, &path) || contains_any(&sentence, HISTORICAL_MARKERS) {
        return EvidenceKind::Historical;
    }
    if contains_any(&sentence, EXTERNAL_MARKERS) {
        return EvidenceKind::External;
    }
    EvidenceKind::Live
}

const EXAMPLE_MARKERS: &[&str] = &[
    "foo",
    "bar",
    "baz",
    "<",
    "...",
    "…",
    "path/to",
    "slug",
    "newcmd",
    "example",
    "e.g.",
    "placeholder",
    "nn-",
    "xxx",
];
const HISTORICAL_MARKERS: &[&str] = &[
    "does not exist",
    "do not exist",
    "no longer",
    "never existed",
    "invented",
    "earlier version",
    "was deleted",
    "was removed",
    "removed in",
    "deleted in",
    "renamed to",
    "used to",
];
const EXTERNAL_MARKERS: &[&str] = &[
    "external",
    "upstream",
    "another project",
    "plugin's",
    "openai",
    "codex-rs",
    "claude code's",
];

fn contains_any(value: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| value.contains(marker))
}

/// True only when "there is no" is immediately followed by this exact
/// backticked path, e.g. "there is no `gc.rs`" — not "there is no reason to
/// change `loom/src/x.rs`", where the phrase names no particular path at all.
/// Both `sentence` and `path` are already lowercased by the caller.
fn is_there_is_no_this_path(sentence: &str, path: &str) -> bool {
    sentence.contains(&format!("there is no `{path}`"))
}

fn is_runtime_path(path: &str) -> bool {
    path.starts_with(".loom/")
        || path.starts_with(".work/")
        || path.starts_with("target/")
        || path.starts_with("node_modules/")
        || path.starts_with('~')
        || path.starts_with("/tmp")
        || path.starts_with('$')
        || path.contains('<')
        || path.contains('>')
}

/// Extract typed source references and identifiers outside fenced code blocks.
pub(crate) fn references_in(body: &str) -> (Vec<EvidenceReference>, Vec<String>) {
    let mut references = Vec::new();
    let mut symbols = Vec::new();
    let mut fence = None;

    for raw_line in body.split_inclusive('\n') {
        let line = raw_line.trim_end_matches(['\n', '\r']);
        let marker = super::fence_marker(line);
        if let Some(open_fence) = fence {
            if marker == Some(open_fence) {
                fence = None;
            }
            continue;
        }
        if let Some(marker) = marker {
            fence = Some(marker);
            continue;
        }
        references_in_line(line, &mut references, &mut symbols);
    }

    (deduplicate(references), deduplicate(symbols))
}

fn references_in_line(
    line: &str,
    references: &mut Vec<EvidenceReference>,
    symbols: &mut Vec<String>,
) {
    let mut span_start = None;
    for (position, character) in line.char_indices() {
        if character != '`' {
            continue;
        }
        if let Some(start) = span_start.take() {
            let span = &line[start..position];
            let sentence = sentence_window(line, start.saturating_sub(1), position + 1);
            for matched in SOURCE_PATH_REGEX.find_iter(span) {
                let followed_by_identifier = span[matched.end()..]
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_alphanumeric());
                if !followed_by_identifier {
                    let source_path = matched.as_str().to_string();
                    references.push(EvidenceReference {
                        kind: classify_reference(&source_path, sentence),
                        source_path,
                    });
                }
            }
            let symbol = span.trim();
            if SYMBOL_REGEX.is_match(symbol) {
                symbols.push(symbol.to_string());
            }
        } else {
            span_start = Some(position + character.len_utf8());
        }
    }
}

fn sentence_window(line: &str, opening: usize, after_closing: usize) -> &str {
    let start = line[..opening]
        .rfind('.')
        .map_or(0, |position| position + 1);
    let end = line[after_closing..]
        .find('.')
        .map_or(line.len(), |position| after_closing + position + 1);
    &line[start..end]
}

fn deduplicate<T>(values: Vec<T>) -> Vec<T>
where
    T: Clone + Ord,
{
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}
