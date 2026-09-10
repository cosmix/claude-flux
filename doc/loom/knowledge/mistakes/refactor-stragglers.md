# Refactor Stragglers

> What a large removal or rename leaves behind: straggler initializers, stale comments, stale docs, duplicate modules.

## Source vs Installed: Editing Wrong File

**Mistake:** Edited `~/.claude/hooks/loom/` (installed copy) instead of `hooks/` (source). Lost on reinstall.
**Fix:** Always edit in project's `hooks/` directory.

## Module Refactoring: Duplicate Files

**Mistake:** Splitting `tests.rs` into `tests/mod.rs` without deleting original caused E0761.
**Fix:** When refactoring `foo.rs` to `foo/mod.rs`, DELETE the original file.

## Stale References After Field Removal

**What happened:** After removing truths/truth_checks fields, stale references remained in comments (complete.rs:393), e2e test fixtures (plans.rs), README.md, skill files, and knowledge files.

**How to avoid:** When removing a struct field, grep the ENTIRE project (not just src/) for references. Include: tests/, doc/, skills/, README, knowledge files, comments, YAML fixtures.

## Stale Documentation After Adding Enum Variants (2026-04-16)

**What happened:** After adding KnowledgeDistill as the 4th StageType variant, three stale references remained: entry-points.md said 3 variants (should be 4), SKILL.md said Integration Verify Stage (Last) (now second-to-last), and sections.rs comment said integration-verify only (code had moved to KnowledgeDistill block).

**Why:** The implementation stage focused on Rust code changes and missed docs/comments that reference counts or ordering.

**Prevention:** When adding a new enum variant that changes ordering or counts, search all knowledge files for old counts, search skills for ordering claims, and search source comments for stale stage-type references.

## Parallel Deletion Stages: Straggler Files Outside Assignment Tables

**What happened:** After a parallel subagent deletion stage (`remove-container-keep-scaffolding`), 7 files remained with stale container references because they were not assigned to any subagent: `commands/mod.rs`, `completions/dynamic/tests.rs`, `plan/schema/mod.rs`, `commands/handoff/create.rs`, `commands/stage/tests/session.rs`, and an orchestrator preflight-checks module (since removed from the tree entirely in a later refactor). These caused compile failures discovered only at integration-verify.

**Why:** Parallel subagent deletion scopes by files owned — files that re-export, import, or reference the deleted code but weren't explicitly in the ownership table are silently missed. Test files (`#[cfg(test)]`) are especially prone since `cargo build` doesn't compile them.

**Prevention:**

- After any parallel deletion stage, the MAIN AGENT must run `cargo build && cargo test --no-run` (not just `cargo build`) — test-only files don't appear in a lib build.
- Before assigning subagents, run `rg` for the target symbols across the ENTIRE tree including `tests/`, `mod.rs` re-exports, and completions.

## Struct Field Removal: Straggler Initializers Across Workspace

**What happened:** Removing a struct field (e.g., removing the `execution` field from `LoomConfig`) left ~25 straggler struct literal initializers across test fixtures, core modules, and examples. Each was an explicit `execution: None` / `execution_backend: None` / `backend: Default::default()` that subagents missed because they only searched within their assigned file set.

**Why:** Rust requires all struct fields in literals unless `..Default::default()` spread is used. In a workspace with many test fixtures, explicit literals far outnumber `Default` spreads.

**Prevention:** After removing a struct field, the main agent MUST `rg` the WHOLE tree (including `tests/`) for `<field_name>:` before considering the work done. Do not rely on per-subagent grep scoped to owned files.

**Fix:** Used `..LoomConfig::default()` spread in all new struct literals going forward.

## Stale Code Comments After Large Structural Removals

**What happened:** The container backend removal (`remove-container-keep-scaffolding` + `collapse-backend-scaffolding`) correctly deleted code but left stale references in comments across 7+ files: `monitor/{core,handlers,detection}.rs` referenced `dispatcher`, `daemon/server/client.rs` had admin-token rationale citing containers, `commands/stage/{complete,knowledge_complete}.rs` had isolated-git/container comments. These were caught only at `integration-verify`.

**Why:** The stage that owned doc cleanup (`strip-container-docs`) ran `rg` for identifiers and string literals but did not search comments or table cells. Comments describing removed concepts stay syntactically valid and compile fine.

**Prevention:** A stage that owns cleanup of a removed concept must `rg` the whole tree for:

1. Identifier names (already done)
2. Human-readable name/framing in comments and docstrings (often missed)
3. Table cells in markdown files, knowledge docs, and SKILL.md files

Use `rg -i "container\|docker\|dispatcher" loom/src/ --include="*.rs"` to catch all forms.

## A Subagent That Splits a File Leaves an Untracked Straggler No Local Check Catches (2026-09-04)

**What happened:** a stage nearly shipped a branch whose committed tree could not compile.
The first commit was staged with an EXPLICIT file list taken from the subagent's report, but
the subagent had split `loom/src/commands/status/web/connection.rs` and created
`loom/src/commands/status/web/head.rs`, which its report never named.
`loom/src/commands/status/web/mod.rs` (committed) carried `mod head;` while the split-off
file stayed untracked — HEAD referenced a file not in git. `cargo build`, clippy, the full
test suite, the smoke script and the pre-commit hook all read the WORKING TREE, where the
file exists, so all of them passed; only `git status --short` showed the untracked `??` line.

**Prevention:** after the last commit of a stage, `git status --short` must be EMPTY — an
untracked `??` line under a source directory is a straggler, not noise. Never derive a
staging list from a subagent's report alone; derive it from `git status`. Stronger check
worth the ~70 seconds: `git clone --no-hardlinks --branch <branch> . $TMPDIR/x && cargo build
--all-targets` — the only thing that tests what the COMMIT contains rather than the disk.

**Fix:** `git reset --soft` to the pre-stage commit, then re-commit the same groups with the
missing file included — content was never at risk since a soft reset keeps the working tree.

## Registering a New Global PreToolUse Hook Has a Fourth Site Beyond the Documented Three

**What happened:** adding a global `PreToolUse` hook has a documented three-site checklist
(`fs/permissions/tests/constants_tests.rs:71`), but `fs/permissions/tests/hooks_tests.rs:32` also
hardcodes `assert_eq!(pre_tool.len(), N)` over the whole `pre_tool_hooks` list. Miss the fourth site
and the new hook compiles, installs, and runs correctly — the only symptom is
`test_hooks_config_structure` failing on a bare count mismatch that names no hook.

**Prevention:** when adding or removing a global hook, grep for every hardcoded count of
`pre_tool_hooks`/`post_tool_use`/similar lists, not just the documented checklist — a passing count
assertion elsewhere is a silent fourth site.
