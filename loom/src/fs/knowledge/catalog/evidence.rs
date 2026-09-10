//! Verification-point checks for declared frontmatter sources.

use super::CatalogIssue;
use crate::fs::knowledge::frontmatter::Frontmatter;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Compare every declared source in one file with its recorded verification
/// revision. This deliberately performs exactly one git invocation per file.
pub(super) fn changed_since_verified(
    project_root: Option<&Path>,
    file: &Path,
    frontmatter: &Frontmatter,
) -> Vec<CatalogIssue> {
    let (Some(project_root), Some(verified)) = (project_root, frontmatter.verified.as_deref())
    else {
        return Vec::new();
    };
    if frontmatter.sources.is_empty() {
        return Vec::new();
    }
    if verified.trim().is_empty() || verified.starts_with('-') {
        tracing::debug!(file = %file.display(), "skipping invalid evidence revision");
        return Vec::new();
    }

    let Some(changed) = changed_paths_since(project_root, file, verified, &frontmatter.sources)
    else {
        return Vec::new();
    };
    frontmatter
        .sources
        .iter()
        .filter(|source| changed.contains(&normalize_path(source)))
        .map(|source_path| CatalogIssue::EvidenceChanged {
            file: PathBuf::from(file),
            source_path: source_path.clone(),
            verified: verified.to_string(),
        })
        .collect()
}

/// Run `git diff --name-only <verified>..HEAD -- <sources>` and return the
/// changed paths, normalized. `None` (with a debug trace) on any git
/// failure: a git problem must never fail the catalog build, only skip the
/// freshness check for this one file.
fn changed_paths_since(
    project_root: &Path,
    file: &Path,
    verified: &str,
    sources: &[String],
) -> Option<BTreeSet<String>> {
    let range = format!("{verified}..HEAD");
    let output = Command::new("git")
        .current_dir(project_root)
        .args(["diff", "--name-only", &range, "--"])
        .args(sources)
        .output();
    let output = match output {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            tracing::debug!(
                file = %file.display(),
                status = %output.status,
                "skipping evidence freshness after git diff failure"
            );
            return None;
        }
        Err(error) => {
            tracing::debug!(
                file = %file.display(),
                %error,
                "skipping evidence freshness after git invocation failure"
            );
            return None;
        }
    };
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(normalize_path)
            .collect(),
    )
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}
