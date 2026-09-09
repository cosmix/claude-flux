use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

use super::{markers, ProjectProfile, ProjectType};

const MAX_DEPTH: usize = 8;
const MAX_ENTRIES: usize = 20_000;
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".loom",
    ".work",
    ".worktrees",
    "node_modules",
    "target",
    "vendor",
    "dist",
    "build",
    ".next",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
    ".codex",
    ".claude",
    ".agents",
    ".terraform",
];

/// This marker is only a traversal boundary; it grants no repository authority.
pub(super) fn checkout_root(cwd: &Path) -> PathBuf {
    for ancestor in cwd.ancestors() {
        if ancestor.join(".git").symlink_metadata().is_ok() {
            return ancestor.to_path_buf();
        }
    }
    cwd.ancestors()
        .find(|path| markers::package_boundary(path))
        .unwrap_or(cwd)
        .to_path_buf()
}

pub(super) fn discover(root: PathBuf) -> ProjectProfile {
    let mut profile = ProjectProfile {
        root,
        ..ProjectProfile::default()
    };
    let mut pending = VecDeque::from([(profile.root.clone(), 0)]);
    let mut remaining = MAX_ENTRIES;
    while let Some((dir, depth)) = pending.pop_front() {
        let types = markers::detect(&dir);
        let path = dir.strip_prefix(&profile.root).unwrap_or(Path::new(""));
        if !types.is_empty() || markers::package_boundary(&dir) {
            profile.packages.push(path.to_path_buf());
        }
        for kind in types {
            profile.types.push(ProjectType {
                kind,
                path: path.to_path_buf(),
            });
        }
        let (children, exhausted) = children(&dir, &mut remaining);
        if exhausted {
            profile.truncated = true;
            break;
        }
        if depth == MAX_DEPTH {
            profile.truncated |= !children.is_empty();
        } else {
            pending.extend(children.into_iter().map(|child| (child, depth + 1)));
        }
    }
    profile.types.sort();
    profile
}

fn children(dir: &Path, remaining: &mut usize) -> (Vec<PathBuf>, bool) {
    let Ok(entries) = fs::read_dir(dir) else {
        return (Vec::new(), false);
    };
    let mut children = Vec::new();
    for entry in entries {
        if *remaining == 0 {
            return (children, true);
        }
        *remaining -= 1;
        let Ok(entry) = entry else { continue };
        let name = entry.file_name();
        if SKIP_DIRS.contains(&name.to_string_lossy().as_ref()) {
            continue;
        }
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            // Nested checkouts belong to a different project.
            if !entry.path().join(".git").exists() {
                children.push(entry.path());
            }
        }
    }
    children.sort();
    (children, false)
}
