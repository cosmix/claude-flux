//! Codex-native installation and configuration for Loom's hook assets.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use tempfile::NamedTempFile;

use super::hooks::install_loom_hooks_to;

const BASH_HOOKS: &[&str] = &[
    "prefer-modern-tools.sh",
    "commit-filter.sh",
    "subagent-verify-guard.sh",
    "git-add-guard.sh",
    "worktree-isolation.sh",
    "no-preexisting-failures.sh",
    "poll-guard.sh",
];

/// Install Codex hook scripts and merge their registrations into hooks.json.
pub fn install_codex_hooks() -> Result<usize> {
    let home = dirs::home_dir().context("Failed to determine home directory")?;
    install_codex_hooks_to(&home.join(".codex"))
}

/// Testable counterpart to [`install_codex_hooks`] for an explicit Codex root.
pub fn install_codex_hooks_to(codex_dir: &Path) -> Result<usize> {
    let hooks_dir = codex_dir.join("hooks/loom");
    let installed = install_loom_hooks_to(&hooks_dir)?;
    merge_hooks_file(&codex_dir.join("hooks.json"), &hooks_dir)?;
    Ok(installed)
}

/// Whether the default Codex tree is missing current scripts or registrations.
pub fn codex_hooks_need_install() -> bool {
    dirs::home_dir().is_some_and(|home| codex_hooks_need_install_in(&home.join(".codex")))
}

/// Testable drift check for an explicit Codex configuration root.
pub fn codex_hooks_need_install_in(codex_dir: &Path) -> bool {
    let hooks_dir = codex_dir.join("hooks/loom");
    if !super::drift::hook_scripts_needing_install(&hooks_dir).is_empty() {
        return true;
    }
    let Ok(mut root) = read_root(&codex_dir.join("hooks.json")) else {
        return true;
    };
    let before = root.clone();
    merge_hooks(&mut root, &hooks_dir).is_err() || root != before
}

fn merge_hooks_file(config_path: &Path, hooks_dir: &Path) -> Result<()> {
    let mut root = read_root(config_path)?;
    merge_hooks(&mut root, hooks_dir)?;
    write_root(config_path, &root)
}

fn read_root(path: &Path) -> Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let bytes = fs::read(path).with_context(|| format!("Failed to read {}", path.display()))?;
    let value: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("Refusing to replace malformed {}", path.display()))?;
    value
        .as_object()
        .cloned()
        .with_context(|| format!("{} must contain a JSON object", path.display()))
}

fn merge_hooks(root: &mut Map<String, Value>, hooks_dir: &Path) -> Result<()> {
    let desired = desired_hooks(hooks_dir);
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks
        .as_object_mut()
        .context("hooks.json field 'hooks' must be a JSON object")?;

    let events: BTreeSet<String> = hooks.keys().chain(desired.keys()).cloned().collect();
    for event in events {
        let rules = hooks.entry(&event).or_insert_with(|| json!([]));
        let rules = rules
            .as_array_mut()
            .with_context(|| format!("hooks.{event} must be a JSON array"))?;
        rules.retain(|rule| !is_loom_rule(rule, hooks_dir));
        if let Some(fresh) = desired.get(&event).and_then(Value::as_array) {
            rules.extend(fresh.iter().cloned());
        }
    }
    Ok(())
}

fn desired_hooks(hooks_dir: &Path) -> Map<String, Value> {
    let bash_handlers = BASH_HOOKS
        .iter()
        .map(|script| handler(hooks_dir, script, None))
        .collect();
    let apply_patch_handlers = vec![
        handler(hooks_dir, "codex-apply-patch.sh", Some("pre")),
        handler(hooks_dir, "no-preexisting-failures.sh", None),
    ];

    Map::from_iter([
        (
            "SessionStart".to_string(),
            json!([rule(
                Some("^(startup|clear)$"),
                vec![handler(hooks_dir, "knowledge-orient.sh", None)]
            )]),
        ),
        (
            "UserPromptSubmit".to_string(),
            json!([rule(
                None,
                vec![handler(hooks_dir, "user-prompt-context.sh", None)]
            )]),
        ),
        (
            "PreToolUse".to_string(),
            json!([
                rule(Some("^Bash$"), bash_handlers),
                rule(Some("^apply_patch$"), apply_patch_handlers)
            ]),
        ),
        (
            "PostToolUse".to_string(),
            json!([rule(
                Some("^apply_patch$"),
                vec![handler(hooks_dir, "codex-apply-patch.sh", Some("post"))]
            )]),
        ),
    ])
}

fn rule(matcher: Option<&str>, handlers: Vec<Value>) -> Value {
    let mut rule = Map::new();
    if let Some(matcher) = matcher {
        rule.insert("matcher".to_string(), json!(matcher));
    }
    rule.insert("hooks".to_string(), Value::Array(handlers));
    Value::Object(rule)
}

fn handler(hooks_dir: &Path, script: &str, argument: Option<&str>) -> Value {
    let path = hooks_dir.join(script);
    let mut command = shell_escape::escape(path.to_string_lossy()).into_owned();
    if let Some(argument) = argument {
        command.push(' ');
        command.push_str(argument);
    }
    json!({"type": "command", "command": command})
}

fn is_loom_rule(rule: &Value, hooks_dir: &Path) -> bool {
    let prefix = hooks_dir.to_string_lossy();
    rule.get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|handlers| {
            handlers.iter().any(|handler| {
                handler
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|command| command.contains(prefix.as_ref()))
            })
        })
}

fn write_root(path: &Path, root: &Map<String, Value>) -> Result<()> {
    let parent = path.parent().context("Codex hooks path has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("Failed to create {}", parent.display()))?;
    let mut body = serde_json::to_string_pretty(root)?;
    body.push('\n');

    let mut temp = NamedTempFile::new_in(parent)
        .with_context(|| format!("Failed to create temporary file in {}", parent.display()))?;
    temp.write_all(body.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
#[path = "codex_hooks_tests.rs"]
mod tests;
