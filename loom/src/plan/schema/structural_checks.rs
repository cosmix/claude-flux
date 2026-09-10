//! Structural preflight checks split out of `validation.rs` to keep
//! `validate_structural_preflight` itself from growing past its recorded
//! size ceiling. Both checks below feed into the same `Vec<String>`
//! `validate_structural_preflight` returns - advisory warnings, same
//! severity as its other checks, never hard errors.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use super::types::StageDefinition;

/// Warn when a stage's `description` references a design brief under
/// `doc/plans/briefs/` that does not exist in the repo.
///
/// Only fires when a `doc/plans/briefs/...` token is actually present in the
/// description text - most stages have no such reference and get no
/// warning. No-op when `repo_root` is `None` (nothing to resolve against).
pub(super) fn check_missing_brief_paths(
    stages: &[StageDefinition],
    repo_root: Option<&Path>,
) -> Vec<String> {
    let mut warnings = Vec::new();
    let Some(root) = repo_root else {
        return warnings;
    };

    for stage in stages {
        let Some(description) = &stage.description else {
            continue;
        };
        for brief_path in extract_brief_paths(description) {
            let resolved = root.join(&brief_path);
            if !resolved.exists() {
                warnings.push(format!(
                    "Stage '{}': description references '{}' under doc/plans/briefs/, \
                     which does not exist at {}",
                    stage.id,
                    brief_path,
                    resolved.display()
                ));
            }
        }
    }

    warnings
}

/// Extract `doc/plans/briefs/...` path-like tokens from free-form text.
///
/// A plain substring/token scan, not a markdown-link parser: split on
/// whitespace and common delimiters, trim surrounding punctuation, and keep
/// tokens that contain the brief-directory prefix.
pub(crate) fn extract_brief_paths(text: &str) -> Vec<String> {
    const PREFIX: &str = "doc/plans/briefs/";
    let mut paths = Vec::new();

    for token in text.split(|c: char| {
        c.is_whitespace() || matches!(c, '(' | ')' | '[' | ']' | '"' | '\'' | ',' | ';' | '|')
    }) {
        let trimmed = token.trim_matches(|c: char| matches!(c, '.' | ',' | ':' | '`'));
        if let Some(idx) = trimmed.find(PREFIX) {
            let path = trimmed[idx..].split('#').next().unwrap_or_default();
            if path.len() > PREFIX.len() {
                paths.push(path.to_string());
            }
        }
    }

    paths
}

/// Warn when two stages with no dependency relationship (in either
/// direction, transitively) declare overlapping `files:` globs. Two stages
/// with no dependency path between them can execute concurrently, and
/// concurrent writes to the same files are a lost-work hazard.
///
/// Overlap is judged on the glob patterns themselves - a string comparison
/// on each pattern's literal (non-wildcard) prefix, never filesystem
/// expansion - so the check is deterministic and cheap.
pub(super) fn check_overlapping_files_without_dependency(
    stages: &[StageDefinition],
) -> Vec<String> {
    let mut warnings = Vec::new();
    if stages.len() < 2 {
        return warnings;
    }

    let index_by_id: HashMap<&str, usize> = stages
        .iter()
        .enumerate()
        .map(|(idx, stage)| (stage.id.as_str(), idx))
        .collect();

    let closures: Vec<HashSet<usize>> = (0..stages.len())
        .map(|idx| transitive_dependencies(idx, stages, &index_by_id))
        .collect();

    for i in 0..stages.len() {
        for j in (i + 1)..stages.len() {
            // A dependency path in either direction means the two stages
            // cannot run concurrently regardless of file overlap.
            if closures[i].contains(&j) || closures[j].contains(&i) {
                continue;
            }

            for p1 in &stages[i].files {
                for p2 in &stages[j].files {
                    if glob_prefixes_overlap(p1, p2) {
                        warnings.push(format!(
                            "Stage '{}': files pattern '{}' overlaps stage '{}' files pattern \
                             '{}' with no dependency relationship between them — two stages \
                             that can run concurrently must not both write the same files",
                            stages[i].id, p1, stages[j].id, p2
                        ));
                    }
                }
            }
        }
    }

    warnings
}

/// BFS over `dependencies` to compute the set of stage indices `start`
/// transitively depends on.
fn transitive_dependencies(
    start: usize,
    stages: &[StageDefinition],
    index_by_id: &HashMap<&str, usize>,
) -> HashSet<usize> {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(start);

    while let Some(current) = queue.pop_front() {
        for dep in &stages[current].dependencies {
            if let Some(&dep_idx) = index_by_id.get(dep.as_str()) {
                if visited.insert(dep_idx) {
                    queue.push_back(dep_idx);
                }
            }
        }
    }

    visited
}

