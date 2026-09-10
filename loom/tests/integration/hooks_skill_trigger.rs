//! Integration tests for the skill-trigger UserPromptSubmit hook.
//!
//! `hooks/skill-trigger.sh` scores skills from an inverted keyword index and
//! prints a `hookSpecificOutput.additionalContext` block suggesting them.
//! These tests cover the defects fixed alongside this file: every qualifying
//! skill is listed (not just the top 3), ranking is deterministic across
//! `PYTHONHASHSEED` values, the `loom-skills` loader line is dropped once a
//! domain skill already qualifies, and a combined `loom-skills` loader call
//! is appended when two or more catalogued skills qualify.
//!
//! Runs the Python hook with the freshly built Loom detector. HOME points to
//! a per-test fixture directory so installed client data is never touched.

use loom::fs::permissions::constants::HOOK_SKILL_TRIGGER;
use loom::process::sandbox_probe::skip_unless;
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use tempfile::TempDir;

struct HookOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

/// A fake `HOME`, populated with a skill-keywords index plus core and
/// catalogued SKILL.md fixtures, and a separate empty cwd so project discovery
/// never contributes ambient signals that would skew keyword-scoring tests.
struct FakeHome {
    home: TempDir,
    cwd: TempDir,
}

fn write_exec(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().expect("hook path has a parent")).expect("create hook dir");
    fs::write(path, content).expect("write file");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
}

fn write_skill_md(path: &Path, name: &str) {
    fs::create_dir_all(path.parent().expect("SKILL.md path has a parent"))
        .expect("create skill dir");
    fs::write(
        path,
        format!("---\nname: {name}\ndescription: {name} desc.\n---\n"),
    )
    .expect("write SKILL.md");
}

impl FakeHome {
    /// Build the fixture home shared by every test below: the base index
    /// keywords from the brief plus ten `alpha0`..`alpha9` entries for the
    /// flood-cap test, two core skills, and the catalogued domain skills.
    fn new() -> Self {
        let home = TempDir::new().expect("create fake HOME");
        let cwd = TempDir::new().expect("create fake cwd");

        let mut index = json!({
            "react": ["loom-react", "loom-typescript"],
            "typescript": ["loom-typescript"],
            "docker": ["loom-docker"],
            "skill": ["loom-skills"],
            "skills": ["loom-skills"],
            "plan": ["loom-plan-writer"],
        });
        let index_obj = index.as_object_mut().expect("index is an object");
        for i in 0..10 {
            index_obj.insert(format!("alpha{i}"), json!([format!("loom-alpha{i}")]));
        }
        let index_path = home.path().join(".claude/hooks/loom/skill-keywords.json");
        fs::create_dir_all(index_path.parent().expect("index path has a parent"))
            .expect("create index dir");
        fs::write(&index_path, index.to_string()).expect("write index");

        for name in ["loom-skills", "loom-plan-writer"] {
            write_skill_md(
                &home
                    .path()
                    .join(".claude/skills")
                    .join(name)
                    .join("SKILL.md"),
                name,
            );
        }
        let mut catalogued: Vec<String> = vec![
            "loom-react".into(),
            "loom-typescript".into(),
            "loom-docker".into(),
        ];
        catalogued.extend((0..10).map(|i| format!("loom-alpha{i}")));
        for name in &catalogued {
            write_skill_md(
                &home
                    .path()
                    .join(".claude/loom-skill-catalog")
                    .join(name)
                    .join("SKILL.md"),
                name,
            );
        }

        FakeHome { home, cwd }
    }

    fn home_path(&self) -> &Path {
        self.home.path()
    }

    fn cwd_path(&self) -> &Path {
        self.cwd.path()
    }
}

/// Whether `python3` can be spawned in this sandbox - unrelated to the hook
/// logic under test, so a denial skips rather than fails.
fn python3_available() -> bool {
    static RESULT: OnceLock<bool> = OnceLock::new();
    *RESULT.get_or_init(|| {
        Command::new("python3")
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    })
}

fn install_hook() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("create hook dir");
    let path = dir.path().join("skill-trigger.sh");
    write_exec(&path, HOOK_SKILL_TRIGGER);
    (dir, path)
}

fn run_hook(hook: &Path, home: &FakeHome, prompt: &str, hash_seed: Option<&str>) -> HookOutput {
    let payload = json!({
        "session_id": "t",
        "cwd": home.cwd_path().display().to_string(),
        "prompt": prompt,
    });

    let mut cmd = Command::new("python3");
    cmd.arg(hook)
        .env("HOME", home.home_path())
        .env("LOOM_BIN", env!("CARGO_BIN_EXE_loom"))
        .env_remove("LOOM_SKILL_DEBUG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match hash_seed {
        Some(seed) => {
            cmd.env("PYTHONHASHSEED", seed);
        }
        None => {
            cmd.env_remove("PYTHONHASHSEED");
        }
    }

    let mut child = cmd.spawn().expect("spawn python3 skill-trigger.sh");
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(payload.to_string().as_bytes()).ok();
    }
    let output = child.wait_with_output().expect("wait for hook");
    HookOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Extract `hookSpecificOutput.additionalContext` from a hook's stdout JSON.
fn additional_context(stdout: &str) -> String {
    let v: Value = serde_json::from_str(stdout.trim()).expect("parse stdout json");
    v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("additionalContext present")
        .to_string()
}

fn skip_unless_python3(test_name: &str) -> bool {
    skip_unless(
        python3_available(),
        test_name,
        "python3 must be spawnable to run the skill-trigger hook",
    )
}

