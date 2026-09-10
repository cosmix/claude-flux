//! Split curated knowledge markdown files into retrievable H2 sections.

use crate::context::schema::LifecycleState;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

pub(crate) mod references;

/// Re-export the canonical knowledge chunk type for knowledge callers.
pub use crate::context::schema::KnowledgeChunk;

static LINK_REGEX: LazyLock<Regex> =
    LazyLock::new(|| match Regex::new(r"\[([^\]]+)\]\(([^)]+\.md)\)") {
        Ok(regex) => regex,
        Err(error) => panic!("link regex must be valid: {error}"),
    });

/// Build one `KnowledgeChunk` from a section at `index`, tracking heading
/// occurrence counts in `occurrences` (shared across a file's sections) so
/// each chunk's anchor stays unique even under repeated headings.
#[allow(clippy::too_many_arguments)]
fn build_chunk(
    index: usize,
    section: Section<'_>,
    relative_path: &str,
    category: &Option<String>,
    state: LifecycleState,
    frontmatter: &crate::fs::knowledge::frontmatter::Frontmatter,
    occurrences: &mut std::collections::BTreeMap<String, usize>,
) -> KnowledgeChunk {
    let body = trim_trailing_blank_lines(section.body);
    let normalized_heading = normalize_heading(section.heading);
    let occurrence = occurrences.entry(normalized_heading.clone()).or_insert(0);
    let derived_id = format!("{relative_path}#{normalized_heading}#{occurrence}");
    *occurrence += 1;

    let (references, symbols) = references::references_in(&body);
    let source_paths = live_source_paths(index, references, frontmatter);
    let content_hash = format!("sha256:{}", hex::encode(Sha256::digest(body.as_bytes())));
    // This is an estimate, not a tokenizer count.
    let estimated_tokens = crate::context::schema::estimate_tokens(&body);
    let links = links_in(&body);

    KnowledgeChunk {
        id: if index == 0 {
            frontmatter.id.clone().unwrap_or(derived_id)
        } else {
            derived_id
        },
        file: PathBuf::from(relative_path),
        anchor: normalized_heading,
        heading: section.heading.trim().to_string(),
        body,
        content_hash,
        estimated_tokens,
        aliases: if index == 0 {
            frontmatter.aliases.clone()
        } else {
            Vec::new()
        },
        category: category.clone(),
        source_paths,
        symbols,
        links,
        state,
    }
}

/// Split a knowledge markdown file into a preamble and its H2 sections.
pub fn chunk_file(path: &Path, bytes: &[u8]) -> anyhow::Result<Vec<KnowledgeChunk>> {
    let text = String::from_utf8_lossy(bytes);
    let (frontmatter, content) = crate::fs::knowledge::frontmatter::split_frontmatter(&text);
    Ok(chunk_sections(path, content, &frontmatter))
}

/// [`chunk_file`]'s body, taking already-split frontmatter and content
/// directly. Exists so a caller that already parsed frontmatter for its own
/// purposes (`catalog::process_file`'s size/blurb checks) does not have to
/// parse it a second time just to chunk the file.
pub(crate) fn chunk_sections(
    path: &Path,
    content: &str,
    frontmatter: &crate::fs::knowledge::frontmatter::Frontmatter,
) -> Vec<KnowledgeChunk> {
    let sections = split_sections(content);
    let relative_path = display_path(path);
    let category = category_for(path);
    let state = frontmatter.state.unwrap_or_default();
    let mut occurrences: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();

    sections
        .into_iter()
        .enumerate()
        .map(|(index, section)| {
            build_chunk(
                index,
                section,
                &relative_path,
                &category,
                state,
                frontmatter,
                &mut occurrences,
            )
        })
        .collect()
}

/// Live-classified backticked source references for one section, with the
/// frontmatter's declared `sources:` folded into the file's first chunk —
/// mirrors [`build_chunk`]'s `index == 0` special case for `id`/`aliases`.
fn live_source_paths(
    index: usize,
    references: Vec<references::EvidenceReference>,
    frontmatter: &crate::fs::knowledge::frontmatter::Frontmatter,
) -> Vec<String> {
    let live = references
        .into_iter()
        .filter(|reference| reference.kind == references::EvidenceKind::Live)
        .map(|reference| reference.source_path);
    if index == 0 {
        deduplicate(frontmatter.sources.iter().cloned().chain(live))
    } else {
        live.collect()
    }
}

struct Section<'a> {
    body: &'a str,
    heading: &'a str,
}

fn split_sections(content: &str) -> Vec<Section<'_>> {
    let mut split_points = Vec::new();
    let mut fence = None;
    let mut offset = 0;

    while let Some((line_end, line)) = line_at(content, offset) {
        let fence_marker = fence_marker(line);
        if let Some(open_fence) = fence {
            if fence_marker == Some(open_fence) {
                fence = None;
            }
        } else if let Some(marker) = fence_marker {
            fence = Some(marker);
        } else if line.starts_with("## ") {
            split_points.push(offset);
        }
        offset = line_end;
    }

    let mut sections = Vec::new();
    if let Some(&first_split) = split_points.first() {
        if !content[..first_split].trim().is_empty() {
            sections.push(Section {
                body: &content[..first_split],
                heading: "",
            });
        }
    } else if !content.trim().is_empty() {
        sections.push(Section {
            body: content,
            heading: "",
        });
    }

    for (position, start) in split_points.iter().enumerate() {
        let end = split_points
            .get(position + 1)
            .copied()
            .unwrap_or(content.len());
        let (_, line) = line_at(content, *start).unwrap_or((end, ""));
        sections.push(Section {
            body: &content[*start..end],
            heading: line.strip_prefix("## ").unwrap_or_default(),
        });
    }
    sections
}

fn line_at(text: &str, start: usize) -> Option<(usize, &str)> {
    if start >= text.len() {
        return None;
    }
    let end = text[start..]
        .find('\n')
        .map(|position| start + position + 1)
        .unwrap_or(text.len());
    let line = text[start..end].trim_end_matches(['\n', '\r']);
    Some((end, line))
}

pub(crate) fn fence_marker(line: &str) -> Option<char> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("```") {
        Some('`')
    } else if trimmed.starts_with("~~~") {
        Some('~')
    } else {
        None
    }
}

fn trim_trailing_blank_lines(body: &str) -> String {
    let mut offset = 0;
    let mut last_non_blank_end = 0;
    while let Some((line_end, line)) = line_at(body, offset) {
        if !line.trim().is_empty() {
            last_non_blank_end = line_end;
        }
        offset = line_end;
    }
    body[..last_non_blank_end].to_string()
}

fn normalize_heading(heading: &str) -> String {
    let mut normalized = String::new();
    let mut needs_separator = false;
    for character in heading.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            if needs_separator && !normalized.is_empty() {
                normalized.push('-');
            }
            normalized.push(character);
            needs_separator = false;
        } else if !normalized.is_empty() {
            needs_separator = true;
        }
    }
    normalized
}

fn links_in(body: &str) -> Vec<(String, String)> {
    LINK_REGEX
        .captures_iter(body)
        .filter_map(|captures| {
            Some((
                captures.get(1)?.as_str().to_string(),
                captures.get(2)?.as_str().to_string(),
            ))
        })
        .collect()
}

fn deduplicate(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn category_for(path: &Path) -> Option<String> {
    let components: Vec<_> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    components.get(components.len().checked_sub(2)?).cloned()
}