/// The literal, non-wildcard prefix of a glob pattern: the substring up to
/// (not including) the first wildcard character, with a trailing `/`
/// trimmed. A pattern with no wildcard yields the whole literal path.
fn glob_prefix(pattern: &str) -> &str {
    let end = pattern.find(['*', '?', '[']).unwrap_or(pattern.len());
    pattern[..end].trim_end_matches('/')
}

/// Two glob patterns overlap when one's prefix is a path-component-wise
/// prefix of the other: equal, or one equals the other plus `/` plus more.
fn glob_prefixes_overlap(p1: &str, p2: &str) -> bool {
    let a = glob_prefix(p1);
    let b = glob_prefix(p2);
    // An empty literal prefix (e.g. "**/*.rs", where the wildcard starts
    // the pattern) means the match set is not bounded by any directory, so
    // it is treated as overlapping everything - "" is not a "/"-joined
    // prefix of a non-empty path, so the component-wise comparison below
    // would otherwise call it disjoint. This is deliberately conservative:
    // the heuristic compares directory prefixes only, never extensions, so
    // "**/*.rs" and "**/*.md" still warn even though they match no files in
    // common. A missed overlap costs a stage its work in a merge conflict;
    // a spurious warning costs a plan author one glance - do not "fix" this.
    if a.is_empty() || b.is_empty() {
        return true;
    }
    a == b || a.starts_with(&format!("{b}/")) || b.starts_with(&format!("{a}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(
        id: &str,
        deps: &[&str],
        files: &[&str],
        description: Option<&str>,
    ) -> StageDefinition {
        let mut s = crate::plan::schema::tests::make_stage(id, id);
        s.dependencies = deps.iter().map(|d| d.to_string()).collect();
        s.files = files.iter().map(|f| f.to_string()).collect();
        s.description = description.map(|d| d.to_string());
        s
    }

    #[test]
    fn overlapping_globs_without_dependency_warn() {
        let stages = vec![
            stage("a", &[], &["src/a/**"], None),
            stage("b", &[], &["src/a/config.rs"], None),
        ];
        let warnings = check_overlapping_files_without_dependency(&stages);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("src/a/**"));
        assert!(warnings[0].contains("src/a/config.rs"));
    }

    #[test]
    fn empty_prefix_glob_overlaps_rooted_pattern() {
        let stages = vec![
            stage("a", &[], &["**/*.rs"], None),
            stage("b", &[], &["loom/src/plan/**"], None),
        ];
        let warnings = check_overlapping_files_without_dependency(&stages);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("**/*.rs"));
        assert!(warnings[0].contains("loom/src/plan/**"));
    }

    #[test]
    fn disjoint_globs_do_not_warn() {
        let stages = vec![
            stage("a", &[], &["src/a/**"], None),
            stage("b", &[], &["src/b/**"], None),
        ];
        assert!(check_overlapping_files_without_dependency(&stages).is_empty());
    }

    #[test]
    fn overlapping_globs_with_dependency_do_not_warn() {
        let stages = vec![
            stage("a", &[], &["src/a/**"], None),
            stage("b", &["a"], &["src/a/config.rs"], None),
        ];
        assert!(check_overlapping_files_without_dependency(&stages).is_empty());
    }

    #[test]
    fn transitive_dependency_suppresses_warning() {
        let stages = vec![
            stage("a", &[], &["src/a/**"], None),
            stage("b", &["a"], &[], None),
            stage("c", &["b"], &["src/a/config.rs"], None),
        ];
        assert!(check_overlapping_files_without_dependency(&stages).is_empty());
    }

    #[test]
    fn missing_brief_path_warns_when_repo_root_present() {
        let dir = tempfile::tempdir().expect("tempdir");
        let stages = vec![stage(
            "a",
            &[],
            &[],
            Some("See `doc/plans/briefs/nonexistent.md` for design context."),
        )];
        let warnings = check_missing_brief_paths(&stages, Some(dir.path()));
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("doc/plans/briefs/nonexistent.md"));
    }

    #[test]
    fn description_without_brief_reference_is_silent() {
        let stages = vec![stage("a", &[], &[], Some("Implement the thing."))];
        assert!(check_missing_brief_paths(&stages, Some(Path::new("/tmp"))).is_empty());
    }

    #[test]
    fn no_repo_root_is_silent() {
        let stages = vec![stage(
            "a",
            &[],
            &[],
            Some("doc/plans/briefs/nonexistent.md"),
        )];
        assert!(check_missing_brief_paths(&stages, None).is_empty());
    }
}
