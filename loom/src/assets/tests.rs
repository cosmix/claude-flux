use super::install::{install_all, InstallPaths, InstallReport};
use crate::skills::SkillLayout;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[path = "tests/doctrine.rs"]
mod doctrine;
#[path = "tests/hooks.rs"]
mod hooks;
#[path = "tests/layout.rs"]
mod layout;
#[path = "tests/placement.rs"]
mod placement;
#[path = "tests/write_assets.rs"]
mod write_assets;

fn paths(temp: &TempDir) -> InstallPaths {
    InstallPaths {
        claude_dir: temp.path().join("claude"),
        codex_dir: temp.path().join("codex"),
    }
}

fn install(temp: &TempDir, layout: SkillLayout) -> InstallReport {
    install_all(&paths(temp), Some(layout), false).unwrap()
}

fn asset_content(assets: &[crate::assets::Asset], path: &str) -> &'static str {
    assets
        .iter()
        .find_map(|(key, content)| (*key == path).then_some(*content))
        .unwrap()
}

fn first_distinctive_line(template: &str) -> &str {
    template.lines().find(|line| line.len() > 15).unwrap()
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    collect_files(root, root, &mut files);
    files
}

/// Directories are recorded alongside files (with an empty value), so a stray
/// empty directory created or removed by a second install is visible too.
fn collect_files(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let relative = path.strip_prefix(root).unwrap().to_path_buf();
        if path.is_dir() {
            files.insert(relative, Vec::new());
            collect_files(root, &path, files);
        } else {
            files.insert(relative, fs::read(&path).unwrap());
        }
    }
}

fn backup_count(dir: &Path, name: &str) -> usize {
    backup_paths(dir, name).len()
}

/// The single backup file for `name` under `dir`. Panics unless exactly one
/// exists, since every caller of this helper is asserting that a prior
/// install already created the one backup it expects to find.
fn the_backup(dir: &Path, name: &str) -> PathBuf {
    let mut found = backup_paths(dir, name);
    assert_eq!(
        found.len(),
        1,
        "expected exactly one backup of {name} under {}",
        dir.display()
    );
    found.remove(0)
}

fn backup_paths(dir: &Path, name: &str) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("{name}.bak."))
        })
        .map(|entry| entry.path())
        .collect()
}

fn seed_user_assets(paths: &InstallPaths) {
    for path in [
        paths.claude_dir.join("skills/rust/SKILL.md"),
        paths.claude_dir.join("skills/my-custom/SKILL.md"),
        paths.claude_dir.join("skills/loom-mine/SKILL.md"),
        paths.claude_dir.join("agents/my-agent.md"),
        paths.claude_dir.join("commands/my-cmd.md"),
        paths.codex_dir.join("skills/my-codex-skill/SKILL.md"),
        paths
            .claude_dir
            .join("loom-skill-catalog/loom-mine/SKILL.md"),
    ] {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "user-owned").unwrap();
    }
}

#[test]
fn core_places_only_core_skills_and_catalogues_the_rest() {
    let temp = TempDir::new().unwrap();
    let paths = paths(&temp);
    install_all(&paths, Some(SkillLayout::Core), false).unwrap();

    assert!(paths
        .claude_dir
        .join("skills/loom-plan-writer/SKILL.md")
        .is_file());
    assert!(paths
        .claude_dir
        .join("loom-skill-catalog/loom-rust/SKILL.md")
        .is_file());
    assert!(!paths.claude_dir.join("skills/loom-rust").exists());
    // loom-md-tables is a core skill, so it lands resident; the assertion that
    // matters is that its nested non-markdown files travel with it.
    assert!(paths
        .claude_dir
        .join("skills/loom-md-tables/fix-md-tables.py")
        .is_file());
}

#[test]
fn all_places_every_skill_and_removes_empty_catalogs() {
    let temp = TempDir::new().unwrap();
    let paths = paths(&temp);
    install_all(&paths, Some(SkillLayout::All), false).unwrap();

    assert!(paths.claude_dir.join("skills/loom-rust/SKILL.md").is_file());
    assert!(paths.codex_dir.join("skills/loom-rust/SKILL.md").is_file());
    assert!(!paths.claude_dir.join("loom-skill-catalog").exists());
    assert!(!paths.codex_dir.join("loom-skill-catalog").exists());
}

