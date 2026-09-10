use anyhow::Result;
use std::collections::BTreeMap;
use std::path::Path;

use super::{excluded, SourceGraphScope};
use crate::git::runner::run_git_checked;

/// Git modes of the entries that are never source files: a symlink, whose
/// working-tree path resolves to whatever it points at (possibly outside the
/// repository), and a gitlink (submodule), which is a directory.
const SYMLINK_MODE: &str = "120000";
const GITLINK_MODE: &str = "160000";

pub(crate) struct Enumeration {
    /// Path → blob object id, from `git ls-tree -r -z HEAD` (Base) or `git ls-files -s -z` (Overlay).
    /// Symlinks and gitlinks are left out.
    pub known: BTreeMap<String, String>,
    /// Existing untracked files from `git ls-files --others --exclude-standard -z` (Overlay only).
    pub untracked: Vec<String>,
    /// Known paths absent from the working tree (Overlay only): what the overlay tombstones
    /// because a tracked file was deleted or moved.
    pub deleted: Vec<String>,
    /// Paths the index tracks as a symlink or gitlink (Overlay only) - never source, so they
    /// never enter `known`. A layer only tombstones one of these when the base already holds a
    /// regular-file entry for the same path (a file that became a symlink or gitlink); one the
    /// base never tracked is simply absent from both, not counted anywhere.
    pub not_source: Vec<String>,
}

pub(crate) fn enumerate(project_root: &Path, scope: &SourceGraphScope) -> Result<Enumeration> {
    match scope {
        SourceGraphScope::Base { .. } => {
            let output = run_git_checked(&["ls-tree", "-r", "-z", "HEAD"], project_root)?;
            // A base layer is built from `known` alone, so a committed symlink
            // or gitlink simply never enters it; there is nothing to tombstone.
            let (known, _) = parse_known(&output, TreeRow::Committed);
            Ok(Enumeration {
                known,
                untracked: Vec::new(),
                deleted: Vec::new(),
                not_source: Vec::new(),
            })
        }
        SourceGraphScope::Overlay { .. } => {
            let output = run_git_checked(&["ls-files", "-s", "-z"], project_root)?;
            let (known, not_source) = parse_known(&output, TreeRow::Index);
            let output = run_git_checked(
                &["ls-files", "--others", "--exclude-standard", "-z"],
                project_root,
            )?;
            let mut untracked = nul_paths(&output)
                .filter(|path| {
                    !excluded(path)
                        && std::fs::symlink_metadata(project_root.join(path))
                            .is_ok_and(|metadata| metadata.file_type().is_file())
                })
                .map(str::to_string)
                .collect::<Vec<_>>();
            untracked.sort();
            let deleted = known
                .keys()
                .filter(|path| !project_root.join(path).exists())
                .cloned()
                .collect::<Vec<_>>();
            Ok(Enumeration {
                known,
                untracked,
                deleted,
                not_source,
            })
        }
    }
}

#[derive(Clone, Copy)]
enum TreeRow {
    Committed,
    Index,
}

/// Split `ls-tree` / `ls-files -s` records into source files (path → blob oid)
/// and the paths tracked as a symlink or gitlink, which are never read.
fn parse_known(output: &str, row: TreeRow) -> (BTreeMap<String, String>, Vec<String>) {
    let mut known = BTreeMap::new();
    let mut not_source = Vec::new();
    for (mode, oid, path) in nul_paths(output).filter_map(|record| parse_record(record, row)) {
        if excluded(path) {
            continue;
        }
        if matches!(mode, SYMLINK_MODE | GITLINK_MODE) {
            not_source.push(path.to_string());
        } else {
            known.insert(path.to_string(), oid.to_string());
        }
    }
    (known, not_source)
}

/// `(mode, oid, path)` from one `<mode> [<kind>] <oid>[ <stage>]\t<path>` record.
fn parse_record(record: &str, row: TreeRow) -> Option<(&str, &str, &str)> {
    let (metadata, path) = record.split_once('\t')?;
    let mut fields = metadata.split_whitespace();
    let mode = fields.next()?;
    if matches!(row, TreeRow::Committed) {
        let _kind = fields.next()?;
    }
    let oid = fields.next()?;
    Some((mode, oid, path))
}

fn nul_paths(output: &str) -> impl Iterator<Item = &str> {
    output.split('\0').filter(|value| !value.is_empty())
}
