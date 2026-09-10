use anyhow::Result;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::excluded;
use crate::context::source_graph::body_hash;
use crate::git::runner::run_git_checked;

pub(crate) struct WorkingTree {
    pub head: String,
    /// sha256 hex over `head`, then one line per dirty path: `<xy> <path> <content_hash|deleted>`.
    pub generation: String,
    /// Path → status code from `git status --porcelain=v1 -z --untracked-files=all` (renames yield both paths).
    pub dirty: BTreeMap<String, String>,
}

pub(crate) fn working_tree(project_root: &Path) -> Result<WorkingTree> {
    let head = run_git_checked(&["rev-parse", "HEAD"], project_root)?;
    let status = run_git_checked(
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        project_root,
    )?;
    let dirty = parse_status(&status);
    let generation = generation(project_root, &head, &dirty)?;
    Ok(WorkingTree {
        head,
        generation,
        dirty,
    })
}

pub(crate) fn clean_generation(head: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(head.as_bytes());
    hasher.update(b"\n");
    hex::encode(hasher.finalize())
}

fn generation(project_root: &Path, head: &str, dirty: &BTreeMap<String, String>) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(head.as_bytes());
    hasher.update(b"\n");
    for (path, status) in dirty {
        let full_path = project_root.join(path);
        // A dirty path that exists but cannot be read (e.g. permission
        // bits) must not fail the whole refresh - fold the error into the
        // identity so the generation still hashes deterministically,
        // mirroring how `layer::read_bytes` treats an unreadable file.
        let identity = match fs::read(&full_path) {
            Ok(bytes) => body_hash(&bytes),
            Err(_) if !full_path.exists() => "deleted".to_string(),
            Err(error) => format!("unreadable:{error}"),
        };
        hasher.update(format!("{status} {path} {identity}\n").as_bytes());
    }
    Ok(hex::encode(hasher.finalize()))
}

fn parse_status(output: &str) -> BTreeMap<String, String> {
    let mut dirty = BTreeMap::new();
    let mut records = output.split('\0');
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        let Some((status, path)) = status_record(record) else {
            continue;
        };
        if !excluded(path) {
            dirty.insert(path.to_string(), status.clone());
        }
        if status.contains('R') || status.contains('C') {
            if let Some(source) = records.next() {
                if !source.is_empty() && !excluded(source) {
                    dirty.insert(source.to_string(), status);
                }
            }
        }
    }
    dirty
}

fn status_record(record: &str) -> Option<(String, &str)> {
    let bytes = record.as_bytes();
    if bytes.len() >= 3 && bytes[2] == b' ' {
        return Some((record[..2].to_string(), &record[3..]));
    }
    // `run_git_checked` trims a leading status-space from the first record.
    if bytes.len() >= 2 && bytes[1] == b' ' {
        return Some((format!(" {}", &record[..1]), &record[2..]));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_record_extracts_a_two_char_status_and_path() {
        assert_eq!(
            status_record("M  file.rs"),
            Some(("M ".to_string(), "file.rs"))
        );
    }

    #[test]
    fn status_record_recovers_a_trimmed_leading_status_space() {
        // The real record is " M mod.rs" (unstaged modification); if
        // `run_git_checked` trims the leading space it becomes "M mod.rs".
        // The recovery branch must reconstruct the original " M" status
        // rather than misreading it as status "M " on path " mod.rs".
        assert_eq!(
            status_record("M mod.rs"),
            Some((" M".to_string(), "mod.rs"))
        );
    }

    #[test]
    fn status_record_rejects_a_record_with_no_recognizable_separator() {
        assert_eq!(status_record("x"), None);
    }

    #[test]
    fn parse_status_on_empty_input_yields_no_dirty_paths() {
        assert!(parse_status("").is_empty());
    }

    #[test]
    fn parse_status_reads_a_plain_modification() {
        let dirty = parse_status("M  file.rs\0");
        assert_eq!(dirty.get("file.rs"), Some(&"M ".to_string()));
        assert_eq!(dirty.len(), 1);
    }

    #[test]
    fn parse_status_reads_an_untracked_entry() {
        let dirty = parse_status("?? new_file.txt\0");
        assert_eq!(dirty.get("new_file.txt"), Some(&"??".to_string()));
        assert_eq!(dirty.len(), 1);
    }

    #[test]
    fn generation_treats_a_dirty_path_missing_from_disk_as_deleted() {
        // The old side of a rename no longer exists on disk. `generation`
        // must fold it into the hash via the "deleted" identity rather than
        // silently reusing content that happens to still pass `fs::read`.
        let temp = tempfile::TempDir::new().unwrap();
        let mut dirty = BTreeMap::new();
        dirty.insert("gone.rs".to_string(), "R ".to_string());
        let head = "deadbeef";

        let got = generation(temp.path(), head, &dirty).unwrap();

        let mut hasher = Sha256::new();
        hasher.update(head.as_bytes());
        hasher.update(b"\n");
        hasher.update(b"R  gone.rs deleted\n");
        let want = hex::encode(hasher.finalize());

        assert_eq!(got, want);
    }

    #[test]
    fn parse_status_rename_record_consumes_both_paths_without_shifting_the_next_record() {
        // A rename occupies two NUL-separated fields: the destination path
        // (with the "R " status prefix) then the bare source path. Getting
        // this wrong would shift every following record by one.
        let input = "R  new.rs\0old.rs\0M  next.rs\0";
        let dirty = parse_status(input);
        assert_eq!(dirty.get("new.rs"), Some(&"R ".to_string()));
        assert_eq!(dirty.get("old.rs"), Some(&"R ".to_string()));
        assert_eq!(
            dirty.get("next.rs"),
            Some(&"M ".to_string()),
            "the record after a rename must not be shifted or swallowed"
        );
        assert_eq!(dirty.len(), 3);
    }
}
