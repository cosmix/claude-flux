//! Build a deterministic catalog over the whole knowledge tree.

use crate::fs::knowledge::chunker::{self, KnowledgeChunk};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

mod evidence;
mod issue;
mod order;
pub(crate) mod prose;
pub(crate) mod size;
mod source_roots;
#[cfg(test)]
mod tests_prose;

pub use issue::CatalogIssue;
use order::compare_issues;
use source_roots::{cargo_package_source_roots, ProjectFileIndex, SourceRefContext};

/// Deterministic retrieval data and non-mutating knowledge-base diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    /// Hash over the catalog's chunk identities and content hashes.
    pub revision: String,
    /// Curated knowledge chunks in relative-path order, followed by every
    /// indexed prose chunk (`prose:`-prefixed id — see `prose`) in the
    /// order `prose::ProseSources::files` returns. Not one global sort: the
    /// two groups are ordering-independent by construction, since ranking
    /// scores every chunk on its own and never assumes catalog order.
    pub chunks: Vec<KnowledgeChunk>,
    /// Diagnostics, sorted deterministically.
    pub issues: Vec<CatalogIssue>,
}

/// Update heading occurrence counts and collect any broken-link or
/// missing-source-ref issues for one chunk of an already-read file.
fn collect_chunk_issues(
    root: &Path,
    relative_path: &Path,
    source_refs: &SourceRefContext,
    chunk: &KnowledgeChunk,
    heading_counts: &mut BTreeMap<PathBuf, BTreeMap<String, usize>>,
    issues: &mut Vec<CatalogIssue>,
) -> anyhow::Result<()> {
    if !chunk.heading.is_empty() {
        *heading_counts
            .entry(relative_path.to_path_buf())
            .or_default()
            .entry(chunk.anchor.clone())
            .or_default() += 1;
    }
    if let Some(issue) = size::oversized_section(relative_path, chunk) {
        issues.push(issue);
    }
    for (_, target) in &chunk.links {
        let exists = contained_link_target(root, relative_path, target)
            .is_some_and(|target_path| path_exists(&target_path));
        if !exists {
            issues.push(CatalogIssue::BrokenLink {
                file: relative_path.to_path_buf(),
                target: target.clone(),
            });
        }
    }
    if let Some(project_root) = source_refs.project_root {
        for source_path in &chunk.source_paths {
            source_roots::push_source_ref_issue(
                project_root,
                source_refs,
                relative_path,
                source_path,
                issues,
            );
        }
    }
    Ok(())
}

/// Chunk one knowledge file and collect any generic-blurb, broken-link, or
/// missing-source-ref issues it produces into `issues`. Heading occurrence
/// counts feed `heading_counts`, which the caller uses for a separate
/// duplicate-heading pass once every file has been processed.
fn process_file(
    root: &Path,
    relative_path: &Path,
    source_refs: &SourceRefContext,
    heading_counts: &mut BTreeMap<PathBuf, BTreeMap<String, usize>>,
    issues: &mut Vec<CatalogIssue>,
) -> anyhow::Result<Vec<KnowledgeChunk>> {
    let absolute_path = root.join(relative_path);
    let bytes = fs::read(&absolute_path)
        .with_context(|| format!("Failed to read knowledge file: {}", absolute_path.display()))?;
    let content = String::from_utf8_lossy(&bytes);
    let (frontmatter, body_content) =
        crate::fs::knowledge::frontmatter::split_frontmatter(&content);
    let file_chunks = chunker::chunk_sections(relative_path, body_content, &frontmatter);

    if let Some(issue) = size::oversized_file(relative_path, &content) {
        issues.push(issue);
    }

    if let Some(blurb) = generic_blurb(&content, relative_path) {
        issues.push(CatalogIssue::GenericBlurb {
            file: relative_path.to_path_buf(),
            blurb,
        });
    }

    for chunk in &file_chunks {
        collect_chunk_issues(
            root,
            relative_path,
            source_refs,
            chunk,
            heading_counts,
            issues,
        )?;
    }
    collect_unverifiable_references(relative_path, source_refs, &file_chunks, issues);
    issues.extend(evidence::changed_since_verified(
        source_refs.project_root,
        relative_path,
        &frontmatter,
    ));

    Ok(file_chunks)
}

