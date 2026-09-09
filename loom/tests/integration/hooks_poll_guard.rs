//! Integration tests for the poll-guard PreToolUse:Bash hook.
//!
//! poll-guard.sh discourages wasted turns (CLAUDE.md rule 14, rule 6): a
//! long `sleep`, a read-only command line repeated past reason, a Bash-side
//! cat/head/tail/sed full-file read (reusing read-guard.sh's own rules 1-3
//! via hooks/_read_discipline.sh, so the two hooks can never drift apart),
//! and a pathless `git show`/`git diff`. The repeated-command escalation and
//! the shared read-discipline rules are both gated by the same `[hooks]
//! deny_enabled` switch as read-guard.sh; these tests exercise the
//! escalating rules with the switch both ways. Build/test/lint runners
//! (`cargo`, `npm`, `make`, ...) are exempt from the repeat-command rule
//! outright - the acceptance loop is SUPPOSED to rerun them - and that
//! exemption is pinned here explicitly, since a regression there would
//! start denying every retried `cargo test`.
//!
//! Runs the hook script directly with bash - no loom invocation.
//!
//! The Bash-side cat/sed/head/tail read cases live in `hooks_poll_guard_reads.rs`, the
//! pathless-vs-scoped `git show`/`git diff` cases in `hooks_poll_guard_git.rs`, and the
//! verify-runner exemption, the `echo`-is-never-counted case, and the live-session deny gate
//! regression in `hooks_poll_guard_gate.rs` (see each file's module docs) - all split out purely
//! for size, sharing this file's harness via `use super::*`.

use loom::fs::permissions::constants::{
    HOOK_COMMON, HOOK_POLL_GUARD, HOOK_READ_DISCIPLINE, HOOK_READ_GUARD, HOOK_READ_LEDGER,
};
use loom::process::sandbox_probe::{process_tree_visible, skip_unless};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::TempDir;

struct HookOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

fn write_exec(path: &Path, content: &str) {
    fs::write(path, content).expect("write file");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
}

/// Install poll-guard.sh plus its three sourced dependencies into a temp dir.
fn setup_hook() -> (TempDir, PathBuf) {
    let temp = TempDir::new().expect("create temp dir");
    write_exec(&temp.path().join("_common.sh"), HOOK_COMMON);
    write_exec(
        &temp.path().join("_read_discipline.sh"),
        HOOK_READ_DISCIPLINE,
    );
    write_exec(&temp.path().join("_read_ledger.sh"), HOOK_READ_LEDGER);
    let hook_path = temp.path().join("poll-guard.sh");
    write_exec(&hook_path, HOOK_POLL_GUARD);
    (temp, hook_path)
}

/// Additionally install read-guard.sh alongside poll-guard.sh, for the one
/// test proving both hooks share the same read ledger.
fn setup_both_hooks() -> (TempDir, PathBuf, PathBuf) {
    let (temp, poll_hook) = setup_hook();
    let read_hook = temp.path().join("read-guard.sh");
    write_exec(&read_hook, HOOK_READ_GUARD);
    (temp, poll_hook, read_hook)
}

/// Write a file with exactly `lines` lines (by `wc -l`) under `dir`.
fn write_file_with_lines(dir: &Path, name: &str, lines: usize) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, "line\n".repeat(lines)).expect("write test file");
    path
}

/// A "covered" `loom map --outline` stub: two symbol rows plus
/// `coverage: full`, so rule 1 (reused from read-guard.sh) treats the file
/// as graph-covered.
fn covered_stub_dir(root: &Path) -> PathBuf {
    let dir = root.join("stub-covered");
    fs::create_dir_all(&dir).expect("create stub dir");
    let body = "-> Outline: stub\n\tL14-L14\tconstant\tPOLL_INTERVAL\tconst POLL_INTERVAL: ...\n\tL40-L61\tfunction\tgather\tfn gather(\ncoverage: full";
    let script = format!("#!/usr/bin/env bash\ncat <<'EOF'\n{body}\nEOF\n");
    write_exec(&dir.join("loom"), &script);
    dir
}

/// One test's isolated loom session: its own `.work`-shaped dir so every
/// ledger and config.toml lands inside a TempDir, never the developer's real
/// `/tmp` or `~/.claude`.
struct Session {
    work: TempDir,
    session_id: &'static str,
    stage_id: &'static str,
    agent_id: &'static str,
    /// `LOOM_MAIN_AGENT_PID` to export, if any - `loom_hook_deny_or_warn`
    /// (`_read_discipline.sh`) only ever denies when this is set AND a live
    /// ancestor of the hook's bash process. `None` (the default) leaves a
    /// would-be deny as a warning, matching an ordinary session with no loom
    /// orchestrator above it.
    main_agent_pid: Option<String>,
}

