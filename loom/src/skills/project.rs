//! Bounded project discovery shared by prompt hooks and stage skill routing.

mod markers;
mod scan;
mod scope;

use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct ProjectType {
    pub kind: String,
    /// Package directory, relative to the checkout root.
    pub path: PathBuf,
}

#[derive(Debug, Default, Serialize)]
pub struct ProjectProfile {
    pub root: PathBuf,
    pub types: Vec<ProjectType>,
    /// Includes packages whose stack has no Loom skill, so they still prevent
    /// inheriting an unrelated language from a parent workspace manifest.
    pub packages: Vec<PathBuf>,
    pub truncated: bool,
}

impl ProjectProfile {
    pub fn discover(cwd: &Path) -> Self {
        let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
        let root = scan::checkout_root(&cwd);
        scan::discover(root)
    }

    /// Select the nearest package for each assignment, including packages below
    /// directory/glob assignments. Unrelated sibling packages never contribute.
    pub fn for_files(&self, files: &[String]) -> Vec<ProjectType> {
        scope::for_files(self, files)
    }

    pub fn for_prompt(&self, cwd: &Path, prompt: &str) -> Vec<ProjectType> {
        scope::for_prompt(self, cwd, prompt)
    }
}

#[cfg(test)]
mod tests;
