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
fn a_detected_repository_type_alone_recommends_nothing() {
    let fixture = Fixture::new();
    for codex in [false, true] {
        let output = fixture.run(codex, fixture.repo.path(), "finish this document");
        assert!(
            output.is_empty(),
            "detection alone should not qualify a skill: {output}"
        );
    }
}

#[test]
fn a_keyword_hit_plus_detection_qualifies_and_marks_the_repo_context() {
    let fixture = Fixture::new();
    for codex in [false, true] {
        let output = fixture.run(codex, fixture.repo.path(), "build a react feature");
        assert!(output.contains("loom-react"), "{output}");
        assert!(
            output.contains("loom-typescript"),
            "keyword hit (react) plus detection (typescript) should qualify: {output}"
        );
        assert!(output.contains("repo:typescript"), "{output}");
        assert!(
            !output.contains("loom-rust"),
            "detection alone (rust) must not qualify: {output}"
        );
    }
}

#[test]
fn codex_rendering_says_read_in_full_only_for_keyword_matches() {
    let fixture = Fixture::new();
    let output = fixture.run(true, fixture.repo.path(), "build a react feature");
    let react_line = output
        .lines()
        .find(|l| l.contains("loom-react"))
        .unwrap_or_else(|| panic!("missing loom-react line: {output}"));
    assert!(
        react_line.contains("in full"),
        "keyword-qualified skill should read in full: {react_line}"
    );
    let typescript_line = output
        .lines()
        .find(|l| l.contains("loom-typescript"))
        .unwrap_or_else(|| panic!("missing loom-typescript line: {output}"));
    assert!(
        typescript_line.contains("if the task touches typescript"),
        "keyword+detection skill should render conditionally: {typescript_line}"
    );
    assert!(
        !typescript_line.contains("in full"),
        "keyword+detection skill must not claim full read: {typescript_line}"
    );
}

#[test]
fn at_most_five_suggestions_are_rendered() {
    let fixture = Fixture::new();
    for codex in [false, true] {
        let root = fixture.agent_root(codex);
        let mut index = json!({
            "react": ["loom-react", "loom-typescript"],
            "typescript": ["loom-typescript"],
            "rust": ["loom-rust"],
        });
        let obj = index.as_object_mut().unwrap();
        for i in 0..5 {
            obj.insert(format!("alpha{i}"), json!([format!("loom-alpha{i}")]));
        }
        write(&root, "hooks/loom/skill-keywords.json", &index.to_string());
        for i in 0..5 {
            write(
                &root,
                &format!("loom-skill-catalog/loom-alpha{i}/SKILL.md"),
                &format!("---\nname: loom-alpha{i}\ndescription: loom-alpha{i} guidance\n---\n"),
            );
        }
        let prompt = "alpha0 alpha1 alpha2 alpha3 alpha4 react typescript";
        let output = fixture.run(codex, fixture.repo.path(), prompt);
        let skill_lines = output.lines().filter(|l| l.starts_with("  -")).count();
        assert_eq!(skill_lines, 5, "expected exactly 5 lines: {output}");
    }
}

#[test]
fn frontend_paths_and_nested_cwd_do_not_suggest_backend_skills() {
    let fixture = Fixture::new();
    for codex in [false, true] {
        for (cwd, prompt) in [
            (fixture.repo.path().join("web/src"), "build a react feature"),
            (
                fixture.repo.path().to_path_buf(),
                "build a react feature in web/src/new.tsx",
            ),
        ] {
            let output = fixture.run(codex, &cwd, prompt);
            assert!(output.contains("loom-react"), "{output}");
            assert!(output.contains("loom-typescript"), "{output}");
            assert!(!output.contains("loom-rust"), "{output}");
        }
    }
}

#[test]
fn a_missing_keyword_index_recommends_nothing() {
    // Without a generated index, `_score_keywords` has no entries to match
    // against, so no prompt can produce a keyword hit. Detection's tie-break
    // contribution alone can no longer cross MIN_SCORE (that promotion path
    // is exactly what this stage removed), so the missing-index case now
    // degrades to no suggestions rather than crashing or emitting malformed
    // output.
    let fixture = Fixture::new();
    for codex in [false, true] {
        fs::remove_file(
            fixture
                .agent_root(codex)
                .join("hooks/loom/skill-keywords.json"),
        )
        .unwrap();
        let output = fixture.run(codex, fixture.repo.path(), "please continue");
        assert!(
            output.is_empty(),
            "missing index should not crash or promote by detection alone: {output}"
        );
    }
}

#[test]
fn codex_uses_its_own_catalog_without_a_claude_installation() {
    let fixture = Fixture::new();
    fs::remove_dir_all(fixture.agent_root(false)).unwrap();
    let output = fixture.run(true, fixture.repo.path(), "rust");
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
        "build a react feature",
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
