//! Cargo-package source-root discovery for catalog source references.

use super::CatalogIssue;
use std::cell::OnceCell;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Read-only inputs needed to validate a chunk's backticked repository
/// source references against the project tree. Bundled so
/// `collect_chunk_issues` and `process_file` stay under clippy's
/// `too_many_arguments` threshold.
pub(super) struct SourceRefContext<'a> {
    pub(super) project_root: Option<&'a Path>,
    pub(super) cargo_source_roots: &'a [PathBuf],
    pub(super) project_files: &'a ProjectFileIndex,
}

impl<'a> SourceRefContext<'a> {
    pub(super) fn new(
        project_root: Option<&'a Path>,
        cargo_source_roots: &'a [PathBuf],
        project_files: &'a ProjectFileIndex,
    ) -> Self {
        Self {
            project_root,
            cargo_source_roots,
            project_files,
        }
    }

    /// [`repository_source_path_exists`] against this context's cargo source
    /// roots and project file index, for the given `project_root`.
    pub(super) fn path_exists(&self, project_root: &Path, source_path: &str) -> bool {
        repository_source_path_exists(
            project_root,
            self.cargo_source_roots,
            self.project_files,
            source_path,
        )
    }

    /// True when `source_path`'s first path component names something that
    /// exists at the project root or under a declared cargo source root —
    /// even though `source_path` in full did not resolve.
    ///
    /// This is what separates a reference into a part of THIS project (a
    /// real file under a known top-level directory, just misspelled or
    /// deleted — still a [`super::CatalogIssue::MissingSourceRef`]) from a
    /// reference into a project this repository does not contain at all (an
    /// external reference the chunker cannot see, since it never touches the
    /// filesystem).
    pub(super) fn first_component_resolves(&self, project_root: &Path, source_path: &str) -> bool {
        let first_component = source_path.split('/').next().unwrap_or(source_path);
        fs::metadata(project_root.join(first_component)).is_ok()
            || self
                .cargo_source_roots
                .iter()
                .any(|source_root| fs::metadata(source_root.join(first_component)).is_ok())
    }
}

/// Whether a backticked source reference names an existing project file.
///
/// The canonical spelling is project-relative (`crates/core/src/models/constants.rs`),
/// but knowledge prose can also use a module-relative Rust path
/// (`models/constants.rs`), a bare basename (`constants.rs`), or a path
/// rooted somewhere other than the project or a cargo package (a shell
/// script under `hooks/`, a fixture nested inside `tests/`). The first two
/// forms resolve exactly, through `project_root` or a source root declared
/// by an actual Cargo package (see [`cargo_package_source_roots`]). The rest
/// fall through to `project_files`, which knows every file in the project.
///
/// A module-relative path must match **exactly one** declared package source
/// root; a suffix path must match **exactly one** project file. Multiple
/// matches are ambiguous and remain a `MissingSourceRef`, rather than
/// silently choosing a candidate by traversal order.
///
/// Callers have already rejected absolute and parent-relative paths, so no
/// candidate can escape its root.
pub(super) fn repository_source_path_exists(
    project_root: &Path,
    cargo_source_roots: &[PathBuf],
    project_files: &ProjectFileIndex,
    source_path: &str,
) -> bool {
    let resolves_exactly = fs::metadata(project_root.join(source_path)).is_ok()
        || cargo_source_roots
            .iter()
            .filter(|source_root| fs::metadata(source_root.join(source_path)).is_ok())
            .take(2)
            .count()
            == 1;
    if resolves_exactly {
        return true;
    }
    if source_path.contains('/') {
        project_files.has_unique_suffix(source_path)
    } else {
        project_files.has_basename(source_path)
    }
}

/// A lazily-built, cached list of every file under a project root, used to
/// resolve source references that name a bare basename or a path suffix
/// rather than a full project-relative or cargo-module-relative path.
///
/// Built at most once: the walk only runs on the first reference that needs
/// it (see [`repository_source_path_exists`]), and a knowledge tree with no
/// backticked source references never triggers it at all.
pub(super) struct ProjectFileIndex {
    project_root: Option<PathBuf>,
    files: OnceCell<Vec<String>>,
}

impl ProjectFileIndex {
    /// `None` when the knowledge tree being cataloged has no known project
    /// root (see `prose::project_root_of`): the index then never has
    /// anything to walk, and `files()` reports an empty list without ever
    /// touching disk.
    pub(super) fn new(project_root: Option<PathBuf>) -> Self {
        Self {
            project_root,
            files: OnceCell::new(),
        }
    }

    fn files(&self) -> &[String] {
        self.files.get_or_init(|| {
            self.project_root
                .as_deref()
                .map(walk_project_files)
                .unwrap_or_default()
        })
    }

    /// True if any project file's final path segment equals `basename`.
    fn has_basename(&self, basename: &str) -> bool {
        self.files()
            .iter()
            .any(|file| file.rsplit('/').next() == Some(basename))
    }

    /// True if exactly one project file ends with `suffix` on a `/`
    /// boundary (or equals it outright). Zero or multiple matches are not
    /// good enough: an ambiguous or absent reference stays reported.
    fn has_unique_suffix(&self, suffix: &str) -> bool {
        self.files()
            .iter()
            .filter(|file| path_has_suffix(file, suffix))
            .take(2)
            .count()
            == 1
    }
}

fn path_has_suffix(file: &str, suffix: &str) -> bool {
    file == suffix
        || file
            .strip_suffix(suffix)
            .is_some_and(|prefix| prefix.ends_with('/'))
}

