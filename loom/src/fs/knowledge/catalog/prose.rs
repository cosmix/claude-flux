//! Index project prose (`doc/**/*.md` by default) into the same structural
//! catalog the curated knowledge tree feeds.
//!
//! Today, a file with no tree-sitter grammar produces only a bare file node
//! (`crate::context::extract::lexical`) and the source ranker drops file
//! nodes outright, so design docs, proposals and plans under `doc/` are
//! unreachable by retrieval no matter how relevant their content. This module
//! closes that gap the same way the knowledge tree already is reachable:
//! chunk prose files with the existing heading chunker
//! ([`crate::fs::knowledge::chunker::chunk_file`]) and fold the result into
//! [`crate::fs::knowledge::catalog::Catalog::chunks`].
//!
//! ## Why the `prose:` prefix
//!
//! Every chunk id this module produces is prefixed [`PROSE_ID_PREFIX`]. Two
//! independent reasons make that non-negotiable:
//!
//! - **Collision-proofing.** A curated chunk id is derived from a path
//!   relative to the *knowledge root* (`architecture.md#topic#0`); a prose
//!   chunk id is derived from a path relative to the *project root*
//!   (`doc/design.md#topic#0`). Two different roots can produce the same
//!   relative path for unrelated files, and a curated author cannot see, let
//!   alone avoid, what an arbitrary project doc happens to be named. The
//!   prefix makes the two id spaces disjoint by construction instead of by
//!   convention.
//! - **Channel-of-origin signal.** The Knowledge Brief a stage or prompt
//!   receives renders chunk ids as-is (see `pack.rs`'s item rendering). A
//!   reader who sees `prose:doc/design.md#...` knows immediately this is an
//!   indexed project doc, not curated, reviewed knowledge — the same
//!   distinction [`crate::context::schema::LifecycleState`] draws for staleness,
//!   drawn here for provenance instead.
//!
//! ## Why resolution is derived here, not threaded as a parameter
//!
//! `catalog::build(root)`, `ingest(knowledge_root)`,
//! `fingerprint_tree(knowledge_root)`, `evaluate(store, knowledge_root)` and
//! `refresh(store, knowledge_root, bool)` all take only the knowledge root.
//! `loom/src/commands/knowledge/sync.rs:26` calls `refresh(&store,
//! &knowledge_root, structural_only)` and is not part of this change, so no
//! signature in that chain may grow a `project_root` or `prose_roots`
//! parameter without touching a call site outside this module's ownership.
//! [`sources_for_knowledge_root`] instead re-derives everything a prose scan
//! needs — project root, config, which roots exist — from the one value
//! every caller already has. Both `catalog::build` (structural chunks) and
//! `crate::context::fingerprint::fingerprint_tree` (structural revision) call
//! it, so the two can never disagree about what is indexed: there is exactly
//! one function that decides "what counts as prose for this knowledge root",
//! and both sides of the freshness contract call it.

use crate::context::config::RetrievalConfig;
use crate::context::schema::LifecycleState;
use crate::context::source_graph::MAX_EXTRACTED_FILE_BYTES;
use crate::fs::knowledge::chunker::{chunk_file, KnowledgeChunk};
use crate::fs::work_dir::WorkDir;
use std::fs;
use std::path::{Path, PathBuf};

/// Prefix on every prose chunk id, and on every prose entry in
/// `tree_revision`'s fingerprint list. See the module doc for why this must
/// exist and why it must be checked at both sites.
pub(crate) const PROSE_ID_PREFIX: &str = "prose:";

/// Where prose lives for one project, resolved once so `catalog::build` and
/// `fingerprint_tree` can never disagree about what is indexed.
///
/// Built only by [`sources_for_knowledge_root`] — there is no public
/// constructor, because every field here is meaningless unless it was
/// derived consistently from the same `knowledge_root`.
pub(crate) struct ProseSources {
    /// Project root the relative paths in [`ProseSources::files`] are
    /// resolved against.
    project_root: PathBuf,
    /// The knowledge root this instance was resolved for. Any directory
    /// equal to this is skipped entirely during the walk — see
    /// [`ProseSources::files`].
    knowledge_root: PathBuf,
    /// Configured prose roots, already resolved to existing directories
    /// under `project_root`.
    roots: Vec<PathBuf>,
}

/// The project root a `doc/loom/knowledge` path belongs to. Moved here from
/// `catalog.rs` (previously a private `fn project_root`) so the catalog
/// build and the tree fingerprint derive it identically, through this one
/// function, rather than keeping two copies of the same three-parent walk.
pub(crate) fn project_root_of(knowledge_root: &Path) -> Option<PathBuf> {
    let loom = knowledge_root.parent()?;
    let doc = loom.parent()?;
    let project = doc.parent()?;
    (knowledge_root.file_name()? == "knowledge"
        && loom.file_name()? == "loom"
        && doc.file_name()? == "doc")
        .then(|| project.to_path_buf())
}