#[test]
fn codex_loader_differs_from_claude_loader() {
    let temp = TempDir::new().unwrap();
    let paths = paths(&temp);
    install_all(&paths, Some(SkillLayout::Core), false).unwrap();

    let installed = fs::read(paths.codex_dir.join("skills/loom-skills/SKILL.md")).unwrap();
    let codex = asset_content(crate::assets::CODEX_SKILLS, "loom-skills/SKILL.md");
    let claude = asset_content(crate::assets::SKILLS, "loom-skills/SKILL.md");
    assert_eq!(installed, codex.as_bytes());
    assert_ne!(installed, claude.as_bytes());
}

#[test]
fn codex_pressure_is_resident_under_both_layouts() {
    for layout in [SkillLayout::Core, SkillLayout::All] {
        let temp = TempDir::new().unwrap();
        assert_eq!(install(&temp, layout).layout, layout);
        assert!(paths(&temp)
            .codex_dir
            .join("skills/pressure/SKILL.md")
            .is_file());
        assert!(!paths(&temp).claude_dir.join("skills/pressure").exists());
    }
}

#[test]
fn hooks_and_managed_documents_have_expected_form() {
    let temp = TempDir::new().unwrap();
    let paths = paths(&temp);
    install_all(&paths, Some(SkillLayout::Core), false).unwrap();

    let hook = paths.claude_dir.join("hooks/loom/post-tool-use.sh");
    assert_eq!(
        fs::metadata(hook).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert!(paths
        .codex_dir
        .join("hooks/loom/codex-apply-patch.sh")
        .is_file());
    let codex_hooks: serde_json::Value =
        serde_json::from_slice(&fs::read(paths.codex_dir.join("hooks.json")).unwrap()).unwrap();
    assert!(codex_hooks["hooks"]["PreToolUse"].is_array());
    assert!(codex_hooks["hooks"]["PostToolUse"].is_array());
    assert!(paths
        .claude_dir
        .join("agents/loom-software-engineer.md")
        .is_file());
    assert!(paths.claude_dir.join("commands/pressure.md").is_file());
    assert!(paths
        .claude_dir
        .join("hooks/loom/skill-keywords.json")
        .is_file());
    for (path, template) in [
        (
            paths.claude_dir.join("CLAUDE.md"),
            crate::assets::CLAUDE_MD_TEMPLATE,
        ),
        (
            paths.codex_dir.join("AGENTS.md"),
            crate::assets::AGENTS_MD_TEMPLATE,
        ),
    ] {
        let content = fs::read_to_string(path).unwrap();
        assert!(content.starts_with("# ─"));
        assert!(content.contains(first_distinctive_line(template)));
    }
}

#[test]
fn reinstall_is_idempotent() {
    let temp = TempDir::new().unwrap();
    install(&temp, SkillLayout::Core);
    let before = snapshot(temp.path());

    let report = install(&temp, SkillLayout::Core);

    assert!(report.backups.is_empty());
    assert_eq!(snapshot(temp.path()), before);
}

#[test]
fn core_after_all_moves_catalogued_skill() {
    let temp = TempDir::new().unwrap();
    let paths = paths(&temp);
    install_all(&paths, Some(SkillLayout::All), false).unwrap();
    install_all(&paths, Some(SkillLayout::Core), false).unwrap();

    for root in [&paths.claude_dir, &paths.codex_dir] {
        assert!(root.join("loom-skill-catalog/loom-rust/SKILL.md").is_file());
        assert!(!root.join("skills/loom-rust").exists());
    }
}

#[test]
fn all_after_core_moves_catalogued_skill_back() {
    let temp = TempDir::new().unwrap();
    let paths = paths(&temp);
    install_all(&paths, Some(SkillLayout::Core), false).unwrap();
    install_all(&paths, Some(SkillLayout::All), false).unwrap();

    for root in [&paths.claude_dir, &paths.codex_dir] {
        assert!(root.join("skills/loom-rust/SKILL.md").is_file());
        assert!(!root.join("loom-skill-catalog").exists());
    }
}

#[test]
fn stale_other_root_copy_is_removed_for_embedded_skill() {
    let temp = TempDir::new().unwrap();
    let paths = paths(&temp);
    fs::create_dir_all(paths.claude_dir.join("loom-skill-catalog/loom-plan-writer")).unwrap();
    fs::create_dir_all(paths.codex_dir.join("loom-skill-catalog/loom-skills")).unwrap();

    install_all(&paths, Some(SkillLayout::Core), false).unwrap();

    assert!(!paths
        .claude_dir
        .join("loom-skill-catalog/loom-plan-writer")
        .exists());
    assert!(!paths
        .codex_dir
        .join("loom-skill-catalog/loom-skills")
        .exists());
}

#[test]
fn layout_record_tracks_the_applied_layout() {
    let temp = TempDir::new().unwrap();
    let paths = paths(&temp);
    install_all(&paths, Some(SkillLayout::Core), false).unwrap();
    assert!(
        fs::read_to_string(paths.claude_dir.join("loom-install.toml"))
            .unwrap()
            .contains("skills = \"core\"")
    );

    install_all(&paths, Some(SkillLayout::All), false).unwrap();
    assert!(
        fs::read_to_string(paths.claude_dir.join("loom-install.toml"))
            .unwrap()
            .contains("skills = \"all\"")
    );
}

#[test]
fn user_owned_assets_survive() {
    for layout in [SkillLayout::Core, SkillLayout::All] {
        let temp = TempDir::new().unwrap();
        let paths = paths(&temp);
        seed_user_assets(&paths);
        install_all(&paths, Some(layout), false).unwrap();

        for path in [
            paths.claude_dir.join("skills/rust/SKILL.md"),
            paths.claude_dir.join("skills/my-custom/SKILL.md"),
            paths.claude_dir.join("skills/loom-mine/SKILL.md"),
            paths.claude_dir.join("agents/my-agent.md"),
            paths.claude_dir.join("commands/my-cmd.md"),
            paths.codex_dir.join("skills/my-codex-skill/SKILL.md"),
        ] {
            assert!(path.is_file(), "{} was removed", path.display());
        }
        assert!(paths
            .claude_dir
            .join("loom-skill-catalog/loom-mine")
            .is_dir());
    }
}

#[test]
fn skill_directory_replaces_a_stale_file_at_either_root() {
    let temp = TempDir::new().unwrap();
    let paths = paths(&temp);
    let resident = paths.claude_dir.join("skills/loom-plan-writer");
    let catalogued = paths.claude_dir.join("loom-skill-catalog/loom-rust");
    for path in [&resident, &catalogued] {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "a file where a skill directory belongs").unwrap();
    }

    install_all(&paths, Some(SkillLayout::Core), false).unwrap();

    assert!(resident.join("SKILL.md").is_file());
    assert!(catalogued.join("SKILL.md").is_file());
}

