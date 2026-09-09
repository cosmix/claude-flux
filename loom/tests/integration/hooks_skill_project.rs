//! Exercise the installed hook against the freshly built shared detector.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use loom::fs::permissions::constants::HOOK_SKILL_TRIGGER;
use serde_json::{json, Value};
use tempfile::TempDir;

struct Fixture {
    home: TempDir,
    repo: TempDir,
}

fn write(root: &Path, path: &str, content: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

impl Fixture {
    fn new() -> Self {
        let home = TempDir::new().unwrap();
        let repo = TempDir::new().unwrap();
        fs::create_dir(repo.path().join(".git")).unwrap();
        write(repo.path(), "backend/Cargo.toml", "[package]");
        write(
            repo.path(),
            "web/package.json",
            r#"{"dependencies":{"react":"19","typescript":"6"}}"#,
        );
        fs::create_dir(repo.path().join("web/src")).unwrap();
        for client in [".claude", ".codex"] {
            let root = home.path().join(client);
            write(&root, "hooks/loom/skill-trigger.sh", HOOK_SKILL_TRIGGER);
            write(
                &root,
                "hooks/loom/skill-keywords.json",
                r#"{"react":["loom-react","loom-typescript"],"typescript":["loom-typescript"],"rust":["loom-rust"]}"#,
            );
            for name in ["loom-react", "loom-typescript", "loom-rust"] {
                write(
                    &root,
                    &format!("loom-skill-catalog/{name}/SKILL.md"),
                    &format!("---\nname: {name}\ndescription: {name} guidance\n---\n"),
                );
            }
        }
        Self { home, repo }
    }

    fn agent_root(&self, codex: bool) -> PathBuf {
        self.home
            .path()
            .join(if codex { ".codex" } else { ".claude" })
    }

    fn run(&self, codex: bool, cwd: &Path, prompt: &str) -> String {
        let mut command = Command::new("python3");
        command
            .arg("-B")
            .arg(self.agent_root(codex).join("hooks/loom/skill-trigger.sh"));
        if codex {
            command.arg("--codex");
        }
        let mut child = command
            .env("HOME", self.home.path())
            .env("CODEX_HOME", self.agent_root(true))
            .env("LOOM_BIN", env!("CARGO_BIN_EXE_loom"))
            .env_remove("LOOM_SKILL_DEBUG")
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("python3 must be available for skill hooks");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(json!({"cwd": cwd, "prompt": prompt}).to_string().as_bytes())
            .unwrap();
        read_context(child.wait_with_output().unwrap())
    }
}

fn read_context(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    if output.stdout.is_empty() {
        return String::new();
    }
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value["hookSpecificOutput"]["hookEventName"],
        "UserPromptSubmit"
    );
    value["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .into()
}

#[test]
fn both_clients_suggest_all_detected_types_without_prompt_keywords() {
    let fixture = Fixture::new();
    for codex in [false, true] {
        let output = fixture.run(codex, fixture.repo.path(), "please continue");
        for name in ["loom-rust", "loom-react", "loom-typescript"] {
            assert!(output.contains(name), "missing {name}: {output}");
        }
        assert!(output.contains("repo:rust (backend)"));
        if codex {
            assert!(output.contains(".codex/loom-skill-catalog/loom-react/SKILL.md"));
            assert!(!output.contains("Skill("));
            assert!(!output.contains(".claude"));
        } else {
            assert!(output.contains("Skill(skill=\"loom-skills\""));
        }
    }
}

#[test]
fn frontend_paths_and_nested_cwd_do_not_suggest_backend_skills() {
    let fixture = Fixture::new();
    for codex in [false, true] {
        for (cwd, prompt) in [
            (fixture.repo.path().join("web/src"), "please continue"),
            (fixture.repo.path().to_path_buf(), "adjust web/src/new.tsx"),
        ] {
            let output = fixture.run(codex, &cwd, prompt);
            assert!(output.contains("loom-react"));
            assert!(output.contains("loom-typescript"));
            assert!(!output.contains("loom-rust"), "{output}");
        }
    }
}

#[test]
fn detected_skills_do_not_require_a_generated_keyword_index() {
    let fixture = Fixture::new();
    for codex in [false, true] {
        fs::remove_file(
            fixture
                .agent_root(codex)
                .join("hooks/loom/skill-keywords.json"),
        )
        .unwrap();
        let output = fixture.run(codex, fixture.repo.path(), "please continue");
        assert!(output.contains("loom-react"));
        assert!(output.contains("loom-rust"));
    }
}

#[test]
fn codex_uses_its_own_catalog_without_a_claude_installation() {
    let fixture = Fixture::new();
    fs::remove_dir_all(fixture.agent_root(false)).unwrap();
    let output = fixture.run(true, fixture.repo.path(), "please continue");
    assert!(output.contains(".codex/loom-skill-catalog/loom-rust/SKILL.md"));
    assert!(!output.contains(".claude"));
}

#[test]
fn codex_prefers_project_native_skill_over_the_catalog_copy() {
    let fixture = Fixture::new();
    write(
        fixture.repo.path(),
        ".agents/skills/loom-react/SKILL.md",
        "---\nname: loom-react\ndescription: local React guidance\n---\n",
    );
    let output = fixture.run(
        true,
        &fixture.repo.path().join("web/src"),
        "please continue",
    );
    assert!(output.contains(".agents/skills/loom-react/SKILL.md"));
    assert!(!output.contains(".codex/loom-skill-catalog/loom-react/SKILL.md"));
}

#[test]
fn skill_index_command_refreshes_both_clients_without_mixing_their_keywords() {
    let fixture = Fixture::new();
    for (codex, keyword) in [(false, "claude-only"), (true, "codex-only")] {
        write(
            &fixture.agent_root(codex),
            "loom-skill-catalog/loom-rust/SKILL.md",
            &format!("---\nname: loom-rust\ndescription: Rust\ntriggers: [{keyword}]\n---\n"),
        );
    }
    let output = Command::new(env!("CARGO_BIN_EXE_loom"))
        .arg("skill-index")
        .env("HOME", fixture.home.path())
        .env("CODEX_HOME", fixture.agent_root(true))
        .current_dir(fixture.repo.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for (codex, own, other) in [
        (false, "claude-only", "codex-only"),
        (true, "codex-only", "claude-only"),
    ] {
        let index: Value = serde_json::from_slice(
            &fs::read(
                fixture
                    .agent_root(codex)
                    .join("hooks/loom/skill-keywords.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(index[own], json!(["loom-rust"]));
        assert!(index.get(other).is_none());
    }
}
