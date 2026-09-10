//! `loom knowledge annotate` — update evidence metadata and topic blurbs.

use crate::cli::AnnotateArgs;
use crate::context::schema::LifecycleState;
use crate::fs::knowledge::catalog::prose::project_root_of;
use crate::fs::knowledge::frontmatter;
use crate::fs::knowledge::index::MAX_BLURB_CHARS;
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Default)]
pub(super) struct Annotation {
    pub(super) state: Option<LifecycleState>,
    pub(super) sources: Vec<String>,
    pub(super) clear_sources: bool,
    pub(super) verified: Option<String>,
    pub(super) aliases: Vec<String>,
    pub(super) blurb: Option<String>,
}

pub(crate) fn annotate(args: AnnotateArgs) -> Result<()> {
    let AnnotateArgs {
        target,
        state,
        source: sources,
        clear_sources,
        verified,
        alias: aliases,
        blurb,
    } = args;
    let knowledge = super::open_knowledge_dir()?;
    let target = crate::fs::knowledge::KnowledgeTarget::parse(&target)?;
    let project_root = project_root_of(knowledge.root())
        .context("Could not determine project root for knowledge annotation")?;
    let annotation = Annotation {
        state: state.as_deref().map(parse_state).transpose()?,
        sources,
        clear_sources,
        verified: verified
            .as_deref()
            .map(|revision| resolve_revision(&project_root, revision))
            .transpose()?,
        aliases,
        blurb,
    };

    let rendered = annotate_path(&knowledge.target_path(&target), &annotation)?;
    knowledge.refresh_index_if_hierarchical();
    println!("{}", rendered.trim_end());
    Ok(())
}

pub(super) fn annotate_path(path: &Path, annotation: &Annotation) -> Result<String> {
    validate_annotation(annotation)?;
    let rendered = frontmatter::update_file(path, |metadata| {
        if let Some(state) = annotation.state {
            metadata.state = Some(state);
        }
        if annotation.clear_sources {
            metadata.sources.clear();
        }
        extend_unique(&mut metadata.sources, &annotation.sources);
        if let Some(verified) = &annotation.verified {
            metadata.verified = Some(verified.clone());
        }
        extend_unique(&mut metadata.aliases, &annotation.aliases);
        Ok(())
    })?;
    if let Some(blurb) = &annotation.blurb {
        update_blurb(path, blurb)?;
    }
    Ok(rendered)
}

pub(super) fn parse_state(state: &str) -> Result<LifecycleState> {
    match state.trim().to_lowercase().as_str() {
        "active" => Ok(LifecycleState::Active),
        "draft" => Ok(LifecycleState::Draft),
        "deprecated" => Ok(LifecycleState::Deprecated),
        "superseded" => Ok(LifecycleState::Superseded),
        "historical" => Ok(LifecycleState::Historical),
        _ => bail!(
            "Unknown knowledge state '{state}'; expected active, draft, deprecated, superseded, or historical"
        ),
    }
}

pub(super) fn resolve_revision(project_root: &Path, revision: &str) -> Result<String> {
    let revision = revision.trim();
    if revision.is_empty() || revision.starts_with('-') {
        bail!("Invalid git revision '{revision}'");
    }
    let commit = format!("{revision}^{{commit}}");
    let output = Command::new("git")
        .current_dir(project_root)
        .args(["rev-parse", "--verify", "--end-of-options", &commit])
        .output()
        .context("Failed to run git rev-parse")?;
    if !output.status.success() {
        bail!("Git revision '{revision}' does not resolve to a commit");
    }
    let resolved = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if resolved.is_empty() {
        bail!("Git revision '{revision}' resolved to an empty value");
    }
    Ok(resolved)
}

fn validate_annotation(annotation: &Annotation) -> Result<()> {
    for (kind, values) in [
        ("source", annotation.sources.as_slice()),
        ("alias", annotation.aliases.as_slice()),
    ] {
        if values
            .iter()
            .any(|value| value.trim().is_empty() || value.contains(['\n', '\r']))
        {
            bail!("--{kind} values must be non-empty single lines");
        }
    }
    if let Some(blurb) = &annotation.blurb {
        if blurb.contains(['\n', '\r']) {
            bail!("--blurb must be a single line");
        }
        if blurb.chars().count() > MAX_BLURB_CHARS {
            bail!("--blurb must be at most {MAX_BLURB_CHARS} characters");
        }
    }
    Ok(())
}

fn extend_unique(existing: &mut Vec<String>, additions: &[String]) {
    for value in additions {
        if !existing.contains(value) {
            existing.push(value.clone());
        }
    }
}

fn update_blurb(path: &Path, blurb: &str) -> Result<()> {
    crate::fs::locking::locked_update(path, |content| rewrite_blurb(&content, blurb))
        .with_context(|| format!("Failed to update blurb in {}", path.display()))
}

fn rewrite_blurb(content: &str, blurb: &str) -> Result<String> {
    let (_, body) = frontmatter::split_frontmatter(content);
    let prefix_end = content.len() - body.len();
    let rewritten = rewrite_blurb_body(body, blurb)?;
    Ok(format!("{}{rewritten}", &content[..prefix_end]))
}

fn rewrite_blurb_body(body: &str, blurb: &str) -> Result<String> {
    let (_, title_end) = first_line_with_prefix(body, "# ")
        .context("Knowledge target has no '# ' title for --blurb")?;
    let header_end = first_line_with_prefix(&body[title_end..], "## ")
        .map_or(body.len(), |(start, _)| title_end + start);
    if let Some((start, end)) = first_line_with_prefix(&body[title_end..header_end], "> ") {
        let start = title_end + start;
        let end = title_end + end;
        let newline = body[start..end].ends_with('\n');
        return Ok(format!(
            "{}> {blurb}{}{}",
            &body[..start],
            if newline { "\n" } else { "" },
            &body[end..]
        ));
    }
    let separator = if title_end == body.len() { "\n" } else { "" };
    Ok(format!(
        "{}{}> {blurb}\n{}",
        &body[..title_end],
        separator,
        &body[title_end..]
    ))
}

fn first_line_with_prefix(content: &str, prefix: &str) -> Option<(usize, usize)> {
    let mut offset = 0;
    for line in content.split_inclusive('\n') {
        let end = offset + line.len();
        if line.trim_end_matches(['\n', '\r']).starts_with(prefix) {
            return Some((offset, end));
        }
        offset = end;
    }
    None
}
