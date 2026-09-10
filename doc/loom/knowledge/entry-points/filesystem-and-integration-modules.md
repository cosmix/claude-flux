---
---
# Filesystem And Integration Modules

> Git, fs/work_dir, handoff, sandbox, remote control, knowledge base

## Git Operations

- `git/worktree/operations.rs` - Create/remove worktrees at `.worktrees/{stage-id}/`
- `git/worktree/base.rs` - Base branch resolution for dependencies
- `git/worktree/settings.rs` - Worktree symlinks (.work, .claude/CLAUDE.md, CLAUDE.md)
- `git/merge/mod.rs` - Merge automation, conflict handling; `require_no_active_merge` guard
- `git/merge/in_progress.rs` - Single source of truth for `MERGE_HEAD` detection (handles `.git`-as-file, relative gitdirs, octopus merges)
- `git/merge/lock.rs` - Stable-inode OS lock that serializes concurrent merges without stale-file reclamation races
- `git/merge/status.rs` - `check_merge_state` (Merged | Pending | Conflict | BranchMissing | Unknown)
- `git/branch/mod.rs` - Branch creation, deletion, ancestry checks (module root; re-exports from `operations.rs`, `cleanup.rs`, `ancestry.rs`, `status.rs`, `naming.rs`, `info.rs`)

## File System State

- `fs/work_dir.rs` - `.work/` directory management (initialize, load, main_project_root)
- `fs/stage_files.rs` - Stage file naming (`{depth}-{stage-id}.md`)
- `fs/session_files.rs` - Session file operations
- `fs/knowledge/dir.rs` - Knowledge directory operations (`KnowledgeDir`; module root `fs/knowledge/mod.rs`)
- `fs/memory/mod.rs` - Session memory operations (module root; re-exports from `persistence.rs`, `query.rs`, `spool.rs`, `storage.rs`, `archive.rs`, `export.rs`, `types.rs`)
- `fs/verifications.rs` - Goal-backward verification results

## Handoff System

- `commands/handoff/create.rs` - CLI `loom handoff create` implementation
- `orchestrator/monitor/handoff_watch.rs` - `HandoffWatch::needs_handoff_from_document` recovers a handoff from the handoff DOCUMENT (not the stage file) for a session running sandboxed without write access to `.work/stages`; caches per filename so no document is parsed twice. An earlier version of this entry pointed at a `handoff/detector.rs` that does not exist.
- `handoff/generator/mod.rs` - Handoff file generation
- `handoff/schema/mod.rs` - HandoffV2 structured format (module root; struct defined in `handoff/schema/v2.rs`)

## Sandbox

- `sandbox/config.rs` - MergedSandboxConfig, merge_config(), expand_paths()
- `sandbox/settings.rs` - generate_settings_json(), write_settings()

## Remote Control Module

- `loom/src/remote_control.rs` - `resolve_invocation(work_dir, name)` per-spawn gate (layers a `--help` probe over `resolve()`, now called only by the crash handler), `preflight(path)`, `write_unsupported_marker(work_dir)`, `run_startup_preflight(path, work_dir)`, `RemoteControlInvocation` / `RemoteControlConfig` / `RemoteControlMode` types

## Other Modules

- `src/claude.rs` - Shared find_claude_path() utility
- `completions/generator.rs` - Custom shell script generation (bash/zsh/fish)
- `completions/dynamic/mod.rs` - Context-aware dynamic completion engine
- `completions/dynamic/commands.rs` - Per-command completion definitions
- `completions/scripts/` - Shell-specific completion script templates
- `completions/install.rs` - Auto-install and migration for shell completions
- `commands/status/ui/tui/mod.rs` - TUI dashboard entry (run_tui)
- `commands/self_update/mod.rs` - Installation, update, skill download
- `process/mod.rs` - bounded subprocess execution and structured timeout errors
- `process/identity.rs` - PID plus start-time identity verification and fail-closed signaling
- `process/environment.rs` - minimal allowlisted environment reconstruction for stage processes
- `skills/` - SkillIndex, SkillMatch, SkillMetadata (index.rs, matcher.rs, types.rs)
- `diagnosis/signal.rs` - generate_diagnosis_signal(), DiagnosisContext
- `map/analyzer.rs` no longer exists — `analyze_codebase(root, deep, focus)` was removed along with `map/{analyzer,detectors,knowledge_sync}.rs`; `loom map` is now three read-only view flags (`--outline`, `--find-all`, `--impact`) defined in `map/mod.rs` — see entry-points/cli-and-plan-pipeline.md § New CLI Surface.