impl Session {
    fn new() -> Self {
        Session {
            work: TempDir::new().expect("create work dir"),
            session_id: "sess-1",
            stage_id: "stage-1",
            agent_id: "agent-1",
            main_agent_pid: None,
        }
    }

    fn work_dir(&self) -> &Path {
        self.work.path()
    }

    /// Turn on the `[hooks] deny_enabled = true` switch for this session.
    fn enable_deny(&self) {
        fs::write(
            self.work_dir().join("config.toml"),
            "[hooks]\ndeny_enabled = true\n",
        )
        .expect("write config.toml");
    }

    /// Mark this session as running under a LIVE loom main-agent process:
    /// this test binary's own pid, a direct ancestor of the hook's bash
    /// child. Required, together with `enable_deny`, for a deny branch to
    /// actually deny rather than degrade to a warning.
    fn with_live_main_agent(mut self) -> Self {
        self.main_agent_pid = Some(std::process::id().to_string());
        self
    }

    /// Mark this session with an explicit (possibly non-ancestor)
    /// `LOOM_MAIN_AGENT_PID`, e.g. `"1"` - proves the liveness check, not
    /// just presence of the variable, gates a deny.
    fn with_main_agent_pid(mut self, pid: &str) -> Self {
        self.main_agent_pid = Some(pid.to_string());
        self
    }
}

/// Run a hook against a raw `payload`, with `stub_dir` (if any) placed first
/// on PATH so `loom map --outline` resolves to the fake binary.
fn run_payload(
    hook: &Path,
    payload: &Value,
    session: &Session,
    stub_dir: Option<&Path>,
) -> HookOutput {
    let payload_str = payload.to_string();
    let path_value = match stub_dir {
        Some(dir) => format!(
            "{}:{}",
            dir.display(),
            std::env::var("PATH").unwrap_or_default()
        ),
        None => std::env::var("PATH").unwrap_or_default(),
    };

    let mut cmd = Command::new("bash");
    cmd.arg(hook)
        .env("PATH", path_value)
        .env("LOOM_WORK_DIR", session.work_dir())
        .env("LOOM_SESSION_ID", session.session_id)
        .env("LOOM_STAGE_ID", session.stage_id)
        // Never let the developer's own shell env leak `loom_debug` output
        // onto stderr - it would fail assertions for reasons unrelated to
        // the hook's actual decision.
        .env_remove("LOOM_HOOK_DEBUG")
        .env_remove("COMMIT_FILTER_DEBUG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(pid) = &session.main_agent_pid {
        cmd.env("LOOM_MAIN_AGENT_PID", pid);
    }

    let mut child = cmd.spawn().expect("spawn hook");
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(payload_str.as_bytes()).ok();
    }
    let output = child.wait_with_output().expect("wait for hook");
    HookOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Convenience wrapper over `run_payload` for a `Bash` tool call.
fn run_bash_hook(
    hook: &Path,
    command: &str,
    session: &Session,
    stub_dir: Option<&Path>,
) -> HookOutput {
    let payload = json!({
        "tool_name": "Bash",
        "tool_input": {"command": command},
        "agent_id": session.agent_id,
        "session_id": session.session_id,
    });
    run_payload(hook, &payload, session, stub_dir)
}

/// Every deny-branch test below needs the SAME probe: whether this sandbox
/// can see its own process tree, which `is_ancestor` (`hooks/_common.sh`)
/// depends on. `test` is the test's path under this file (a bare name, or
/// `submodule::name`); this adds the `hooks_poll_guard::` prefix so the
/// printed SKIP line matches `cargo test`'s own naming.
fn skip_unless_gate_visible(test: &str) -> bool {
    skip_unless(
        process_tree_visible(),
        &format!("hooks_poll_guard::{test}"),
        "the guard's deny branch needs a visible process tree",
    )
}

/// Extract the single `LOOM_HOOK_WARN: ...` additionalContext string from a
/// warn response's stdout JSON.
fn warn_context(stdout: &str) -> String {
    let v: Value = serde_json::from_str(stdout.trim()).expect("parse stdout json");
    v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("additionalContext present")
        .to_string()
}

// 1. `sleep N >= 30s` warns and names `loom subagents watch`; short sleeps do
//    not - the threshold is `>=`, tested at its boundary.
#[test]
fn long_sleep_warns_short_sleep_does_not() {
    let (_hook_dir, hook) = setup_hook();
    let session = Session::new();

    let out = run_bash_hook(&hook, "sleep 300", &session, None);
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    assert!(
        warn_context(&out.stdout).contains("loom subagents watch"),
        "stdout={}",
        out.stdout
    );

    let out = run_bash_hook(&hook, "sleep 5", &session, None);
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    assert!(out.stdout.trim().is_empty(), "stdout={}", out.stdout);

    let out = run_bash_hook(&hook, "sleep 29", &session, None);
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    assert!(
        out.stdout.trim().is_empty(),
        "sleep 29 must not warn: {}",
        out.stdout
    );

    let out = run_bash_hook(&hook, "sleep 30", &session, None);
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    assert!(
        !out.stdout.trim().is_empty(),
        "sleep 30 must warn: {}",
        out.stdout
    );
}

