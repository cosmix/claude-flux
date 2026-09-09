use std::fs;
use std::path::Path;

use tempfile::TempDir;

use super::{for_files, resolve_skill};
use crate::skills::SkillIndex;

fn skill(root: &Path, name: &str, triggers: &str) {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Test skill\ntriggers: [{triggers}]\n---\n"),
    )
    .unwrap();
}

#[test]
fn detected_skill_is_promoted_even_when_keywords_already_matched() {
    let root = TempDir::new().unwrap();
    let skills = TempDir::new().unwrap();
    skill(skills.path(), "loom-rust", "rust, lifetime");
    let index = SkillIndex::load_from_directory(skills.path()).unwrap();
    assert_eq!(index.match_skills("rust lifetime", 8).len(), 1);
    let matches = for_files(
        &index,
        "rust lifetime",
        root.path(),
        &["src/main.rs".into()],
        &[],
    );
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].name, "loom-rust");
    assert_eq!(matches[0].score, 10.0);
    assert!(matches[0]
        .matched_triggers
        .iter()
        .any(|t| t.starts_with("project-type:rust")));
    let rendered = crate::orchestrator::signals::format_skill_recommendations(&matches);
    assert!(rendered.contains("Load these now"));
}

#[test]
fn react_stage_receives_both_framework_and_language_without_backend_skill() {
    let root = TempDir::new().unwrap();
    fs::create_dir(root.path().join(".git")).unwrap();
    fs::write(root.path().join("Cargo.toml"), "[workspace]").unwrap();
    fs::create_dir(root.path().join("web")).unwrap();
    fs::write(
        root.path().join("web/package.json"),
        r#"{"dependencies":{"react":"19","typescript":"6"}}"#,
    )
    .unwrap();
    let skills = TempDir::new().unwrap();
    for name in ["loom-rust", "loom-react", "loom-typescript"] {
        skill(skills.path(), name, "");
    }
    let index = SkillIndex::load_from_directory(skills.path()).unwrap();
    let matches = for_files(
        &index,
        "adjust the layout",
        root.path(),
        &["web/src/App.tsx".into()],
        &[],
    );
    assert_eq!(
        matches.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
        ["loom-react", "loom-typescript"]
    );
}

#[test]
fn resolver_supports_prefixed_bare_and_absent_skills() {
    let skills = TempDir::new().unwrap();
    skill(skills.path(), "loom-rust", "");
    skill(skills.path(), "python", "");
    let index = SkillIndex::load_from_directory(skills.path()).unwrap();
    assert_eq!(resolve_skill(&index, "rust").unwrap().name, "loom-rust");
    assert_eq!(resolve_skill(&index, "python").unwrap().name, "python");
    assert!(resolve_skill(&index, "golang").is_none());
}