/// Resolve the prose sources for one knowledge root.
///
/// Returns `None` when no project root can be derived from `knowledge_root`
/// (see [`project_root_of`]), or when every configured `prose_roots` entry
/// is absent from disk — including the documented opt-out `prose_roots =
/// []`, which must cost nothing downstream: no `ProseSources`, no walk, no
/// fingerprints.
pub(crate) fn sources_for_knowledge_root(knowledge_root: &Path) -> Option<ProseSources> {
    let project_root = project_root_of(knowledge_root)?;
    let config = RetrievalConfig::load(&config_root(&project_root));
    // `RetrievalConfig` already rejects absolute and `..` entries when it
    // parses `prose_roots` (`context/config.rs:278-300`); the only check left
    // to do here is "does this configured root actually exist as a
    // directory right now".
    let roots: Vec<PathBuf> = config
        .prose_roots
        .iter()
        .map(|relative| project_root.join(relative))
        .filter(|absolute| fs::metadata(absolute).is_ok_and(|metadata| metadata.is_dir()))
        .collect();
    if roots.is_empty() {
        return None;
    }
    Some(ProseSources {
        project_root,
        knowledge_root: knowledge_root.to_path_buf(),
        roots,
    })
}

/// The root whose `.loom/config.toml` governs prose indexing: the MAIN
/// project root when this checkout is a linked worktree (its `.loom/work` is
/// a symlink into the main repository), otherwise the checkout itself.
///
/// Without this, a stage running in `.worktrees/<id>/` would silently fall
/// back to DEFAULT prose roots while the host ran on the operator's — and
/// since both share one context store, the tree revision would flap between
/// them and every retrieval on either side would rebuild the catalog the
/// other just built.
///
/// Deliberately not `WorkDir::new(project_root)` directly, which searches
/// UPWARD from `project_root` and could bind a temp-dir fixture to an
/// unrelated enclosing project. Instead, only a state-dir path that IS a
/// symlink right at `project_root` (either spelling; nested checked first)
/// is handed to `WorkDir::new`, so the only upward search that can run
/// starts from the symlink's already-resolved target — the hop count for
/// either layout stays [`WorkDir::project_root`]'s call, never recomputed
/// here.
fn config_root(project_root: &Path) -> PathBuf {
    for candidate in [
        project_root.join(".loom").join("work"),
        project_root.join(".work"),
    ] {
        if candidate.is_symlink() {
            if let Some(root) = WorkDir::new(&candidate)
                .ok()
                .and_then(|wd| wd.project_root().map(Path::to_path_buf))
            {
                return root;
            }
        }
    }
    project_root.to_path_buf()
}

impl ProseSources {
    /// Project root the relative paths in [`ProseSources::files`] are
    /// resolved against.
    pub(crate) fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// Indexable prose files, PROJECT-relative, `/`-separated, sorted and
    /// deduped — configured roots may nest (e.g. `["doc", "doc/design"]`),
    /// so the same file can otherwise be walked and returned twice.
    pub(crate) fn files(&self) -> Vec<PathBuf> {
        let mut collected = Vec::new();
        for root in &self.roots {
            collect_prose_files(
                &self.project_root,
                root,
                &self.knowledge_root,
                &mut collected,
            );
        }
        collected.retain(|relative| prose_lifecycle(relative, None).is_some());
        collected.sort();
        collected.dedup();
        collected
    }

    /// Chunks for every file in [`ProseSources::files`], ids already
    /// prefixed with [`PROSE_ID_PREFIX`] and lifecycle derived from its path
    /// plus an explicit frontmatter `state:` override.
    ///
    /// The rest of the id shape is identical to a curated chunk's
    /// (`prose:doc/design.md#appendix-a#0`) so nothing downstream has to
    /// special-case it: `--require-id` accepts it as-is, and
    /// `split_once('#')` on it still yields the same `(file#anchor,
    /// occurrence)` split a curated id yields.
    ///
    /// `chunk.file` ends up PROJECT-relative (`doc/design.md`) here, where a
    /// curated chunk's `file` is knowledge-root-relative (`architecture.md`).
    /// That asymmetry is intentional — it is what makes `pack.rs`'s item-path
    /// rendering show the right path for each origin — but it is a real trap
    /// for a later reader who assumes every `KnowledgeChunk::file` resolves
    /// against the same root.
    pub(crate) fn chunks(&self) -> Vec<KnowledgeChunk> {
        let mut chunks = Vec::new();
        for relative in self.files() {
            let absolute = self.project_root.join(&relative);
            let bytes = match fs::read(&absolute) {
                Ok(bytes) => bytes,
                Err(error) => {
                    tracing::debug!(path = %absolute.display(), %error, "skipping unreadable prose file");
                    continue;
                }
            };
            let explicit = crate::fs::knowledge::frontmatter::file_state(&bytes);
            let Some(lifecycle) = prose_lifecycle(&relative, explicit) else {
                continue;
            };
            let file_chunks = match chunk_file(&relative, &bytes) {
                Ok(file_chunks) => file_chunks,
                Err(error) => {
                    tracing::debug!(path = %relative.display(), %error, "skipping prose file that failed to chunk");
                    continue;
                }
            };
            for mut chunk in file_chunks {
                chunk.id = format!("{PROSE_ID_PREFIX}{}", chunk.id);
                chunk.state = lifecycle;
                chunks.push(chunk);
            }
        }
        chunks
    }
}

