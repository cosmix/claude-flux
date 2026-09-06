//! Repeat-read escalation cases, binary/PDF extension exemptions, the tier-1 knowledge override,
//! the live-session deny gate, plus the small no-op and ledger-shape checks, for
//! `hooks_read_guard.rs`.
//!
//! Split out purely for size: shrinking the escalation tests down to the function-length ceiling
//! still left the parent file over the 400-line file cap, so they - and a few tests that didn't
//! fit the parent's rule-1 narrative - live here instead, sharing the parent's harness (hook
//! installation, `Session`, `run_read_hook`, `warn_context`) via `use super::*` - read the
//! parent's module docs first.

use super::*;

/// One repeated-full-read call: run the hook, assert the exit code, then hand the output to
/// `check` for the call-specific message assertion.
fn expect_full_read(
    hook: &Path,
    tool_input: &Value,
    session: &Session,
    expected_code: i32,
    label: &str,
    check: impl FnOnce(&HookOutput),
) {
    let out = run_read_hook(hook, tool_input.clone(), session, None);
    assert_eq!(out.code, expected_code, "{label}: stderr={}", out.stderr);
    check(&out);
}

// 6. Repeat full reads of the same (small, so rule 1 never fires) file
//    escalate: 1st clean, 2nd warns, 3rd denies (switch on) / warns (off).
#[test]
fn repeat_full_reads_escalate_to_deny_only_with_switch_on() {
    if skip_unless_gate_visible("repeat::repeat_full_reads_escalate_to_deny_only_with_switch_on") {
        return;
    }
    let (_hook_dir, hook) = setup_hook();
    let files = TempDir::new().expect("files dir");
    let file = write_file_with_lines(files.path(), "small.rs", 50);
    let tool_input = json!({"file_path": file.to_string_lossy()});

    let on = Session::new().with_live_main_agent();
    on.enable_deny();
    expect_full_read(&hook, &tool_input, &on, 0, "1st read", |out| {
        assert!(
            out.stdout.trim().is_empty(),
            "1st read must be clean: {}",
            out.stdout
        );
    });
    expect_full_read(&hook, &tool_input, &on, 0, "2nd read", |out| {
        assert!(
            warn_context(&out.stdout).contains("read in full at"),
            "stdout={}",
            out.stdout
        );
    });
    expect_full_read(&hook, &tool_input, &on, 2, "3rd read", |out| {
        assert!(
            out.stderr.contains("read in full") && out.stderr.contains("2 times"),
            "stderr={}",
            out.stderr
        );
    });

    let off = Session::new();
    run_read_hook(&hook, tool_input.clone(), &off, None);
    run_read_hook(&hook, tool_input.clone(), &off, None);
    expect_full_read(&hook, &tool_input, &off, 0, "3rd read, switch off", |out| {
        assert!(
            warn_context(&out.stdout).contains("2 times"),
            "stdout={}",
            out.stdout
        );
    });
}

// 7. Three identical range reads warn; the rule never escalates to deny,
//    even with the switch on.
#[test]
fn repeat_range_reads_warn_but_never_deny() {
    let (_hook_dir, hook) = setup_hook();
    let files = TempDir::new().expect("files dir");
    let file = write_file_with_lines(files.path(), "small.rs", 50);
    let tool_input = json!({"file_path": file.to_string_lossy(), "offset": 5, "limit": 10});

    let session = Session::new().with_live_main_agent();
    session.enable_deny();

    for n in 1..=5 {
        let out = run_read_hook(&hook, tool_input.clone(), &session, None);
        assert_eq!(out.code, 0, "call {n}: stderr={}", out.stderr);
        if n >= 3 {
            let ctx = warn_context(&out.stdout);
            assert!(
                ctx.contains(&format!("has been read {n} times")),
                "call {n}: ctx={ctx}"
            );
        } else {
            assert!(
                out.stdout.trim().is_empty(),
                "call {n}: stdout={}",
                out.stdout
            );
        }
    }
}

