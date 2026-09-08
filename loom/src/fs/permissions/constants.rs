//! Permission constants for loom

/// Common utilities shared across loom hooks (source guard, strip_embedded_content)
pub const HOOK_COMMON: &str = include_str!("../../../../hooks/_common.sh");

/// Read-discipline utilities shared by read-guard and poll-guard hooks
pub const HOOK_READ_DISCIPLINE: &str = include_str!("../../../../hooks/_read_discipline.sh");

/// TSV ledger read/write/cap utilities, sourced by `_read_discipline.sh`
/// (never registered as a hook directly)
pub const HOOK_READ_LEDGER: &str = include_str!("../../../../hooks/_read_ledger.sh");

/// Commit guard hook - enforces commit and stage completion in loom worktrees
/// Runs as a global Stop hook, blocks exit if uncommitted changes or stage incomplete
pub const HOOK_COMMIT_GUARD: &str = include_str!("../../../../hooks/commit-guard.sh");

// Embedded hook scripts for loom worktree sessions
// These are installed to ~/.claude/hooks/loom/ for use by Claude Code

/// PostToolUse hook - updates heartbeat after each tool use
pub const HOOK_POST_TOOL_USE: &str = include_str!("../../../../hooks/post-tool-use.sh");

/// Trusted PostToolUse bridge from sandboxed verification to a narrow daemon transition.
pub const HOOK_LOOM_CONTROL_COMPLETE: &str =
    include_str!("../../../../hooks/loom-control-complete.sh");

/// SessionStart hook - initializes heartbeat when session starts
pub const HOOK_SESSION_START: &str = include_str!("../../../../hooks/session-start.sh");

/// PreCompact hook - triggers handoff before context compaction
pub const HOOK_PRE_COMPACT: &str = include_str!("../../../../hooks/pre-compact.sh");

/// SessionEnd hook - handles session completion
pub const HOOK_SESSION_END: &str = include_str!("../../../../hooks/session-end.sh");

/// SubagentStop hook - records a Task-tool subagent's completion and refreshes
/// the parent session's heartbeat (the parent runs no tools of its own while
/// blocked waiting on a subagent, so PostToolUse cannot refresh it there).
pub const HOOK_SUBAGENT_STOP: &str = include_str!("../../../../hooks/subagent-stop.sh");

/// SubagentStart hook - records a Task-tool subagent's spawn type in the ledger
pub const HOOK_SUBAGENT_START: &str = include_str!("../../../../hooks/subagent-start.sh");

/// AskUserQuestion pre hook - marks stage as waiting for input
pub const HOOK_ASK_USER_PRE: &str = include_str!("../../../../hooks/ask-user-pre.sh");

/// AskUserQuestion post hook - resumes stage after user input
pub const HOOK_ASK_USER_POST: &str = include_str!("../../../../hooks/ask-user-post.sh");

/// PreferModernTools hook - suggests rg/fd instead of grep/find
pub const HOOK_PREFER_MODERN_TOOLS: &str = include_str!("../../../../hooks/prefer-modern-tools.sh");

/// CommitFilter hook - blocks forbidden patterns in git commits (e.g., Claude attribution)
pub const HOOK_COMMIT_FILTER: &str = include_str!("../../../../hooks/commit-filter.sh");

/// SubagentVerifyGuard hook - blocks a subagent from running project-wide verification
/// (bare `cargo test`/`clippy`/`build`, etc.); scoped runs and integration-verify
/// stage subagents are still allowed. Runs as a global PreToolUse:Bash hook.
pub const HOOK_SUBAGENT_VERIFY_GUARD: &str =
    include_str!("../../../../hooks/subagent-verify-guard.sh");

/// SkillTrigger hook - suggests skills based on prompt keywords (UserPromptSubmit)
pub const HOOK_SKILL_TRIGGER: &str = include_str!("../../../../hooks/skill-trigger.sh");

