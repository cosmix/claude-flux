use anyhow::Result;
use std::collections::BTreeMap;
use std::path::Path;

use super::{excluded, SourceGraphScope};
use crate::git::runner::run_git_checked;

pub(crate) struct Enumeration {
    /// Path → blob object id, from `git ls-tree -r -z HEAD` (Base) or `git ls-files -s -z` (Overlay).
    pub known: BTreeMap<String, String>,
    /// Existing untracked files from `git ls-files --others --exclude-standard -z` (Overlay only).
    pub untracked: Vec<String>,
    /// Known paths absent from the working tree (Overlay only).
    pub deleted: Vec<String>,
}

pub(crate) fn enumerate(project_root: &Path, scope: &SourceGraphScope) -> Result<Enumeration> {
    match scope {
        SourceGraphScope::Base { .. } => {
            let output = run_git_checked(&["ls-tree", "-r", "-z", "HEAD"], project_root)?;
            Ok(Enumeration {
                known: parse_known(&output, TreeRow::Committed),
                untracked: Vec::new(),
                deleted: Vec::new(),
            })
        }
        SourceGraphScope::Overlay { .. } => {
            let output = run_git_checked(&["ls-files", "-s", "-z"], project_root)?;
            let known = parse_known(&output, TreeRow::Index);
            let output = run_git_checked(
                &["ls-files", "--others", "--exclude-standard", "-z"],
                project_root,
            )?;
            let mut untracked = nul_paths(&output)
                .filter(|path| !excluded(path) && project_root.join(path).exists())
                .map(str::to_string)
                .collect::<Vec<_>>();
            untracked.sort();
            let deleted = known
                .keys()
                .filter(|path| !project_root.join(path).exists())
                .cloned()
                .collect();
            Ok(Enumeration {
                known,
                untracked,
                deleted,
            })
        }
    }
}

#[derive(Clone, Copy)]
enum TreeRow {
    Committed,
    Index,
}

fn parse_known(output: &str, row: TreeRow) -> BTreeMap<String, String> {
    nul_paths(output)
        .filter_map(|record| {
            let (metadata, path) = record.split_once('\t')?;
            if excluded(path) {
                return None;
            }
            let mut fields = metadata.split_whitespace();
            let _mode = fields.next()?;
            if matches!(row, TreeRow::Committed) {
                let _kind = fields.next()?;
            }
            let oid = fields.next()?;
            Some((path.to_string(), oid.to_string()))
        })
        .collect()
}

fn nul_paths(output: &str) -> impl Iterator<Item = &str> {
    output.split('\0').filter(|value| !value.is_empty())
}
