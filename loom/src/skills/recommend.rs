//! Combine keyword recommendations with direct, package-scoped type evidence.

use std::path::Path;

use crate::language::DetectedLanguage;

use super::project::ProjectProfile;
use super::{SkillIndex, SkillMatch, SkillMetadata};

pub const MAX_RECOMMENDATIONS: usize = 8;

pub fn for_files(
    index: &SkillIndex,
    text: &str,
    root: &Path,
    files: &[String],
    fallback: &[DetectedLanguage],
) -> Vec<SkillMatch> {
    let mut matches = index.match_skills(text, MAX_RECOMMENDATIONS);
    let profile = ProjectProfile::discover(root);
    let types = profile.for_files(files);
    for kind in &types {
        if let Some(metadata) = resolve_skill(index, &kind.kind) {
            promote(
                &mut matches,
                metadata,
                format!("project-type:{} ({})", kind.kind, kind.path.display()),
            );
        }
    }
    if types.is_empty() && files.is_empty() {
        for language in fallback {
            if let Some(metadata) = resolve_skill(index, language.skill_name()) {
                promote(&mut matches, metadata, "project-language".into());
            }
        }
    }
    matches.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.name.cmp(&b.name))
    });
    matches.truncate(MAX_RECOMMENDATIONS);
    matches
}

fn promote(matches: &mut Vec<SkillMatch>, metadata: &SkillMetadata, marker: String) {
    if let Some(existing) = matches.iter_mut().find(|skill| skill.name == metadata.name) {
        existing.score = existing.score.max(10.0);
        if !existing.matched_triggers.contains(&marker) {
            existing.matched_triggers.push(marker);
        }
    } else {
        matches.push(SkillMatch::new(
            metadata.name.clone(),
            metadata.description.clone(),
            10.0,
            vec![marker],
        ));
    }
}

fn resolve_skill<'a>(index: &'a SkillIndex, base: &str) -> Option<&'a SkillMetadata> {
    index
        .get_by_name(&format!("loom-{base}"))
        .or_else(|| index.get_by_name(base))
}

#[cfg(test)]
mod tests;
