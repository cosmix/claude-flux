use std::fs;
use std::os::unix::fs::PermissionsExt;

use serde_json::{json, Value};
use tempfile::TempDir;

use super::{codex_hooks_need_install_in, install_codex_hooks_to, BASH_HOOKS};
use crate::fs::permissions::constants::LOOM_HOOKS;

#[test]
fn install_writes_assets_and_codex_native_config() {
    let temp = TempDir::new().unwrap();
    assert!(codex_hooks_need_install_in(temp.path()));
    let count = install_codex_hooks_to(temp.path()).unwrap();

    assert_eq!(count, LOOM_HOOKS.len());
    let bridge = temp.path().join("hooks/loom/codex-apply-patch.sh");
    assert_eq!(
        fs::metadata(bridge).unwrap().permissions().mode() & 0o777,
        0o755
    );

    let config: Value =
        serde_json::from_slice(&fs::read(temp.path().join("hooks.json")).unwrap()).unwrap();
    assert_eq!(config["hooks"]["SessionStart"].as_array().unwrap().len(), 1);
    assert_eq!(
        config["hooks"]["UserPromptSubmit"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(config["hooks"]["PreToolUse"].as_array().unwrap().len(), 2);
    let bash = &config["hooks"]["PreToolUse"][0];
    assert_eq!(bash["matcher"], "^Bash$");
    assert_eq!(bash["hooks"].as_array().unwrap().len(), BASH_HOOKS.len());
    assert_eq!(
        config["hooks"]["PostToolUse"][0]["matcher"],
        "^apply_patch$"
    );
    assert!(!codex_hooks_need_install_in(temp.path()));
}

#[test]
fn codex_prompt_registers_skill_detection_with_native_rendering() {
    let temp = TempDir::new().unwrap();
    install_codex_hooks_to(temp.path()).unwrap();
    let config: Value =
        serde_json::from_slice(&fs::read(temp.path().join("hooks.json")).unwrap()).unwrap();
    let commands = config["hooks"]["UserPromptSubmit"][0]["hooks"]
        .as_array()
        .unwrap();
    assert!(commands.iter().any(|hook| hook["command"]
        .as_str()
        .unwrap()
        .ends_with("skill-trigger.sh --codex")));
    assert!(commands.iter().any(|hook| hook["command"]
        .as_str()
        .unwrap()
        .ends_with("user-prompt-context.sh")));
}

#[test]
fn reinstall_replaces_loom_rules_and_preserves_user_hooks() {
    let temp = TempDir::new().unwrap();
    let config = json!({
        "description": "operator owned",
        "hooks": {
            "PreToolUse": [
                {"matcher": "Bash", "hooks": [{"type": "command", "command": "/custom/check.sh"}]},
                {"matcher": "Bash", "hooks": [{"type": "command", "command": format!("{}/hooks/loom/old.sh", temp.path().display())}]}
            ],
            "Stop": [{"hooks": [{"type": "command", "command": "/custom/stop.sh"}]}]
        }
    });
    fs::write(
        temp.path().join("hooks.json"),
        serde_json::to_vec_pretty(&config).unwrap(),
    )
    .unwrap();

    install_codex_hooks_to(temp.path()).unwrap();
    install_codex_hooks_to(temp.path()).unwrap();

    let installed: Value =
        serde_json::from_slice(&fs::read(temp.path().join("hooks.json")).unwrap()).unwrap();
    assert_eq!(installed["description"], "operator owned");
    let pre = installed["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(
        pre.iter()
            .filter(|rule| rule.to_string().contains("/custom/check.sh"))
            .count(),
        1
    );
    assert_eq!(pre.len(), 3);
    assert_eq!(installed["hooks"]["Stop"], config["hooks"]["Stop"]);
}

#[test]
fn malformed_user_config_is_never_replaced() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("hooks.json");
    fs::write(&path, "{ definitely not json").unwrap();

    let error = install_codex_hooks_to(temp.path()).unwrap_err();

    assert!(error.to_string().contains("Refusing to replace malformed"));
    assert_eq!(fs::read_to_string(path).unwrap(), "{ definitely not json");
}