/// Directories whose contents never hold a reference worth resolving:
/// version control metadata, build output, dependency caches, and loom's own
/// worktree/state directories. Also skips any directory starting with `.`.
fn should_skip_dir(name: &str) -> bool {
    name.starts_with('.') || matches!(name, "target" | "node_modules")
}

/// Recursively collect every file under `project_root`, as relative,
/// forward-slashed paths. Unreadable directories are skipped rather than
/// failing the whole walk: a permission-denied subdirectory must not turn a
/// diagnostic pass into a hard error (see [`super::path_exists`]'s rationale).
fn walk_project_files(project_root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    walk_dir(project_root, project_root, &mut files);
    files
}

fn walk_dir(root: &Path, directory: &Path, files: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            if !should_skip_dir(&name) {
                walk_dir(root, &path, files);
            }
        } else if file_type.is_file() {
            if let Ok(relative) = path.strip_prefix(root) {
                files.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

/// Return the `src` directories of Cargo packages that can be established
/// from the repository root without recursively searching arbitrary paths.
///
/// Explicit workspace members cover conventional multi-crate repositories;
/// the direct-child scan supports repositories that use a single nested crate
/// but have no workspace manifest (including this repository). A directory is
/// accepted only when its own `Cargo.toml` parses and contains `[package]`.
pub(super) fn cargo_package_source_roots(project_root: &Path) -> Vec<PathBuf> {
    let mut package_roots = BTreeSet::new();
    insert_cargo_package_root(project_root, &mut package_roots);

    let root_manifest = project_root.join("Cargo.toml");
    for member in cargo_workspace_members(&root_manifest) {
        insert_cargo_package_root(&project_root.join(member), &mut package_roots);
    }

    if let Ok(entries) = fs::read_dir(project_root) {
        for entry in entries.filter_map(Result::ok) {
            if entry.file_type().is_ok_and(|file_type| file_type.is_dir()) {
                insert_cargo_package_root(&entry.path(), &mut package_roots);
            }
        }
    }

    package_roots
        .into_iter()
        .map(|package_root| package_root.join("src"))
        .collect()
}

/// Add `directory` when it is a concrete Cargo package directory.
fn insert_cargo_package_root(directory: &Path, package_roots: &mut BTreeSet<PathBuf>) {
    let manifest = directory.join("Cargo.toml");
    if cargo_manifest_has_package(&manifest) {
        package_roots.insert(directory.to_path_buf());
    }
}

/// Parse the root manifest's literal workspace-member paths. Globs and paths
/// that escape the root are intentionally ignored: expanding them would make
/// the diagnostic's accepted source roots depend on an unbounded filesystem
/// walk instead of on a finite, auditable manifest declaration.
fn cargo_workspace_members(manifest: &Path) -> Vec<PathBuf> {
    let Ok(content) = fs::read_to_string(manifest) else {
        return Vec::new();
    };
    let Ok(manifest) = toml::from_str::<toml::Value>(&content) else {
        return Vec::new();
    };
    manifest
        .get("workspace")
        .and_then(toml::Value::as_table)
        .and_then(|workspace| workspace.get("members"))
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .filter_map(valid_workspace_member_path)
        .collect()
}

/// A literal workspace member path contained within its declaring project.
fn valid_workspace_member_path(member: &str) -> Option<PathBuf> {
    let path = Path::new(member);
    (!path.is_absolute()
        && !member.contains('*')
        && !member.contains('?')
        && !path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir)))
    .then(|| path.to_path_buf())
}

/// Whether `manifest` is a readable TOML Cargo manifest for one package.
fn cargo_manifest_has_package(manifest: &Path) -> bool {
    fs::read_to_string(manifest)
        .ok()
        .and_then(|content| toml::from_str::<toml::Value>(&content).ok())
        .is_some_and(|manifest| manifest.get("package").is_some_and(toml::Value::is_table))
}

/// A backticked span worth probing on disk at all: not an absolute path, not
/// a `//`-prefixed value, and not one that climbs above its root with `..`.
pub(super) fn looks_like_repository_path(source_path: &str) -> bool {
    let path = Path::new(source_path);
    !source_path.starts_with("//")
        && !path.is_absolute()
        && !path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
}

/// A `MissingSourceRef` or `UnverifiableReference` note for one `Live`
/// backticked source path that does not resolve, or nothing when it does.
///
/// A path whose first component names nothing this project has — neither at
/// the project root nor under a declared cargo source root — is reported as
/// an external note instead of a missing reference: the chunker cannot make
/// that call itself, since it never sees the filesystem (see
/// [`SourceRefContext::first_component_resolves`]).
pub(super) fn push_source_ref_issue(
    project_root: &Path,
    source_refs: &SourceRefContext,
    relative_path: &Path,
    source_path: &str,
    issues: &mut Vec<CatalogIssue>,
) {
    if !looks_like_repository_path(source_path)
        || source_refs.path_exists(project_root, source_path)
    {
        return;
    }
    // A bare basename (no `/`) has no first path component distinct from the
    // leaf itself, so "external by resolution" cannot say anything about it
    // — it always stays a plain missing reference, resolved or not against
    // `ProjectFileIndex::has_basename`.
    let external = source_path.contains('/')
        && !source_refs.first_component_resolves(project_root, source_path);
    if external {
        issues.push(CatalogIssue::UnverifiableReference {
            file: relative_path.to_path_buf(),
            source_path: source_path.to_string(),
            kind: "external".to_string(),
        });
    } else {
        issues.push(CatalogIssue::MissingSourceRef {
            file: relative_path.to_path_buf(),
            source_path: source_path.to_string(),
        });
    }
}