/// Recursively collect indexable prose `*.md` files under `dir` into `out`,
/// each already made PROJECT-relative with `/` separators.
///
/// Every I/O failure on any entry is a skip (`tracing::debug!`), never a
/// propagated error: prose indexing is a best-effort enhancement layered
/// onto a catalog build and a tree fingerprint that both worked before prose
/// existed, and neither may be made able to fail by it.
fn collect_prose_files(
    project_root: &Path,
    dir: &Path,
    knowledge_root: &Path,
    out: &mut Vec<PathBuf>,
) {
    // The curated tree lives under a configured prose root by default
    // (`prose_roots = ["doc"]`, knowledge at `doc/loom/knowledge`), so
    // without this exact-match skip every curated chunk would be indexed a
    // second time as prose: duplicate bodies in the pack, a doubled corpus,
    // and a curated/prose pair competing for the same slot.
    if dir == knowledge_root {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::debug!(dir = %dir.display(), %error, "skipping unreadable prose directory");
            return;
        }
    };
    for entry in entries {
        match entry {
            Ok(entry) => visit_prose_entry(project_root, &entry, knowledge_root, out),
            Err(error) => {
                tracing::debug!(dir = %dir.display(), %error, "skipping unreadable prose directory entry");
            }
        }
    }
}

/// Handle one directory entry: recurse into a subdirectory, or filter a file
/// through [`push_prose_file`]. Dotfiles and dot-directories are skipped
/// outright, matching the curated-tree walk's convention
/// (`catalog.rs`'s `collect_markdown_files`).
///
/// `file_type()` does NOT follow symlinks (unlike `fs::metadata`), so a
/// symlinked directory reports neither `is_dir()` nor `is_file()` here and
/// falls through both arms below — this is what stops a symlink under a
/// prose root from aiming the indexer outside the project.
fn visit_prose_entry(
    project_root: &Path,
    entry: &fs::DirEntry,
    knowledge_root: &Path,
    out: &mut Vec<PathBuf>,
) {
    let name = entry.file_name();
    let name = name.to_string_lossy();
    if name.starts_with('.') {
        return;
    }
    let path = entry.path();
    let file_type = match entry.file_type() {
        Ok(file_type) => file_type,
        Err(error) => {
            tracing::debug!(path = %path.display(), %error, "skipping prose entry of unknown type");
            return;
        }
    };
    if file_type.is_dir() {
        collect_prose_files(project_root, &path, knowledge_root, out);
    } else if file_type.is_file() {
        push_prose_file(project_root, &path, &name, entry, out);
    }
}

/// Apply the file-level filters (`*.md`, size cap, lifecycle path exclusions)
/// to one candidate file and push its project-relative path onto `out` when
/// it survives all of them.
fn push_prose_file(
    project_root: &Path,
    path: &Path,
    name: &str,
    entry: &fs::DirEntry,
    out: &mut Vec<PathBuf>,
) {
    if !name.ends_with(".md") {
        return;
    }
    let metadata = match entry.metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            tracing::debug!(path = %path.display(), %error, "skipping prose file with unreadable metadata");
            return;
        }
    };
    if metadata.len() as usize > MAX_EXTRACTED_FILE_BYTES {
        return;
    }
    let Ok(relative) = path.strip_prefix(project_root) else {
        return;
    };
    if prose_lifecycle(relative, None).is_none() {
        return;
    }
    out.push(normalize_relative(relative));
}

/// Derive prose authority from a PROJECT-relative path. `None` means the file
/// is excluded from both chunking and fingerprinting.
pub(crate) fn prose_lifecycle(
    relative: &Path,
    explicit: Option<LifecycleState>,
) -> Option<LifecycleState> {
    let components: Vec<String> = relative
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_string_lossy()
                .trim()
                .to_lowercase()
        })
        .collect();
    let file_name = components.last().map(String::as_str).unwrap_or_default();
    let under_plans = components.iter().any(|component| component == "plans");

    if components.iter().any(|component| component == "archive") {
        return None;
    }
    if under_plans && file_name.starts_with("done-") {
        return None;
    }
    if let Some(explicit) = explicit {
        return Some(explicit);
    }
    if under_plans && file_name.starts_with("review-") {
        return Some(LifecycleState::Historical);
    }
    if under_plans && (file_name.starts_with("plan-") || file_name.starts_with("in_progress-plan-"))
    {
        return Some(LifecycleState::Draft);
    }
    if under_plans && components.iter().any(|component| component == "briefs") {
        return Some(LifecycleState::Draft);
    }
    if file_name.starts_with("report-") || file_name.starts_with("proposal-") {
        return Some(LifecycleState::Historical);
    }
    Some(LifecycleState::Active)
}

/// Rejoin `relative`'s components with `/`, matching the normalization idiom
/// at `crate::context::fingerprint::collect_markdown_files` — so
/// `KnowledgeChunk::file` never carries a platform-specific separator
/// regardless of which OS built the catalog.
fn normalize_relative(relative: &Path) -> PathBuf {
    relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
        .into()
}
