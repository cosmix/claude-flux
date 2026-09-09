//! Loading a recommended Codex skill must survive the Bash read guard.

use super::*;

#[test]
fn full_codex_and_native_skill_reads_are_exempt_even_when_repeated() {
    let (_hooks, hook) = setup_hook();
    let files = TempDir::new().unwrap();
    let stubs = TempDir::new().unwrap();
    let stub_dir = covered_stub_dir(stubs.path());
    let session = Session::new();
    for root in [
        ".codex/skills",
        ".codex/loom-skill-catalog",
        ".agents/skills",
    ] {
        let dir = files.path().join(root).join("loom-react");
        fs::create_dir_all(&dir).unwrap();
        let skill = write_file_with_lines(&dir, "SKILL.md", 500);
        let command = format!("cat {}", shell_escape::escape(skill.to_string_lossy()));
        for _ in 0..3 {
            let output = run_bash_hook(&hook, &command, &session, Some(&stub_dir));
            assert_eq!(output.code, 0, "{}", output.stderr);
            assert!(output.stdout.is_empty(), "{}", output.stdout);
            assert!(output.stderr.is_empty(), "{}", output.stderr);
        }
    }
    let ordinary = write_file_with_lines(files.path(), "ordinary.md", 500);
    let command = format!("cat {}", shell_escape::escape(ordinary.to_string_lossy()));
    let output = run_bash_hook(&hook, &command, &session, Some(&stub_dir));
    assert!(
        output.code != 0 || !output.stdout.is_empty() || !output.stderr.is_empty(),
        "positive control: ordinary large files must still reach the read guard"
    );
}