/// LearningValidator hook - validates session outcomes on Stop (memory usage checks)
pub const HOOK_LEARNING_VALIDATOR: &str = include_str!("../../../../hooks/learning-validator.sh");

/// GitAddGuard hook - blocks dangerous git add patterns (git add -A, git add ., git add .loom)
pub const HOOK_GIT_ADD_GUARD: &str = include_str!("../../../../hooks/git-add-guard.sh");

/// WorktreeIsolation hook - enforces worktree boundaries (blocks git -C, path traversal, cross-worktree access)
pub const HOOK_WORKTREE_ISOLATION: &str = include_str!("../../../../hooks/worktree-isolation.sh");

/// WorktreeFileGuard hook - defense-in-depth for file tools (Read, Write, Edit, Glob, Grep)
/// Validates target paths are within worktree boundary using LOOM_WORKTREE_PATH
pub const HOOK_WORKTREE_FILE_GUARD: &str = include_str!("../../../../hooks/worktree-file-guard.sh");

/// CredentialGuard hook - keeps the file tools (Read, Glob, Grep, Edit,
/// MultiEdit, Write, NotebookEdit) out of credential files. This is the
/// replacement for the `Read(...)` permission deny rules loom used to write:
/// any settings file carrying a `permissions.deny` entry that starts with
/// `Read(` makes Claude Code's Bash path validator prompt on every
/// relative-path `rg`/`grep`/`diff`/`git`/`cp`/`mv` issued after a `cd`.
/// `sandbox.filesystem.denyRead` is unaffected and still binds Bash; this hook
/// is the file-tool half of the same boundary.
pub const HOOK_CREDENTIAL_GUARD: &str = include_str!("../../../../hooks/credential-guard.sh");

/// NoPreexistingFailures hook - advisory pushback when an agent writes off a
/// red gate as pre-existing/flaky/environmental (CLAUDE.md rule 15)
pub const HOOK_NO_PREEXISTING_FAILURES: &str =
    include_str!("../../../../hooks/no-preexisting-failures.sh");

/// PlansPathGuard hook - blocks Write/Edit of plan files under .claude/plans paths
/// Fires in ALL sessions (plan mode runs interactively); redirects to doc/plans/
pub const HOOK_PLANS_PATH_GUARD: &str = include_str!("../../../../hooks/plans-path-guard.sh");

/// CodexForwardGuard hook - pins a codex forwarder subagent to the trusted
/// forwarding wrapper and blocks every other tool call.
pub const HOOK_CODEX_FORWARD_GUARD: &str = include_str!("../../../../hooks/codex-forward-guard.sh");

/// Trusted argv boundary used by codex forwarders.
pub const HOOK_CODEX_FORWARD: &str = include_str!("../../../../hooks/codex-forward.sh");

/// StageTerminalGuard hook - blocks Write/Edit/Task/Agent once a stage's own
/// status file says it is already completed/verified. Hard enforcement that
/// `loom stage complete` is the session's LAST act (commit-guard.sh is only
/// advisory).
pub const HOOK_STAGE_TERMINAL_GUARD: &str =
    include_str!("../../../../hooks/stage-terminal-guard.sh");

/// SpawnGuard hook - enforces subagent spawning constraints for Task and Agent tools
pub const HOOK_SPAWN_GUARD: &str = include_str!("../../../../hooks/spawn-guard.sh");

/// ReadGuard hook - enforces source-graph navigation before file reads
pub const HOOK_READ_GUARD: &str = include_str!("../../../../hooks/read-guard.sh");

/// PollGuard hook - enforces disciplined subagent polling through Bash
pub const HOOK_POLL_GUARD: &str = include_str!("../../../../hooks/poll-guard.sh");

/// UserPromptContext hook - delegates retrieval-backed context injection to
/// `loom hook user-prompt`; contains no retrieval logic of its own.
pub const HOOK_USER_PROMPT_CONTEXT: &str = include_str!("../../../../hooks/user-prompt-context.sh");