// 2. Build/test/lint runner exemption and the "echo is never counted" case
// live in `hooks_poll_guard_gate.rs`, split out purely for size, alongside
// the live-session deny gate regression - see its module docs.
#[path = "hooks_poll_guard_gate.rs"]
mod gate;

// 3. A repeated read-only poll (`git status`) escalates: clean at 1-2, warns
//    at 3-4, denies at 5 (switch on) / warns at 5 (switch off).
#[test]
fn repeated_git_status_escalates_to_deny_only_with_switch_on() {
    if skip_unless_gate_visible("repeated_git_status_escalates_to_deny_only_with_switch_on") {
        return;
    }
    let (_hook_dir, hook) = setup_hook();
    let on = Session::new().with_live_main_agent();
    on.enable_deny();
    for n in 1..=2 {
        let out = run_bash_hook(&hook, "git status", &on, None);
        assert_eq!(out.code, 0, "run {n}: stderr={}", out.stderr);
        assert!(
            out.stdout.trim().is_empty(),
            "run {n} must be clean: {}",
            out.stdout
        );
    }
    for n in 3..=4 {
        let out = run_bash_hook(&hook, "git status", &on, None);
        assert_eq!(out.code, 0, "run {n}: stderr={}", out.stderr);
        assert!(
            warn_context(&out.stdout).contains(&format!("run {n} times")),
            "run {n}: {}",
            out.stdout
        );
    }
    let out = run_bash_hook(&hook, "git status", &on, None);
    assert_eq!(
        out.code, 2,
        "5th run must deny with the switch on: stderr={}",
        out.stderr
    );
    assert!(out.stderr.contains("run 5 times"), "stderr={}", out.stderr);

    let off = Session::new();
    for _ in 1..=4 {
        run_bash_hook(&hook, "git status", &off, None);
    }
    let out = run_bash_hook(&hook, "git status", &off, None);
    assert_eq!(
        out.code, 0,
        "switch off must never deny: stderr={}",
        out.stderr
    );
    assert!(
        warn_context(&out.stdout).contains("run 5 times"),
        "stdout={}",
        out.stdout
    );
}

// 4, 4b and 5. The Bash-side cat/sed/head/tail read cases (rule 1 reuse,
// the shared read ledger, and the head/tail bounded-vs-bare distinction)
// live in `hooks_poll_guard_reads.rs`, split out purely for size - see its
// module docs before editing one. It shares this file's harness via
// `use super::*`.
#[path = "hooks_poll_guard_reads.rs"]
mod reads;

#[path = "hooks_poll_guard_skills.rs"]
mod skills;

// 6. Pathless vs. path-scoped `git show`/`git diff` (rule 4) live in
// `hooks_poll_guard_git.rs`, split out purely for size - see its module
// docs.
#[path = "hooks_poll_guard_git.rs"]
mod git;

// 7. A non-Bash tool exits 0 silently; a command the hook has no rule for
//    also exits 0 with no output.
#[test]
fn non_bash_tool_and_unmatched_command_are_silently_ignored() {
    let (_hook_dir, hook) = setup_hook();
    let session = Session::new();

    let wrong_tool = json!({
        "tool_name": "Read",
        "tool_input": {"file_path": "/tmp/whatever"},
        "agent_id": session.agent_id,
    });
    let out = run_payload(&hook, &wrong_tool, &session, None);
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    assert!(
        out.stdout.trim().is_empty() && out.stderr.trim().is_empty(),
        "stdout={} stderr={}",
        out.stdout,
        out.stderr
    );

    let out = run_bash_hook(&hook, "rg -n foo src/", &session, None);
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    assert!(out.stdout.trim().is_empty(), "stdout={}", out.stdout);
}

// Direct check of the polls ledger's TSV row shape: key, timestamp - the
// format the repeat-command counter actually depends on.
#[test]
fn polls_ledger_row_is_tab_separated_key_timestamp() {
    let (_hook_dir, hook) = setup_hook();
    let session = Session::new();

    let out = run_bash_hook(&hook, "git status", &session, None);
    assert_eq!(out.code, 0, "stderr={}", out.stderr);

    let ledger = session.work_dir().join(format!(
        "hooks/polls/{}/{}.tsv",
        session.session_id, session.agent_id
    ));
    let content = fs::read_to_string(&ledger).unwrap_or_else(|e| panic!("read ledger: {e}"));
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 1, "content={content:?}");
    let fields: Vec<&str> = lines[0].split('\t').collect();
    assert_eq!(fields.len(), 2, "row={fields:?}");
    assert_eq!(fields[0], "git status");
    assert!(!fields[1].is_empty());
}