fn collect_unverifiable_references(
    relative_path: &Path,
    source_refs: &SourceRefContext,
    chunks: &[KnowledgeChunk],
    issues: &mut Vec<CatalogIssue>,
) {
    let Some(project_root) = source_refs.project_root else {
        return;
    };
    let references: std::collections::BTreeSet<_> = chunks
        .iter()
        .flat_map(|chunk| crate::fs::knowledge::chunker::references::references_in(&chunk.body).0)
        .filter(|reference| {
            reference.kind != crate::fs::knowledge::chunker::references::EvidenceKind::Live
        })
        .collect();
    for reference in references {
        let resolves = source_roots::looks_like_repository_path(&reference.source_path)
            && source_refs.path_exists(project_root, &reference.source_path);
        if !resolves {
            issues.push(CatalogIssue::UnverifiableReference {
                file: relative_path.to_path_buf(),
                source_path: reference.source_path,
                kind: reference.kind.as_str().to_string(),
            });
        }
    }
}

/// Turn the per-file heading tallies [`process_file`] accumulated into one
/// [`CatalogIssue::DuplicateHeading`] per heading seen more than once.
///
/// A separate pass, not part of `process_file`: a duplicate is only knowable
/// once every section of a file has been counted.
fn push_duplicate_headings(
    heading_counts: BTreeMap<PathBuf, BTreeMap<String, usize>>,
    issues: &mut Vec<CatalogIssue>,
) {
    for (file, counts) in heading_counts {
        for (heading, occurrences) in counts {
            if occurrences > 1 {
                issues.push(CatalogIssue::DuplicateHeading {
                    file: file.clone(),
                    heading,
                    occurrences,
                });
            }
        }
    }
}

/// Build a deterministic catalog rooted at a knowledge directory, extended
/// with every chunk indexed from the project's configured prose roots (see
/// `prose`).
pub fn build(root: &Path) -> anyhow::Result<Catalog> {
    let files = markdown_files(root)?;
    let project_root = prose::project_root_of(root);
    let cargo_source_roots = project_root
        .as_deref()
        .map(cargo_package_source_roots)
        .unwrap_or_default();
    let project_files = ProjectFileIndex::new(project_root.clone());
    let source_refs =
        SourceRefContext::new(project_root.as_deref(), &cargo_source_roots, &project_files);
    let mut chunks = Vec::new();
    let mut issues = Vec::new();
    let mut heading_counts: BTreeMap<PathBuf, BTreeMap<String, usize>> = BTreeMap::new();

    for relative_path in files {
        let file_chunks = process_file(
            root,
            &relative_path,
            &source_refs,
            &mut heading_counts,
            &mut issues,
        )?;
        chunks.extend(file_chunks);
    }

    push_duplicate_headings(heading_counts, &mut issues);

    if let Some(issue) = size::oversized_index(root) {
        issues.push(issue);
    }

    issues.sort_by(compare_issues);

    // Prose is appended to the SAME chunk list the curated tree produced, so
    // one BM25 corpus covers both and `revision_for` below folds prose edits
    // into the catalog's own revision. Prose contributes no `issues`: the
    // duplicate-heading, generic-blurb, broken-link and missing-source-ref
    // diagnostics are contracts on the CURATED tree, and reporting them for
    // arbitrary project docs would bury the ones an author can act on.
    if let Some(sources) = prose::sources_for_knowledge_root(root) {
        chunks.extend(sources.chunks());
    }

    Ok(Catalog {
        revision: revision_for(&chunks),
        chunks,
        issues,
    })
}

fn markdown_files(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    match fs::metadata(root) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Ok(Vec::new()),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to inspect knowledge root: {}", root.display()))
        }
    }

    let mut files = Vec::new();
    collect_markdown_files(root, root, &mut files)?;
    files.sort_by_key(|path| display_path(path));
    Ok(files)
}

