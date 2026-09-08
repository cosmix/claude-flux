//! Tests for command, subcommand, and flag completions

use crate::completions::dynamic::commands::*;
use crate::completions::dynamic::complete_knowledge_scopes;

#[test]
fn test_complete_commands_all() {
    let results = complete_commands("").unwrap();
    assert!(results.len() >= 20); // At least 20 top-level commands
    assert!(results.contains(&"init".to_string()));
    assert!(results.contains(&"run".to_string()));
    assert!(results.contains(&"stage".to_string()));
    assert!(results.contains(&"status".to_string()));
    assert!(results.contains(&"completions".to_string()));
}

#[test]
fn test_complete_commands_prefix() {
    let results = complete_commands("st").unwrap();
    assert!(results.contains(&"stage".to_string()));
    assert!(results.contains(&"status".to_string()));
    assert!(results.contains(&"stop".to_string()));
    assert!(!results.contains(&"init".to_string()));
}

#[test]
fn test_complete_commands_no_match() {
    let results = complete_commands("xyz").unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_complete_subcommands_stage() {
    let results = complete_subcommands("stage", "").unwrap();
    assert!(results.contains(&"complete".to_string()));
    assert!(results.contains(&"block".to_string()));
    assert!(results.contains(&"reset".to_string()));
    assert!(results.contains(&"human-review".to_string()));
    assert!(results.contains(&"dispute-criteria".to_string()));
    assert!(results.contains(&"output".to_string()));
}

#[test]
fn test_complete_subcommands_stage_prefix() {
    let results = complete_subcommands("stage", "com").unwrap();
    assert_eq!(results.len(), 1);
    assert!(results.contains(&"complete".to_string()));
}

#[test]
fn test_complete_subcommands_knowledge() {
    let results = complete_subcommands("knowledge", "").unwrap();
    // `check` is the read-only diagnostics command (`commands/knowledge/check.rs`),
    // not the long-deleted subcommand that once bore the name — it belongs with
    // the live subcommands below, not on the `gone` list.
    for present in ["update", "replace-section", "context", "sync", "check"] {
        assert!(
            results.contains(&present.to_string()),
            "knowledge must still complete `{present}`"
        );
    }
    for gone in [
        "show",
        "init",
        "index",
        "list",
        "audit",
        "gc",
        "bootstrap",
        "status",
    ] {
        assert!(
            !results.contains(&gone.to_string()),
            "knowledge must not complete the deleted `{gone}`"
        );
    }
}

#[test]
fn test_complete_subcommands_memory() {
    let results = complete_subcommands("memory", "").unwrap();
    assert!(results.contains(&"note".to_string()));
    assert!(results.contains(&"decision".to_string()));
    assert!(results.contains(&"list".to_string()));
    assert!(results.contains(&"show".to_string()));
}

#[test]
fn test_complete_subcommands_sessions() {
    let results = complete_subcommands("sessions", "").unwrap();
    assert!(results.contains(&"list".to_string()));
    assert!(results.contains(&"kill".to_string()));
}

#[test]
fn test_complete_subcommands_worktree() {
    let results = complete_subcommands("worktree", "").unwrap();
    assert!(results.contains(&"list".to_string()));
    assert!(results.contains(&"remove".to_string()));
}

#[test]
fn test_complete_subcommands_output() {
    let results = complete_subcommands("output", "").unwrap();
    assert!(results.contains(&"set".to_string()));
    assert!(results.contains(&"get".to_string()));
    assert!(results.contains(&"list".to_string()));
    assert!(results.contains(&"remove".to_string()));
}

#[test]
fn test_complete_subcommands_unknown() {
    let results = complete_subcommands("unknown", "").unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_complete_flags_run() {
    let results = complete_flags(&["run"], "--").unwrap();
    assert!(results.contains(&"--manual".to_string()));
    assert!(results.contains(&"--watch".to_string()));
    assert!(results.contains(&"--foreground".to_string()));
    assert!(results.contains(&"--max-parallel".to_string()));
    assert!(results.contains(&"--no-merge".to_string()));
}

#[test]
fn test_complete_flags_run_prefix() {
    let results = complete_flags(&["run"], "--m").unwrap();
    assert!(results.contains(&"--manual".to_string()));
    assert!(results.contains(&"--max-parallel".to_string()));
    assert!(!results.contains(&"--watch".to_string()));
}

#[test]
fn test_complete_flags_status() {
    let results = complete_flags(&["status"], "--").unwrap();
    assert!(results.contains(&"--live".to_string()));
    assert!(results.contains(&"--compact".to_string()));
    assert!(results.contains(&"--verbose".to_string()));
}

#[test]
fn test_complete_flags_stage_complete() {
    let results = complete_flags(&["stage", "complete"], "--").unwrap();
    assert!(results.contains(&"--no-verify".to_string()));
    assert!(results.contains(&"--force-unsafe".to_string()));
    assert!(results.contains(&"--session".to_string()));
}

#[test]
fn test_complete_flags_stage_retry() {
    let results = complete_flags(&["stage", "retry"], "--").unwrap();
    assert!(results.contains(&"--force".to_string()));
    assert!(results.contains(&"--context".to_string()));
}

#[test]
fn test_complete_flags_unknown_command() {
    let results = complete_flags(&["nonexistent"], "--").unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_complete_shell_types_all() {
    let results = complete_shell_types("").unwrap();
    assert_eq!(results.len(), 3);
    assert!(results.contains(&"bash".to_string()));
    assert!(results.contains(&"zsh".to_string()));
    assert!(results.contains(&"fish".to_string()));
}

#[test]
fn test_complete_shell_types_prefix() {
    let results = complete_shell_types("b").unwrap();
    assert_eq!(results.len(), 1);
    assert!(results.contains(&"bash".to_string()));
}

#[test]
fn test_complete_model_names_all() {
    // Pin to the wiring (completer tracks `CLAUDE_MODELS`), not a frozen copy
    // of the list, so a vocabulary change can't silently desync the two.
    let results = complete_model_names("").unwrap();
    let expected: Vec<String> = crate::claude::CLAUDE_MODELS
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(results, expected);
    assert!(results.contains(&"fable".to_string()));
}

#[test]
fn test_complete_codex_model_names_all() {
    let results = complete_codex_model_names("").unwrap();
    let expected: Vec<String> = crate::codex::CODEX_MODELS
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(results, expected);
    assert!(results.contains(&"gpt-6-astra".to_string()));
}

#[test]
fn test_complete_codex_model_names_prefix() {
    let results = complete_codex_model_names("gpt-5.6-").unwrap();
    assert!(results.contains(&"gpt-5.6-sol".to_string()));
    assert!(results.contains(&"gpt-5.6-terra".to_string()));
    assert!(results.contains(&"gpt-5.6-luna".to_string()));
    assert!(!results.contains(&"gpt-6-astra".to_string()));
}

#[test]
fn test_complete_trigger_types_all() {
    let results = complete_trigger_types("").unwrap();
    assert_eq!(results.len(), 3);
    assert!(results.contains(&"manual".to_string()));
    assert!(results.contains(&"precompact".to_string()));
    assert!(results.contains(&"session_end".to_string()));
}

#[test]
fn test_has_subcommands() {
    assert!(has_subcommands("stage"));
    assert!(has_subcommands("sessions"));
    assert!(has_subcommands("worktree"));
    assert!(has_subcommands("knowledge"));
    assert!(has_subcommands("memory"));
    assert!(has_subcommands("plan"));
    assert!(!has_subcommands("init"));
    assert!(!has_subcommands("run"));
    assert!(!has_subcommands("status"));
}

#[test]
fn test_complete_commands_includes_plan() {
    let results = complete_commands("").unwrap();
    assert!(results.contains(&"plan".to_string()));
}

#[test]
fn test_complete_subcommands_plan() {
    let results = complete_subcommands("plan", "").unwrap();
    assert_eq!(results, vec!["verify".to_string()]);
}

#[test]
fn test_complete_flags_plan_verify() {
    let results = complete_flags(&["plan", "verify"], "--").unwrap();
    assert!(results.contains(&"--json".to_string()));
    assert!(results.contains(&"--no-color".to_string()));
    assert!(results.contains(&"--strict".to_string()));
}

#[test]
fn test_complete_commands_includes_pressure() {
    let results = complete_commands("").unwrap();
    assert!(results.contains(&"pressure".to_string()));
}

#[test]
fn test_complete_commands_pressure_prefix() {
    let results = complete_commands("pr").unwrap();
    assert!(results.contains(&"pressure".to_string()));
}

#[test]
fn test_complete_flags_pressure() {
    let results = complete_flags(&["pressure"], "--").unwrap();
    assert!(results.contains(&"--rounds".to_string()));
    assert!(results.contains(&"--dry-run".to_string()));
}

#[test]
fn test_complete_commands_includes_attach() {
    let results = complete_commands("").unwrap();
    assert!(results.contains(&"attach".to_string()));
}

#[test]
fn test_pressure_has_no_subcommands() {
    assert!(!has_subcommands("pressure"));
}

#[test]
fn test_plan_verify_positional_completes_plan_files() {
    use crate::completions::dynamic::complete_plan_files;
    use std::fs;
    use tempfile::TempDir;

    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let plans_dir = root.join("doc/plans");
    fs::create_dir_all(&plans_dir).unwrap();
    fs::write(plans_dir.join("PLAN-foo.md"), "# Test").unwrap();

    let results = complete_plan_files(root, "").unwrap();
    assert!(results.iter().any(|r| r.contains("PLAN-foo.md")));
}

#[test]
fn test_complete_flags_knowledge_context() {
    let results = complete_flags(&["knowledge", "context"], "").unwrap();
    assert!(results.contains(&"--query".to_string()));
    assert!(results.contains(&"--budget-tokens".to_string()));
    assert!(results.contains(&"--explain".to_string()));
    assert!(results.contains(&"--json".to_string()));
    assert!(results.contains(&"--scope".to_string()));
    assert!(results.contains(&"--require-id".to_string()));
}

#[test]
fn test_complete_knowledge_scopes_all() {
    let results = complete_knowledge_scopes("").unwrap();
    assert_eq!(
        results,
        vec![
            "all".to_string(),
            "knowledge".to_string(),
            "source".to_string()
        ]
    );
}