// 9. A non-Read tool name, and a Read with no file_path, both exit 0
//    silently.
#[test]
fn non_read_tool_and_missing_file_path_are_silently_ignored() {
    let (_hook_dir, hook) = setup_hook();
    let session = Session::new();

    let wrong_tool = json!({
        "tool_name": "Bash",
        "tool_input": {"command": "echo hi"},
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

    let no_path = run_read_hook(&hook, json!({}), &session, None);
    assert_eq!(no_path.code, 0, "stderr={}", no_path.stderr);
    assert!(
        no_path.stdout.trim().is_empty() && no_path.stderr.trim().is_empty(),
        "stdout={} stderr={}",
        no_path.stdout,
        no_path.stderr
    );
}

// Direct check of the reads ledger's TSV row shape: path, kind, lines,
// timestamp - the format the repeat-read counters actually depend on, not
// just the guard's own observable behaviour.
#[test]
fn reads_ledger_row_is_tab_separated_path_kind_lines_timestamp() {
    let (_hook_dir, hook) = setup_hook();
    let files = TempDir::new().expect("files dir");
    let file = write_file_with_lines(files.path(), "small.rs", 50);
    let session = Session::new();

    let out = run_read_hook(
        &hook,
        json!({"file_path": file.to_string_lossy(), "offset": 5, "limit": 10}),
        &session,
        None,
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);

    let ledger = session.work_dir().join(format!(
        "hooks/reads/{}/{}.tsv",
        session.session_id, session.agent_id
    ));
    let content = fs::read_to_string(&ledger).unwrap_or_else(|e| panic!("read ledger: {e}"));
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 1, "content={content:?}");
    let fields: Vec<&str> = lines[0].split('\t').collect();
    assert_eq!(fields.len(), 4, "row={fields:?}");
    assert_eq!(fields[0], file.to_string_lossy());
    assert_eq!(fields[1], "range");
    assert_eq!(fields[2], "5-15");
    assert!(!fields[3].is_empty());
}

// 10. A binary/image extension is skipped even when large and unbounded (rule 1).
#[test]
fn binary_extension_skips_rule_one_even_when_large() {
    let (_hook_dir, hook) = setup_hook();
    let files = TempDir::new().expect("files dir");
    let stubs = TempDir::new().expect("stubs dir");
    let stub_dir = covered_stub_dir(stubs.path());
    let file = write_file_with_lines(files.path(), "big.png", 500);

    let session = Session::new().with_live_main_agent();
    session.enable_deny();
    let out = run_read_hook(
        &hook,
        json!({"file_path": file.to_string_lossy()}),
        &session,
        Some(&stub_dir),
    );

    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    assert!(out.stdout.trim().is_empty(), "stdout={}", out.stdout);
}

// 11. A binary/image extension is ALSO skipped by the repeat-read rule (rule
//     2), not just the large-file rule: three full reads of the same `.png`
//     never warn or deny, even with the switch on.
#[test]
fn binary_extension_skips_repeat_rule_too() {
    let (_hook_dir, hook) = setup_hook();
    let files = TempDir::new().expect("files dir");
    let file = write_file_with_lines(files.path(), "icon.png", 50);
    let session = Session::new().with_live_main_agent();
    session.enable_deny();

    for n in 1..=3 {
        let out = run_read_hook(
            &hook,
            json!({"file_path": file.to_string_lossy()}),
            &session,
            None,
        );
        assert_eq!(out.code, 0, "read {n}: stderr={}", out.stderr);
        assert!(
            out.stdout.trim().is_empty(),
            "read {n} must not warn: {}",
            out.stdout
        );
    }
}

// 12. The Read tool's `pages` parameter (a PDF page range - `offset`/`limit`
//     do not apply to a PDF) is a BOUNDED "range" read exactly like
//     offset/limit: three successive page-range reads of the same file are
//     never denied, even with the switch on. Before the fix a `pages` read
//     fell through to the "full" branch, so the 3rd call was denied with
//     offset/limit advice that means nothing for a PDF.
#[test]
fn pdf_pages_bounded_reads_are_never_denied() {
    let (_hook_dir, hook) = setup_hook();
    let files = TempDir::new().expect("files dir");
    let file = write_file_with_lines(files.path(), "spec.pdf", 50);
    let session = Session::new().with_live_main_agent();
    session.enable_deny();

    for pages in ["1-20", "21-40", "41-60"] {
        let out = run_read_hook(
            &hook,
            json!({"file_path": file.to_string_lossy(), "pages": pages}),
            &session,
            None,
        );
        assert_eq!(out.code, 0, "pages={pages}: stderr={}", out.stderr);
        assert!(
            out.stdout.trim().is_empty(),
            "pages={pages} must not warn: {}",
            out.stdout
        );
    }
}

/// Run a knowledge-file read and assert its outcome: the tier-1 shape (`expect_exempt`) always
/// warns naming `loom knowledge context` and never denies, even though the file is large enough
/// for rule 1 to deny it outright; the tier-2 control takes the ordinary rule-1 denial path.
fn assert_knowledge_read(
    hook: &Path,
    session: &Session,
    stub_dir: &Path,
    path: &Path,
    expect_exempt: bool,
) {
    let out = run_read_hook(
        hook,
        json!({"file_path": path.to_string_lossy()}),
        session,
        Some(stub_dir),
    );
    if expect_exempt {
        assert_eq!(out.code, 0, "stderr={}", out.stderr);
        assert!(
            warn_context(&out.stdout).contains("loom knowledge context"),
            "stdout={}",
            out.stdout
        );
    } else {
        assert_eq!(
            out.code, 2,
            "tier-2 file must not be exempt: stdout={} stderr={}",
            out.stdout, out.stderr
        );
    }
}

/// Run a knowledge-file read that must be exempt outright: exit 0 with nothing at all on
/// stdout, unlike `assert_knowledge_read`'s tier-1 shape which still warns. Used for
/// `INDEX.md`, which `_loom_is_knowledge_index_path` short-circuits ahead of rule 3.
fn assert_knowledge_read_silent(hook: &Path, session: &Session, stub_dir: &Path, path: &Path) {
    let out = run_read_hook(
        hook,
        json!({"file_path": path.to_string_lossy()}),
        session,
        Some(stub_dir),
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    assert!(
        out.stdout.trim().is_empty(),
        "INDEX.md must be silent: stdout={}",
        out.stdout
    );
}

// 13. A tier-1 knowledge file overrides rules 1 and 2 outright: always warns,
//     never denies, even though the file is large enough for rule 1 to have
//     denied it. `INDEX.md` is the sanctioned orientation read (CLAUDE.md
//     rule 12) and is exempt outright - silent, exit 0, no warning at all,
//     ahead of the tier-1 warning rule. A tier-2 topic file is the control -
//     NOT exempt.
#[test]
fn tier1_knowledge_read_warns_index_is_exempt_tier2_is_not_exempt() {
    let test = "repeat::tier1_knowledge_read_warns_index_is_exempt_tier2_is_not_exempt";
    if skip_unless_gate_visible(test) {
        return;
    }
    let (_hook_dir, hook) = setup_hook();
    let files = TempDir::new().expect("files dir");
    let stubs = TempDir::new().expect("stubs dir");
    let stub_dir = covered_stub_dir(stubs.path());
    let session = Session::new().with_live_main_agent();
    session.enable_deny();

    let kdir = files.path().join("doc/loom/knowledge");
    fs::create_dir_all(&kdir).expect("create knowledge dir");

    let mistakes = kdir.join("mistakes.md");
    fs::write(&mistakes, "line\n".repeat(500)).expect("write mistakes.md");
    assert_knowledge_read(&hook, &session, &stub_dir, &mistakes, true);

    let index = kdir.join("INDEX.md");
    fs::write(&index, "line\n".repeat(500)).expect("write INDEX.md");
    assert_knowledge_read_silent(&hook, &session, &stub_dir, &index);

    let tier2_dir = kdir.join("mistakes");
    fs::create_dir_all(&tier2_dir).expect("create tier2 dir");
    let tier2 = tier2_dir.join("refactor-stragglers.md");
    fs::write(&tier2, "line\n".repeat(500)).expect("write tier2 file");
    assert_knowledge_read(&hook, &session, &stub_dir, &tier2, false);
}

// 14. BLOCKER regression: `loom_hook_deny_or_warn` denies ONLY when BOTH the
//     `[hooks] deny_enabled` switch is on AND `LOOM_MAIN_AGENT_PID` is set to
//     a LIVE ancestor of the hook's bash process. `LOOM_WORK_DIR` is a
//     persisted, repo-wide pin (the settings `env` block overrides it for
//     every session in the repo), so gating on the switch alone would
//     hard-block an ordinary interactive session's Read calls the moment an
//     operator turns denies on repo-wide, with no orchestrator anywhere in
//     the process tree to fix it. Proves the blocker is closed: the switch
//     alone, and the switch plus a PID that is NOT a live ancestor (e.g.
//     "1"), both still only warn - never deny.
#[test]
fn deny_requires_a_live_main_agent_not_just_the_switch() {
    let (_hook_dir, hook) = setup_hook();
    let files = TempDir::new().expect("files dir");
    let stubs = TempDir::new().expect("stubs dir");
    let stub_dir = covered_stub_dir(stubs.path());

    let switch_only = Session::new();
    switch_only.enable_deny();
    let file_a = write_file_with_lines(files.path(), "switch-only.rs", 500);
    let out = run_read_hook(
        &hook,
        json!({"file_path": file_a.to_string_lossy()}),
        &switch_only,
        Some(&stub_dir),
    );
    assert_eq!(
        out.code, 0,
        "switch alone must never deny: stderr={}",
        out.stderr
    );
    assert!(
        warn_context(&out.stdout).contains("500"),
        "stdout={}",
        out.stdout
    );

    let non_ancestor = Session::new().with_main_agent_pid("1");
    non_ancestor.enable_deny();
    let file_b = write_file_with_lines(files.path(), "non-ancestor.rs", 500);
    let out = run_read_hook(
        &hook,
        json!({"file_path": file_b.to_string_lossy()}),
        &non_ancestor,
        Some(&stub_dir),
    );
    assert_eq!(
        out.code, 0,
        "a LOOM_MAIN_AGENT_PID that is not a live ancestor must never deny: stderr={}",
        out.stderr
    );
    assert!(
        warn_context(&out.stdout).contains("500"),
        "stdout={}",
        out.stdout
    );
}
