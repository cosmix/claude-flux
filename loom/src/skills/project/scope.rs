use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use super::{ProjectProfile, ProjectType};

pub(super) fn for_files(profile: &ProjectProfile, files: &[String]) -> Vec<ProjectType> {
    if files.is_empty() {
        return profile.types.clone();
    }
    let mut selected = BTreeSet::new();
    for file in files {
        let Some(prefix) = assignment_prefix(&profile.root, file) else {
            continue;
        };
        let nearest = profile
            .packages
            .iter()
            .filter(|path| prefix.starts_with(path))
            .map(|path| path.components().count())
            .max();
        for kind in &profile.types {
            let ancestor =
                prefix.starts_with(&kind.path) && Some(kind.path.components().count()) == nearest;
            if (ancestor || kind.path.starts_with(&prefix)) && compatible(&kind.kind, file) {
                selected.insert(kind.clone());
            }
        }
        for language in crate::language::detect_languages_from_files(std::slice::from_ref(file)) {
            if !selected
                .iter()
                .any(|kind| kind.kind == language.skill_name())
            {
                selected.insert(ProjectType {
                    kind: language.skill_name().to_string(),
                    path: prefix.parent().unwrap_or(Path::new("")).to_path_buf(),
                });
            }
        }
    }
    selected.into_iter().collect()
}

pub(super) fn for_prompt(profile: &ProjectProfile, cwd: &Path, prompt: &str) -> Vec<ProjectType> {
    let mut paths = Vec::new();
    for word in prompt.split_whitespace() {
        let word = word.trim_matches(|c: char| "`'\"(),:;<>".contains(c));
        if word.is_empty() || word == "." || word == ".." {
            continue;
        }
        if let Some(path) = prompt_path(profile, cwd, word) {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        paths.push(cwd.to_string_lossy().into_owned());
    }
    for_files(profile, &paths)
}

fn assignment_prefix(root: &Path, file: &str) -> Option<PathBuf> {
    let path = Path::new(file);
    let relative = if path.is_absolute() {
        path.strip_prefix(root).ok()?
    } else {
        path
    };
    let mut prefix = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::CurDir => (),
            Component::Normal(part) if !part.to_string_lossy().contains(['*', '?', '[']) => {
                prefix.push(part);
            }
            Component::Normal(_) => break,
            _ => return None,
        }
    }
    Some(prefix)
}

fn compatible(kind: &str, file: &str) -> bool {
    let languages = crate::language::detect_languages_from_files(&[file.to_string()]);
    languages.first().is_none_or(|language| {
        kind == language.skill_name()
            || (*language == crate::language::DetectedLanguage::TypeScript && kind == "react")
    })
}

fn prompt_path(profile: &ProjectProfile, cwd: &Path, word: &str) -> Option<String> {
    if word.contains("://") {
        return None;
    }
    for path in [cwd.join(word), profile.root.join(word)] {
        let text = path.to_string_lossy();
        let Some(prefix) = assignment_prefix(&profile.root, &text) else {
            continue;
        };
        let prefix_exists = profile.root.join(&prefix).exists();
        let parent_exists = prefix
            .parent()
            .is_some_and(|p| profile.root.join(p).is_dir());
        let source_file =
            !crate::language::detect_languages_from_files(&[word.to_string()]).is_empty();
        if path.exists() || (word.contains('*') && prefix_exists) || (source_file && parent_exists)
        {
            return Some(text.into_owned());
        }
    }
    None
}