/// KnowledgeOrient hook - GLOBAL SessionStart nudge pointing a freshly
/// started session at this checkout's doc/loom/knowledge/INDEX.md, when one
/// exists. Unlike HOOK_SESSION_START (worktree-only, wired up per stage),
/// this runs in every repository and every session.
pub const HOOK_KNOWLEDGE_ORIENT: &str = include_str!("../../../../hooks/knowledge-orient.sh");

/// Codex apply_patch adapter - validates every patch target through the
/// canonical file guards and records successful edits for source retrieval.
pub const HOOK_CODEX_APPLY_PATCH: &str = include_str!("../../../../hooks/codex-apply-patch.sh");

/// All Loom hook scripts with their filenames. The complete asset set is
/// installed below both ~/.claude/hooks/loom and ~/.codex/hooks/loom; each
/// harness registers only the scripts compatible with its event payloads.
pub const LOOM_HOOKS: &[(&str, &str)] = &[
    // Common utilities (sourced by other hooks)
    ("_common.sh", HOOK_COMMON),
    ("_read_discipline.sh", HOOK_READ_DISCIPLINE),
    ("_read_ledger.sh", HOOK_READ_LEDGER),
    // Session lifecycle hooks
    ("post-tool-use.sh", HOOK_POST_TOOL_USE),
    ("loom-control-complete.sh", HOOK_LOOM_CONTROL_COMPLETE),
    ("session-start.sh", HOOK_SESSION_START),
    ("pre-compact.sh", HOOK_PRE_COMPACT),
    ("session-end.sh", HOOK_SESSION_END),
    ("subagent-stop.sh", HOOK_SUBAGENT_STOP),
    ("subagent-start.sh", HOOK_SUBAGENT_START),
    ("learning-validator.sh", HOOK_LEARNING_VALIDATOR),
    // Global hooks (commit enforcement, user question handling, tool guidance)
    ("commit-guard.sh", HOOK_COMMIT_GUARD),
    ("ask-user-pre.sh", HOOK_ASK_USER_PRE),
    ("ask-user-post.sh", HOOK_ASK_USER_POST),
    ("prefer-modern-tools.sh", HOOK_PREFER_MODERN_TOOLS),
    ("commit-filter.sh", HOOK_COMMIT_FILTER),
    ("subagent-verify-guard.sh", HOOK_SUBAGENT_VERIFY_GUARD),
    ("git-add-guard.sh", HOOK_GIT_ADD_GUARD),
    ("worktree-isolation.sh", HOOK_WORKTREE_ISOLATION),
    ("worktree-file-guard.sh", HOOK_WORKTREE_FILE_GUARD),
    ("credential-guard.sh", HOOK_CREDENTIAL_GUARD),
    ("plans-path-guard.sh", HOOK_PLANS_PATH_GUARD),
    ("no-preexisting-failures.sh", HOOK_NO_PREEXISTING_FAILURES),
    ("codex-forward-guard.sh", HOOK_CODEX_FORWARD_GUARD),
    ("codex-forward.sh", HOOK_CODEX_FORWARD),
    ("stage-terminal-guard.sh", HOOK_STAGE_TERMINAL_GUARD),
    ("spawn-guard.sh", HOOK_SPAWN_GUARD),
    ("read-guard.sh", HOOK_READ_GUARD),
    ("poll-guard.sh", HOOK_POLL_GUARD),
    // Skill suggestion hooks
    ("skill-trigger.sh", HOOK_SKILL_TRIGGER),
    ("user-prompt-context.sh", HOOK_USER_PROMPT_CONTEXT),
    // Orientation hooks (SessionStart)
    ("knowledge-orient.sh", HOOK_KNOWLEDGE_ORIENT),
    // Codex compatibility hooks
    ("codex-apply-patch.sh", HOOK_CODEX_APPLY_PATCH),
];

