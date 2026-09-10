//! Discover design briefs using the same references recognized by plan validation.

use anyhow::{bail, Context, Result};
use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::plan::schema::extract_brief_paths;

pub(super) fn collect_inputs(root: &Path, plan: &Path) -> Result<BTreeSet<PathBuf>> {
    let plan = plan.strip_prefix(root).unwrap_or(plan);
    let root = root.canonicalize()?;
    let plan = relative_path(&root, plan)?;
    let mut pending = VecDeque::from([plan.clone()]);
    // Include all worker briefs in the conventional bundle, even if the plan only
    // links to coordinator briefs that in turn name the workers.
    if let Some(stem) = plan.file_stem().and_then(|name| name.to_str()) {
        let slug = stem
            .trim_start_matches("IN_PROGRESS-")
            .trim_start_matches("DONE-")
            .trim_start_matches("PLAN-");
        let bundle = PathBuf::from("doc/plans/briefs").join(slug);
        if root.join(&bundle).exists() {
            pending.push_back(bundle);
        }
    }
    let mut inputs = BTreeSet::new();
    let mut visited = BTreeSet::new();
    while let Some(path) = pending.pop_front() {
        if expand_pattern(&root, &path, &mut pending)? {
            continue;
        }
        let path = relative_path(&root, &path)?;
        if !visited.insert(path.clone()) {
            continue;
        }
        let absolute = root.join(&path);
        if absolute.is_dir() {
            for entry in fs::read_dir(&absolute)? {
                pending.push_back(path.join(entry?.file_name()));
            }
        } else {
            inputs.insert(path);
            if absolute.is_file() && absolute.extension().is_some_and(|ext| ext == "md") {
                let content = fs::read_to_string(&absolute)
                    .with_context(|| format!("Cannot read plan input {}", absolute.display()))?;
                pending.extend(extract_brief_paths(&content).into_iter().map(PathBuf::from));
            }
        }
    }
    Ok(inputs)
}

fn expand_pattern(root: &Path, path: &Path, pending: &mut VecDeque<PathBuf>) -> Result<bool> {
    let text = path.to_str().context("Plan input path is not UTF-8")?;
    if root.join(path).exists() || !text.contains(['*', '?', '[']) {
        return Ok(false);
    }
    let pattern = format!(
        "{}/{}",
        glob::Pattern::escape(&root.to_string_lossy()),
        text
    );
    let matches = glob::glob(&pattern)?.collect::<std::result::Result<Vec<_>, _>>()?;
    if matches.is_empty() {
        return Ok(false); // Retain the literal path for the missing-input diagnostic.
    }
    pending.extend(matches);
    Ok(true)
}

fn relative_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let absolute = root.join(path);
    let relative = absolute.strip_prefix(root).with_context(|| {
        format!(
            "Plan input {} must be inside the repository",
            path.display()
        )
    })?;
    if relative
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        bail!(
            "Plan input {} must not traverse parent directories",
            path.display()
        );
    }
    if let Ok(resolved) = absolute.canonicalize() {
        if resolved != absolute.components().collect::<PathBuf>() {
            bail!(
                "Plan input {} must use a regular repository path, not a symlink",
                path.display()
            );
        }
    }
    Ok(relative.components().collect())
}