fn collect_markdown_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    let entries = fs::read_dir(directory).with_context(|| {
        format!(
            "Failed to read knowledge directory: {}",
            directory.display()
        )
    })?;
    for entry in entries {
        let entry =
            entry.with_context(|| format!("Failed to read entry in {}", directory.display()))?;
        let path = entry.path();
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if file_name.starts_with('.') {
            continue;
        }
        let file_type = entry
            .file_type()
            .with_context(|| format!("Failed to inspect knowledge entry: {}", path.display()))?;
        if file_type.is_dir() {
            collect_markdown_files(root, &path, files)?;
        } else if file_type.is_file() && file_name.ends_with(".md") && file_name != "INDEX.md" {
            let relative = path.strip_prefix(root).with_context(|| {
                format!("Failed to relativize knowledge path: {}", path.display())
            })?;
            files.push(relative.to_path_buf());
        }
    }
    Ok(())
}

fn revision_for(chunks: &[KnowledgeChunk]) -> String {
    let mut lines: Vec<_> = chunks
        .iter()
        .map(|chunk| format!("{}:{}\n", chunk.id, chunk.content_hash))
        .collect();
    lines.sort();
    hex::encode(Sha256::digest(lines.concat().as_bytes()))
}

fn generic_blurb(content: &str, relative_path: &Path) -> Option<String> {
    let category = relative_path.parent()?.file_name()?.to_str()?;
    let blurb = content
        .lines()
        .find_map(|line| line.strip_prefix("> "))?
        .trim()
        .to_string();
    let scaffold = crate::fs::knowledge::templates::scaffold_blurb(category);
    (blurb == scaffold).then_some(blurb)
}

/// True if `path` exists on disk. Every `fs::metadata` failure — not only
/// `NotFound` — is treated the same as "does not exist": the result feeds a
/// diagnostic ("is this link or source reference broken?"), never a hard
/// error, so a permission-denied or other transient failure on ONE path in
/// ONE knowledge file must not become a fatal `Err` that takes down
/// `catalog::build` and, through it, every stage's Knowledge Brief
/// (`ingest` -> `retrieve_for_stage` -> `orchestrator::signals::retrieval`).
fn path_exists(path: &Path) -> bool {
    fs::metadata(path).is_ok()
}

/// Resolve a markdown link `target` from the knowledge file at
/// `relative_path` to an absolute path — but only when it stays inside
/// `root`. Returns `None` for a target that must not be probed on disk at
/// all: an absolute target, or one that, once `.`/`..` are folded away
/// lexically, would land outside the knowledge tree (e.g.
/// `../../../etc/passwd`).
///
/// `..` is otherwise legitimate here — a tier-2 file at
/// `architecture/topic.md` routinely links `../concerns.md` up to a tier-1
/// file, and that must keep resolving. Resolution is purely lexical
/// (component-by-component `.`/`..` folding), never `Path::canonicalize`:
/// the target may legitimately not exist yet, which is exactly the
/// question [`path_exists`] is being asked to answer.
fn contained_link_target(root: &Path, relative_path: &Path, target: &str) -> Option<PathBuf> {
    if Path::new(target).is_absolute() {
        return None;
    }

    let start = relative_path.parent().unwrap_or(Path::new(""));
    let mut normalized = PathBuf::new();
    for component in start.components().chain(Path::new(target).components()) {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !normalized.pop() {
                    // Popped past the knowledge root itself: the target
                    // escapes the tree.
                    return None;
                }
            }
            std::path::Component::Normal(part) => normalized.push(part),
            // `target` is already confirmed relative above, and `start` is
            // always relative (it comes from a knowledge-relative file
            // path), so neither a root nor a Windows prefix component can
            // occur here.
            std::path::Component::RootDir | std::path::Component::Prefix(_) => return None,
        }
    }

    Some(root.join(normalized))
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