/// Loom permissions for the MAIN REPO context
/// Includes worktree permissions so settings.json can be read by worktrees
/// and all sessions share the same permission file (approvals propagate)
pub const LOOM_PERMISSIONS: &[&str] = &[
    // Read access to loom state, plus the ONE directory agents write with a
    // file tool. `.loom/work/` is otherwise read-only to file tools (CLAUDE.md
    // rule 11: state changes go through the `loom` CLI), and handoffs are the
    // sole direct write root — the same narrowing `sandbox/settings.rs` already
    // applies to generated stage settings (`Edit(/<abs>/handoffs/**)`).
    //
    // Deliberately NOT `Edit(<state>/**)` in EITHER spelling: this file is
    // copied verbatim into every stage worktree's settings.json, and a broad
    // Edit grant over the resolved state root re-exposes `admin.token` /
    // `user.token` (S-1, the daemon-RPC privilege escalation
    // `git/worktree/settings.rs` documents). Nor `Write(...)` in any spelling:
    // Claude Code's file permission check consults only `Edit(path)` rules, so
    // a `Write(path)` grant enforces nothing and only prints a startup warning.
    //
    // Both layouts are listed because this array is layout-unaware and its
    // entries are literal path patterns. A project whose workspace predates the
    // move still lives at `.work/`, and loom keeps reading AND writing it there;
    // with only the nested spelling, regenerating that project's settings.json
    // would strip the handoff-write grant its agents depend on. The unused
    // spelling matches nothing on either layout, so listing both costs nothing.
    "Read(.loom/work/**)",
    "Edit(.loom/work/handoffs/**)",
    "Read(.work/**)",
    "Edit(.work/handoffs/**)",
    // Read access to instruction files
    "Read(.claude/CLAUDE.md)",
    "Read(~/.claude/CLAUDE.md)",
    // Read access to loom hooks and catalogued skills loaded on demand
    "Read(~/.claude/hooks/loom/**)",
    "Read(~/.claude/loom-skill-catalog/**)",
    // Covers the loom-skills loader's fallback read on a `--skills all`
    // install (no catalog directory) and the installed skills themselves.
    "Read(~/.claude/skills/**)",
    // Loom CLI commands (use :* for prefix matching)
    "Bash(loom *)",
    // Loom's own codex forwarding wrapper. codex-forward-guard.sh independently
    // pins forwarder subagents to exactly one invocation of this wrapper.
    "Bash(~/.claude/hooks/loom/codex-forward.sh:*)",
];

/// Loom permissions for WORKTREE context
/// Worktrees are at .worktrees/stage-X/ with symlink .loom/work -> ../../../.loom/work
pub const LOOM_PERMISSIONS_WORKTREE: &[&str] = &[
    // Access via the symlink path (how Claude sees the paths). Same shape as
    // LOOM_PERMISSIONS above and for the same reasons: read-only over the state
    // root except handoffs, no broad `Edit(<state>/**)` in either spelling that
    // would re-expose the daemon tokens, and never a `Write(...)` rule (inert —
    // Claude Code's file permission check consults only `Edit(path)`). Both
    // layouts are listed for the same reason too: a worktree of a project whose
    // workspace predates the move symlinks `.work`, not `.loom/work`.
    "Read(.loom/work/**)",
    "Edit(.loom/work/handoffs/**)",
    "Read(.work/**)",
    "Edit(.work/handoffs/**)",
    // Read access to instruction files
    "Read(.claude/CLAUDE.md)",
    "Read(~/.claude/CLAUDE.md)",
    // Read access to loom hooks and catalogued skills loaded on demand
    "Read(~/.claude/hooks/loom/**)",
    "Read(~/.claude/loom-skill-catalog/**)",
    // Covers the loom-skills loader's fallback read on a `--skills all`
    // install (no catalog directory) and the installed skills themselves.
    "Read(~/.claude/skills/**)",
    // Loom CLI commands (use :* for prefix matching)
    "Bash(loom *)",
    // Loom's own codex forwarding wrapper. codex-forward-guard.sh independently
    // pins forwarder subagents to exactly one invocation of this wrapper.
    "Bash(~/.claude/hooks/loom/codex-forward.sh:*)",
];
