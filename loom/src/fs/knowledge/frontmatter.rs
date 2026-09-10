//! Read and update the leading YAML frontmatter of a knowledge document.

use crate::context::schema::LifecycleState;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

const KNOWN_KEYS: &[&str] = &["id", "aliases", "state", "sources", "verified"];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Frontmatter {
    pub(crate) id: Option<String>,
    pub(crate) aliases: Vec<String>,
    pub(crate) state: Option<LifecycleState>,
    pub(crate) sources: Vec<String>,
    pub(crate) verified: Option<String>,
    unknown_yaml: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct KnownFrontmatter {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    aliases: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<LifecycleState>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    sources: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verified: Option<String>,
}

impl Frontmatter {
    fn parse(raw: &str) -> Result<Self> {
        let known = if raw.trim().is_empty() {
            KnownFrontmatter::default()
        } else {
            serde_yaml::from_str(raw).context("Invalid YAML frontmatter")?
        };
        Ok(Self {
            id: known.id,
            aliases: known.aliases,
            state: known.state,
            sources: known.sources,
            verified: known.verified,
            unknown_yaml: unknown_yaml(raw),
        })
    }

    fn render_block(&self) -> Result<String> {
        let known = KnownFrontmatter {
            id: self.id.clone(),
            aliases: self.aliases.clone(),
            state: self.state,
            sources: self.sources.clone(),
            verified: self.verified.clone(),
        };
        let mut yaml = serde_yaml::to_string(&known).context("Failed to serialize frontmatter")?;
        if yaml == "{}\n" {
            yaml.clear();
        }
        if !self.unknown_yaml.is_empty() {
            if !yaml.is_empty() && !yaml.ends_with('\n') {
                yaml.push('\n');
            }
            yaml.push_str(&self.unknown_yaml);
            if !yaml.ends_with('\n') {
                yaml.push('\n');
            }
        }
        Ok(format!("---\n{yaml}---\n"))
    }
}

/// Parse frontmatter for readers. Malformed YAML is stripped but contributes
/// no metadata, preserving the chunker's long-standing best-effort behavior.
pub(crate) fn split_frontmatter(text: &str) -> (Frontmatter, &str) {
    let Some((raw, body)) = raw_frontmatter(text) else {
        return (Frontmatter::default(), text);
    };
    (Frontmatter::parse(raw).unwrap_or_default(), body)
}

pub(crate) fn file_frontmatter(bytes: &[u8]) -> Frontmatter {
    let text = String::from_utf8_lossy(bytes);
    split_frontmatter(&text).0
}

pub(crate) fn file_state(bytes: &[u8]) -> Option<LifecycleState> {
    file_frontmatter(bytes).state
}

/// Apply a fallible metadata mutation under the same parent-directory lock as
/// all other knowledge writers, returning the resulting frontmatter block.
pub(crate) fn update_file<F>(path: &Path, update: F) -> Result<String>
where
    F: FnOnce(&mut Frontmatter) -> Result<()>,
{
    let mut rendered = None;
    crate::fs::locking::locked_update(path, |content| {
        if content.is_empty() {
            bail!(
                "Cannot annotate missing or empty knowledge file: {}",
                path.display()
            );
        }
        let (mut frontmatter, body) = match raw_frontmatter(&content) {
            Some((raw, body)) => (Frontmatter::parse(raw)?, body),
            None => (Frontmatter::default(), content.as_str()),
        };
        update(&mut frontmatter)?;
        let block = frontmatter.render_block()?;
        rendered = Some(block.clone());
        Ok(format!("{block}{body}"))
    })?;
    rendered.context("frontmatter update did not run")
}

fn raw_frontmatter(text: &str) -> Option<(&str, &str)> {
    let (first_end, first_line) = line_at(text, 0)?;
    if first_line != "---" {
        return None;
    }
    let mut offset = first_end;
    while let Some((line_end, line)) = line_at(text, offset) {
        if line == "---" {
            return Some((&text[first_end..offset], &text[line_end..]));
        }
        offset = line_end;
    }
    None
}

fn line_at(text: &str, start: usize) -> Option<(usize, &str)> {
    if start >= text.len() {
        return None;
    }
    let end = text[start..]
        .find('\n')
        .map_or(text.len(), |position| start + position + 1);
    Some((end, text[start..end].trim_end_matches(['\n', '\r'])))
}

/// Preserve unknown top-level key blocks byte-for-byte while known fields are
/// rewritten canonically. Unknown key order and multiline values are retained.
fn unknown_yaml(raw: &str) -> String {
    let mut output = String::new();
    let mut keep_block = true;
    for line in raw.split_inclusive('\n') {
        if let Some(key) = top_level_key(line) {
            keep_block = !KNOWN_KEYS.contains(&key);
        }
        if keep_block {
            output.push_str(line);
        }
    }
    output
}

fn top_level_key(line: &str) -> Option<&str> {
    let line = line.trim_end_matches(['\n', '\r']);
    if line.is_empty()
        || line.chars().next().is_some_and(char::is_whitespace)
        || line.starts_with('#')
    {
        return None;
    }
    let (key, _) = line.split_once(':')?;
    Some(key.trim().trim_matches(['\'', '"']))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn update_preserves_unknown_yaml_and_body() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("topic.md");
        let body = "# Topic\n\nBody.\n";
        fs::write(
            &path,
            format!("---\nowner:\n  team: docs\nstate: draft\n---\n{body}"),
        )
        .unwrap();

        update_file(&path, |frontmatter| {
            frontmatter.state = Some(LifecycleState::Active);
            Ok(())
        })
        .unwrap();

        let updated = fs::read_to_string(path).unwrap();
        assert!(updated.contains("owner:\n  team: docs\n"));
        assert!(updated.ends_with(body));
    }
}