#[test]
fn dropped_skill_files_are_pruned() {
    let temp = TempDir::new().unwrap();
    let paths = paths(&temp);
    install_all(&paths, Some(SkillLayout::Core), false).unwrap();
    let skill = paths.claude_dir.join("skills/loom-plan-writer");
    fs::write(skill.join("stray.md"), "dropped upstream").unwrap();

    install_all(&paths, Some(SkillLayout::Core), false).unwrap();

    assert!(!skill.join("stray.md").exists());
    assert!(skill.join("SKILL.md").is_file());
}

/// A skill directory is pruned, so a link inside one is unlinked rather than
/// followed: pruning through it would delete files loom does not own.
#[test]
fn symlinked_subdirectory_inside_a_skill_tree_is_pruned() {
    let temp = TempDir::new().unwrap();
    let paths = paths(&temp);
    install_all(&paths, Some(SkillLayout::Core), false).unwrap();
    let target = temp.path().join("operator-notes");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("keep.md"), "operator content").unwrap();
    let link = paths.claude_dir.join("skills/loom-plan-writer/linked");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    install_all(&paths, Some(SkillLayout::Core), false).unwrap();

    assert!(link.symlink_metadata().is_err());
    assert!(target.join("keep.md").is_file());
}

/// The link is unlinked, not written through: `prune_skill` would otherwise
/// delete whatever the operator keeps at its target.
#[test]
fn symlinked_skill_directory_is_replaced_by_a_real_one() {
    let temp = TempDir::new().unwrap();
    let paths = paths(&temp);
    let target = temp.path().join("elsewhere/loom-plan-writer");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("operator.md"), "operator content").unwrap();
    let dest = paths.claude_dir.join("skills/loom-plan-writer");
    fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, &dest).unwrap();

    install_all(&paths, Some(SkillLayout::Core), false).unwrap();

    assert!(!dest.symlink_metadata().unwrap().file_type().is_symlink());
    assert!(dest.join("SKILL.md").is_file());
    assert!(target.join("operator.md").is_file());
    assert!(!target.join("SKILL.md").exists());
}

#[test]
fn install_all_rejects_identical_trees() {
    let temp = TempDir::new().unwrap();
    let paths = InstallPaths {
        claude_dir: temp.path().join("both"),
        codex_dir: temp.path().join("both"),
    };

    assert!(install_all(&paths, Some(SkillLayout::Core), false).is_err());
}