## WorkDir Directory Helpers (Existing vs. Missing)

`fs/work_dir.rs:270-294` — existing helpers:

- `signals_dir()` → `.work/signals/`
- `handoffs_dir()` → `.work/handoffs/`
- `archive_dir()` → `.work/archive/`
- `stages_dir()` → `.work/stages/`
- `sessions_dir()` → `.work/sessions/`
- `crashes_dir()` → `.work/crashes/`
- `knowledge_dir()` → `.work/knowledge/`
- `ensure_dir(&self, name: &str) -> Result<PathBuf>` — create any subdir on demand

**Both helpers are now implemented:** `disputes_dir()` → `.work/disputes/` (`fs/work_dir.rs:239`) and `plan_versions_dir()` → `.work/plan_versions/` (`fs/work_dir.rs:244`)

Two more subdirectories, resolved by their own modules rather than a `WorkDir` helper:

- `acceptance-cache/` — cached acceptance passes, `<sha256>.json`, keyed by criterion text and tree digest (`verify/criteria/cache.rs`)
- `capsules/` — per-session generated Claude Code settings files (`<session-id>.settings.json`) for judge sessions, written at spawn and removed on close (`terminal/native/session_settings.rs`)

## Sandbox Settings — ANTHROPIC_API_KEY

`sandbox/settings.rs:16-34` — `SENSITIVE_ENV_KEYS` array filters `ANTHROPIC_API_KEY` from agent sandbox environments.

- This is env hygiene only. It no longer has anything to do with adjudication: an earlier version of this line claimed an absent `ANTHROPIC_API_KEY` disabled adjudication and sent disputes straight to `NeedsHumanReview`, which stopped being true when the adjudicator moved to a spawned `claude -p` session. What disables it now is a missing `claude` binary — see conventions.md § Adjudicator Transport Convention.

## HTTP Client Pattern — self_update/client.rs

`commands/self_update/client.rs` — `create_http_client() -> Result<Client>`:

- `Client::builder().connect_timeout(10s).timeout(120s).user_agent("loom-self-update").build()`
- `validate_response_status(&response, context)` — checks `is_success()`, returns descriptive HTTP errors
- Streaming download with size limit enforcement (buffer size 8192)
- Error propagation: `.context("Failed to ...")` pattern throughout

This is the pattern for loom's HTTP consumers (self-update). The adjudicator is NOT one of them: an earlier version of this line said an adjudicator HTTP client should mirror it, but the adjudicator spawns a `claude -p` session and makes no HTTP call at all — see conventions.md § Adjudicator Transport Convention.

## Tiered Knowledge Base (2026-07-28)

| Path                                               | Role                                                                                                                        |
| ----------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `loom/src/fs/knowledge/types.rs`                   | `KnowledgeFile`, `KnowledgeTarget`, `KnowledgeLayout`, tier-1 alias table                                                   |
| `loom/src/fs/knowledge/dir.rs`                     | `KnowledgeDir` — initialize, append, layout detection; `replace_section`/`replace_section_target` delegate the actual splicing to `splice.rs`                                                      |
| `loom/src/fs/knowledge/splice.rs`                  | `splice_section` — level-agnostic (`##` through `######`) section splicer; returns `SectionOutcome`                                                   |
| `loom/src/fs/knowledge/index.rs`                   | `scan_topics`, `generate_index`, `write_index`                                                                              |
| `loom/src/fs/knowledge/templates.rs`               | tier-1/tier-2 scaffolds                                                                                                     |
| `loom/src/commands/knowledge/mod.rs`               | the four knowledge verbs — `update` (append), `replace-section` (overwrite the section body at any heading level `##` to `######`), `context`, `sync`               |
| `loom/src/cli/types_memory.rs`                     | clap definitions for the knowledge subcommands                                                                              |

`loom knowledge sync`, or any `loom knowledge update`, regenerates `INDEX.md` — the index
regenerates on every knowledge write — and on a flat directory creates it, which is what flips
the layout to hierarchical. See [Knowledge Hierarchy](../architecture/knowledge-hierarchy.md).
