use serde_json::{json, Value};

pub(super) fn build(hooks_dir: &str) -> Value {
    json!({
        "PreToolUse": pre_tool_hooks(hooks_dir),
        "PostToolUse": [
            hook(hooks_dir, "AskUserQuestion", "ask-user-post.sh"),
            hook(hooks_dir, "Bash", "loom-control-complete.sh"),
        ],
        "Stop": [hook(hooks_dir, "*", "commit-guard.sh")],
        "UserPromptSubmit": [
            hook(hooks_dir, "*", "skill-trigger.sh"),
            hook(hooks_dir, "*", "user-prompt-context.sh"),
        ],
        "SessionStart": [hook(hooks_dir, "*", "knowledge-orient.sh")],
    })
}

/// Matchers are exact tool names, so every write-tool guard must name each
/// write tool explicitly. `MultiEdit` mutates files exactly as `Edit` does and
/// carries the same `tool_input.file_path`, so it is registered alongside every
/// `Edit` entry below - an unpaired `Edit` entry is a guard MultiEdit bypasses.
///
/// The same reasoning puts `SendMessage` under `codex-forward-guard.sh`: a
/// forwarder that relays codex's output through a message and ends its turn
/// with a summary strips the `--- LOOM-CODEX-EVIDENCE ---` trailer from the
/// report the orchestrator actually harvests (observed 2026-09-02).
const PRE_TOOL_HOOKS: &[(&str, &str)] = &[
    ("AskUserQuestion", "ask-user-pre.sh"),
    ("Bash", "prefer-modern-tools.sh"),
    ("Bash", "commit-filter.sh"),
    ("Bash", "subagent-verify-guard.sh"),
    ("Bash", "git-add-guard.sh"),
    ("Bash", "worktree-isolation.sh"),
    ("Edit", "worktree-file-guard.sh"),
    ("MultiEdit", "worktree-file-guard.sh"),
    ("Write", "worktree-file-guard.sh"),
    ("NotebookEdit", "worktree-file-guard.sh"),
    ("Edit", "plans-path-guard.sh"),
    ("MultiEdit", "plans-path-guard.sh"),
    ("Write", "plans-path-guard.sh"),
    ("Read", "worktree-file-guard.sh"),
    ("Glob", "worktree-file-guard.sh"),
    ("Grep", "worktree-file-guard.sh"),
    ("Read", "credential-guard.sh"),
    ("Glob", "credential-guard.sh"),
    ("Grep", "credential-guard.sh"),
    ("Edit", "credential-guard.sh"),
    ("MultiEdit", "credential-guard.sh"),
    ("Write", "credential-guard.sh"),
    ("NotebookEdit", "credential-guard.sh"),
    ("Bash", "no-preexisting-failures.sh"),
    ("Write", "no-preexisting-failures.sh"),
    ("Edit", "no-preexisting-failures.sh"),
    ("MultiEdit", "no-preexisting-failures.sh"),
    ("Bash", "codex-forward-guard.sh"),
    ("Bash", "loom-control-complete.sh"),
    ("Edit", "codex-forward-guard.sh"),
    ("MultiEdit", "codex-forward-guard.sh"),
    ("Write", "codex-forward-guard.sh"),
    ("NotebookEdit", "codex-forward-guard.sh"),
    ("Read", "codex-forward-guard.sh"),
    ("Task", "codex-forward-guard.sh"),
    ("Agent", "codex-forward-guard.sh"),
    ("SendMessage", "codex-forward-guard.sh"),
    ("Task", "spawn-guard.sh"),
    ("Agent", "spawn-guard.sh"),
    ("Read", "read-guard.sh"),
    ("Bash", "poll-guard.sh"),
    ("Write", "stage-terminal-guard.sh"),
    ("Edit", "stage-terminal-guard.sh"),
    ("MultiEdit", "stage-terminal-guard.sh"),
    ("NotebookEdit", "stage-terminal-guard.sh"),
    ("Task", "stage-terminal-guard.sh"),
    ("Agent", "stage-terminal-guard.sh"),
];

fn pre_tool_hooks(hooks_dir: &str) -> Vec<Value> {
    PRE_TOOL_HOOKS
        .iter()
        .map(|(matcher, script)| hook(hooks_dir, matcher, script))
        .collect()
}

fn hook(hooks_dir: &str, matcher: &str, script: &str) -> Value {
    json!({
        "matcher": matcher,
        "hooks": [{"type": "command", "command": format!("{hooks_dir}/{script}")}],
    })
}
