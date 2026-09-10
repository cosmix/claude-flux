---
---
# Security And Isolation

> 4-layer worktree defense, security model, settings.local.json sites.

## Worktree Isolation (4-Layer Defense)

1. **Git layer** -- Separate worktrees at `.worktrees/<stage-id>/` with branch `loom/<stage-id>`. Symlinks: `.work` -> shared state, .claude/CLAUDE.md -> instructions, root `CLAUDE.md` -> project guidance.
2. **Sandbox layer** -- `MergedSandboxConfig` (`sandbox/config.rs`) generates `settings.local.json` with filesystem deny/allow policy, network domains, and fail-closed sandbox availability. Plan-configured `excluded_commands` are rejected; generated settings do not grant broad executable exemptions. Knowledge writes use the narrow Loom control path rather than direct file edits.
3. **Signal layer** -- Four stage-type-specific stable prefix generators in cache.rs (standard, knowledge, integration-verify, knowledge-distill). Include isolation rules and subagent restrictions.
4. **Hook layer** -- commit-guard.sh blocks exit without commit. commit-filter.sh blocks subagent git operations and subagent-verify-guard.sh blocks subagent full-suite verification, both gated on `loom_is_subagent()` (live-ancestor `LOOM_MAIN_AGENT_PID`, then a payload-first classification via `loom_payload_agent_verdict` — the intervening-Claude-process walk is only the fallback — not a PPID comparison).

## Security Model

- **ID validation**: Alphanumeric + dash/underscore, max 128 chars, no path traversal (validation.rs)
- **Acceptance criteria**: Runs arbitrary shell commands (trusted model)
- **Socket**: Mode 0o600 (owner only), max 100 connections, 10MB message limit, Unix only
- **Self-update**: minisign signature verification for binaries; `agents.zip`, `skills.zip`, and `CLAUDE.md.template` ARE SHA256-verified against the release checksums asset (self-update refuses to install an asset with no checksum entry). The asset-name mismatch an earlier version of this bullet described is fixed: `commands/self_update/mod.rs::checksum_asset` fetches `SHA256SUMS.txt`, the name the release workflow publishes (corrected 2026-09-03; `PLAN-embed-assets-and-complete-self-update` removes the config-asset download path entirely)
- **Shell escaping**: escape_shell_single_quote(), escape_applescript_string() in emulator.rs
- **permission_mode field** (`SandboxConfig` / `StageSandboxConfig`): Resolves as stage > plan > stage-type default. Default by stage type: ALL four stage types → `auto` (Knowledge, KnowledgeDistill, Standard, IntegrationVerify) — loom stages run autonomously with no human to answer prompts, so the agent auto-accepts actions its heuristics deem safe; the sandbox deny/allow rules are the safety boundary. Override to a stricter mode (`accept-edits`, `plan`) at plan or stage level if needed. **Delivery:** the resolved mode is passed as the `--permission-mode` CLI flag by `build_claude_command` at spawn — NOT via `permissions.defaultMode` in the worktree's `settings.local.json`, which Claude Code v2.1.142+ ignores for `auto` (a repo cannot grant itself auto mode; only the CLI flag or user/managed settings are honored). Auto mode itself requires a supporting account/model (Opus 4.6+/Sonnet 4.6+); loom's job is only to request it correctly. See entry-points.md and mistakes.md.

## Per-Worktree Gitignore for settings.local.json

After worktree creation, `.claude/settings.local.json` is appended (idempotently) to `<worktree>/.git/info/exclude`. Uses per-worktree exclude to avoid polluting the repo's `.gitignore`.

- Standard/IntegrationVerify/KnowledgeDistill: append to `<worktree>/.git/info/exclude`
- Knowledge stages: append to main repo's `.git/info/exclude` (no worktree created)
- The per-worktree exclude file lives at `<worktree>/.git/info/exclude` — NOT at `<worktree-dir>/.git/info/exclude` (the latter is a FILE pointing at the real gitdir, not a directory; the real exclude is at `<repo>/.git/worktrees/<stage-id>/info/exclude`)

## Claude Code Worktree Isolation Disabled in Generated Settings

Loom owns the per-stage git worktree, so it disables Claude Code's _own_ worktree
isolation (`worktree.bgIsolation`) in every settings file it generates. Claude
Code's default (`"worktree"`) blocks Edit/Write in the checkout until
`EnterWorktree`, which would push subagents into nested worktrees on top of loom's
— leaving stray branches and tangled checkouts. Loom emits `"none"` so subagents
edit the loom worktree directly (Claude Code v2.1.143+; older versions ignore it).

Two write sites, both targeting `settings.local.json` (never the committed
`settings.json`, to avoid imposing on non-loom teammates):

- **Worktree stage sessions** — `sandbox/settings.rs:generate_settings_json()`
  emits a top-level `"worktree": { "bgIsolation": "none" }` block. Survives the
  `merge_existing_permissions()` step, which only touches `permissions.*`.
- **Main-repo sessions** (knowledge stages, interactive) —
  `fs/permissions/settings.rs:ensure_loom_hooks_local()` sets it idempotently
  alongside the agent-teams env var.
