//! Pins that every hook currently listed in `LOOM_HOOKS` lands at its
//! destination, executable, on a fresh install. It iterates the same list
//! the installer walks, so it catches a hook that fails to install or lands
//! non-executable; it cannot catch a hook silently missing from `LOOM_HOOKS`
//! itself - that side is covered by
//! `fs/permissions/tests/constants_tests.rs`.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use super::{install, paths};
use crate::fs::permissions::constants::LOOM_HOOKS;
use crate::skills::SkillLayout;
use tempfile::TempDir;

#[test]
fn every_embedded_hook_is_installed_executable() {
    let temp = TempDir::new().unwrap();
    let report = install(&temp, SkillLayout::Core);
    let paths = paths(&temp);

    assert_eq!(report.hooks, LOOM_HOOKS.len());
    assert_eq!(report.codex_hooks, LOOM_HOOKS.len());
    for hooks_dir in [
        paths.claude_dir.join("hooks/loom"),
        paths.codex_dir.join("hooks/loom"),
    ] {
        for (filename, _) in LOOM_HOOKS {
            let path = hooks_dir.join(filename);
            assert!(path.is_file(), "{filename} was not installed");
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o755,
                "{filename} is not installed executable"
            );
        }
    }
}

#[test]
fn both_clients_receive_their_own_skill_keyword_index() {
    let temp = TempDir::new().unwrap();
    install(&temp, SkillLayout::Core);
    let paths = paths(&temp);
    for root in [paths.claude_dir, paths.codex_dir] {
        let index: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("hooks/loom/skill-keywords.json")).unwrap())
                .unwrap();
        assert!(index["rust"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("loom-rust")));
        assert!(index["react"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("loom-react")));
    }
}