#[test]
fn every_qualifying_skill_is_listed() {
    if skip_unless_python3("hooks_skill_trigger::every_qualifying_skill_is_listed") {
        return;
    }
    let home = FakeHome::new();
    let (_hook_dir, hook) = install_hook();

    let out = run_hook(
        &hook,
        &home,
        "build a react form in typescript and ship it in docker",
        None,
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    let ctx = additional_context(&out.stdout);

    let ts_pos = ctx
        .find("- loom-typescript --")
        .unwrap_or_else(|| panic!("missing loom-typescript line: {ctx}"));
    let docker_pos = ctx
        .find("- loom-docker --")
        .unwrap_or_else(|| panic!("missing loom-docker line: {ctx}"));
    let react_pos = ctx
        .find("- loom-react --")
        .unwrap_or_else(|| panic!("missing loom-react line: {ctx}"));
    assert!(
        ts_pos < docker_pos && docker_pos < react_pos,
        "unexpected line order: {ctx}"
    );

    let combined = ctx
        .lines()
        .find(|l| l.contains("All catalogued matches at once"))
        .unwrap_or_else(|| panic!("missing combined line: {ctx}"));
    assert!(
        combined.contains("args=\"loom-typescript loom-docker loom-react\""),
        "combined line has wrong names/order: {combined}"
    );
}

#[test]
fn ranking_is_deterministic_across_hash_seeds() {
    if skip_unless_python3("hooks_skill_trigger::ranking_is_deterministic_across_hash_seeds") {
        return;
    }
    let home = FakeHome::new();
    let (_hook_dir, hook) = install_hook();
    let prompt = "build a react form in typescript and ship it in docker";

    let mut outputs = Vec::new();
    for seed in ["0", "1", "2", "3"] {
        let out = run_hook(&hook, &home, prompt, Some(seed));
        assert_eq!(out.code, 0, "seed {seed}: stderr={}", out.stderr);
        outputs.push((seed, out.stdout));
    }
    let (first_seed, first_stdout) = &outputs[0];
    for (seed, stdout) in &outputs[1..] {
        assert_eq!(
            first_stdout, stdout,
            "seed {seed} output differs from seed {first_seed}"
        );
    }
}

#[test]
fn loader_line_dropped_when_domain_skills_qualify() {
    if skip_unless_python3("hooks_skill_trigger::loader_line_dropped_when_domain_skills_qualify") {
        return;
    }
    let home = FakeHome::new();
    let (_hook_dir, hook) = install_hook();

    let out = run_hook(&hook, &home, "skills for react", None);
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    let ctx = additional_context(&out.stdout);
    assert!(
        !ctx.contains("/loom-skills"),
        "loom-skills loader line should be dropped once react qualifies: {ctx}"
    );

    let out = run_hook(&hook, &home, "list the skills", None);
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    let ctx = additional_context(&out.stdout);
    assert!(
        ctx.contains("/loom-skills"),
        "loom-skills loader line should be kept as the only match: {ctx}"
    );
    assert!(
        !ctx.contains("All catalogued matches"),
        "a single core-skill match should have no combined line: {ctx}"
    );
}

#[test]
fn single_catalogued_match_has_no_combined_line() {
    if skip_unless_python3("hooks_skill_trigger::single_catalogued_match_has_no_combined_line") {
        return;
    }
    let home = FakeHome::new();
    let (_hook_dir, hook) = install_hook();

    let out = run_hook(&hook, &home, "docker", None);
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    let ctx = additional_context(&out.stdout);
    assert!(
        ctx.contains("loom-docker"),
        "expected loom-docker line: {ctx}"
    );
    assert!(
        !ctx.contains("loom-react") && !ctx.contains("loom-typescript"),
        "unexpected extra skill matched: {ctx}"
    );
    assert!(
        !ctx.contains("All catalogued matches"),
        "a single catalogued match should have no combined line: {ctx}"
    );
}

#[test]
fn core_skill_is_never_in_combined_args() {
    if skip_unless_python3("hooks_skill_trigger::core_skill_is_never_in_combined_args") {
        return;
    }
    let home = FakeHome::new();
    let (_hook_dir, hook) = install_hook();

    let out = run_hook(&hook, &home, "plan a react app in typescript", None);
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    let ctx = additional_context(&out.stdout);
    assert!(
        ctx.contains("/loom-plan-writer"),
        "expected core skill line: {ctx}"
    );
    let combined = ctx
        .lines()
        .find(|l| l.contains("All catalogued matches at once"))
        .unwrap_or_else(|| panic!("missing combined line: {ctx}"));
    assert!(combined.contains("loom-react"), "combined line: {combined}");
    assert!(
        combined.contains("loom-typescript"),
        "combined line: {combined}"
    );
    assert!(
        !combined.contains("loom-plan-writer"),
        "core skill leaked into combined args: {combined}"
    );
}

#[test]
fn cap_limits_flood() {
    if skip_unless_python3("hooks_skill_trigger::cap_limits_flood") {
        return;
    }
    let home = FakeHome::new();
    let (_hook_dir, hook) = install_hook();

    let prompt = "alpha0 alpha1 alpha2 alpha3 alpha4 alpha5 alpha6 alpha7 alpha8 alpha9";
    let out = run_hook(&hook, &home, prompt, None);
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    let ctx = additional_context(&out.stdout);

    let skill_lines: Vec<&str> = ctx.lines().filter(|l| l.starts_with("  -")).collect();
    assert_eq!(skill_lines.len(), 5, "expected exactly 5 lines: {ctx}");

    let combined = ctx
        .lines()
        .find(|l| l.contains("All catalogued matches at once"))
        .unwrap_or_else(|| panic!("missing combined line: {ctx}"));
    let args = combined
        .split("args=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_else(|| panic!("missing args in combined line: {combined}"));
    assert_eq!(
        args.split_whitespace().count(),
        5,
        "expected 5 names in combined args: {combined}"
    );
}
