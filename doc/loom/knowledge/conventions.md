# Coding Conventions

> Discovered coding conventions in the codebase.
>
> **Related files:** [patterns.md](patterns.md) for design patterns, [architecture.md](architecture.md) for system overview.

## File & Branch Naming

Rust-level style and structure conventions: naming (files, branches, sessions), error
handling, serialization, module layout, testing, ID/input validation, shared constants,
display/color rules, enum/builder patterns, comment style, code size limits, dependency
management, import deduplication, map module conventions, splitting a file safely,
docstring honesty, test fixtures, native-grammar dependency pins, the `INDEX_VERSION`
bump rule, and version/release identity → [Code Style and Structure](conventions/code-style-and-structure.md).

## Error Handling

See [Code Style and Structure](conventions/code-style-and-structure.md).

## Serialization

See [Code Style and Structure](conventions/code-style-and-structure.md).

## Module Organization & Re-exports

See [Code Style and Structure](conventions/code-style-and-structure.md).

## Testing

See [Code Style and Structure](conventions/code-style-and-structure.md).

## ID and Input Validation

See [Code Style and Structure](conventions/code-style-and-structure.md).

## Constants

See [Code Style and Structure](conventions/code-style-and-structure.md).

## Display Conventions

See [Code Style and Structure](conventions/code-style-and-structure.md).

## Git Operations

Git/worktree/build workflow conventions: git worktree and merge command shapes, the
active-merge guard and phantom-merge revert logging rules, `cargo fmt`/`cargo test`
invocation discipline in a shared worktree, the real test gate
(`cargo test --all-targets --no-fail-fast`) and its two non-hermetic tests, the shared
`maintainability-baseline.txt` ledger, the Bash tool's persistent working directory
gotcha, the explicit-push-only rule, and the daemon-credential read ban
→ [Git and Build Workflow](conventions/git-and-build-workflow.md).

## Plan YAML Schema

Plan/stage/signal/skill contract conventions: the plan YAML schema and its `acceptance`
field shape, hook conventions and the hook stdin/stdout contract (exit codes,
`hookSpecificOutput`, per-event schemas), permission-mode YAML values, signal file
format, the additive-schema-field pattern (`#[serde(default)]` over bespoke migration),
which seven knowledge files exist, and the skill file format/frontmatter/trigger rules
→ [Plan YAML and Hook Contracts](conventions/plan-yaml-and-hooks.md).

## Enum Conventions

See [Code Style and Structure](conventions/code-style-and-structure.md).

## Builder Pattern

See [Code Style and Structure](conventions/code-style-and-structure.md).

## Hook Conventions

See [Plan YAML and Hook Contracts](conventions/plan-yaml-and-hooks.md).

## Comment Style

See [Code Style and Structure](conventions/code-style-and-structure.md).

## Skill File Format

See [Plan YAML and Hook Contracts](conventions/plan-yaml-and-hooks.md).

## Code Size Limits

See [Code Style and Structure](conventions/code-style-and-structure.md).

## Dependency Management

See [Code Style and Structure](conventions/code-style-and-structure.md).

## Knowledge Files

See [Plan YAML and Hook Contracts](conventions/plan-yaml-and-hooks.md).

## Import Deduplication

See [Code Style and Structure](conventions/code-style-and-structure.md).

## Signal File Format

See [Plan YAML and Hook Contracts](conventions/plan-yaml-and-hooks.md).

## Map Module Conventions

See [Code Style and Structure](conventions/code-style-and-structure.md).

## Permission Mode YAML Values

See [Plan YAML and Hook Contracts](conventions/plan-yaml-and-hooks.md).

## Plan YAML Schema: Acceptance Field

See [Plan YAML and Hook Contracts](conventions/plan-yaml-and-hooks.md).

## Hook Output Contract

See [Plan YAML and Hook Contracts](conventions/plan-yaml-and-hooks.md).

## Dispute File Ownership Convention

Dispute/adjudication conventions: per-dispute file ownership and authority split, the
adjudicator's scope (which fields it may amend), the dispute/evidence/amendment budget
caps, the on-disk `attempts` respawn counter (and the removed `.inflight` marker/bug),
the daemon-as-filesystem-writer rule for `.work/` persistence, and the adjudicator's
spawned-session transport (no API key, no subprocess)
→ [Dispute and Adjudication](conventions/dispute-and-adjudication.md).

## Adjudicator Scope Convention

See [Dispute and Adjudication](conventions/dispute-and-adjudication.md).

## Dispute Budget Limits Convention

See [Dispute and Adjudication](conventions/dispute-and-adjudication.md).

## Adjudication Attempt Budget Convention

See [Dispute and Adjudication](conventions/dispute-and-adjudication.md).

## Daemon-as-Filesystem-Writer Convention

See [Dispute and Adjudication](conventions/dispute-and-adjudication.md).

## Adjudicator Transport Convention

See [Dispute and Adjudication](conventions/dispute-and-adjudication.md).

## Vendored Agent Assets Live at Repo Root

See [Guidance Channels and Plugin Scope](conventions/guidance-channels-and-plugin-scope.md).

## Guidance Delivery Channels Convention

Vendored-asset location, guidance-channel selection (hooks vs. signals vs. skills vs.
CLAUDE.md, and when to escalate prose to a hook), the main-agent-only verification rule,
and Claude Code plugin scope/settings-regeneration behaviour in loom repos
→ [Guidance Channels and Plugin Scope](conventions/guidance-channels-and-plugin-scope.md).

## Verification Is the Main Agent's Job

See [Guidance Channels and Plugin Scope](conventions/guidance-channels-and-plugin-scope.md).

## Git Push Requires Explicit User Request

See [Git and Build Workflow](conventions/git-and-build-workflow.md).

## Claude Code Plugin Scope in Loom Repos (2026-08-07)

See [Guidance Channels and Plugin Scope](conventions/guidance-channels-and-plugin-scope.md).

## Additive Schema Fields: Prefer `#[serde(default)]` Over Bespoke Migration (2026-08-07)

See [Plan YAML and Hook Contracts](conventions/plan-yaml-and-hooks.md).

## Formatting and Test Invocation in a Shared Worktree

See [Git and Build Workflow](conventions/git-and-build-workflow.md).

## Dependency Pins for Native-Grammar Crates

See [Code Style and Structure](conventions/code-style-and-structure.md).

## Splitting a File

See [Code Style and Structure](conventions/code-style-and-structure.md).

## Docstring Honesty

See [Code Style and Structure](conventions/code-style-and-structure.md).

## Deliberately-Invalid Test Fixtures

See [Code Style and Structure](conventions/code-style-and-structure.md).

## Working Directory

See [Git and Build Workflow](conventions/git-and-build-workflow.md).

## The Maintainability Ledger Is Shared State, and Only One Concurrent Stage May Own It

See [Git and Build Workflow](conventions/git-and-build-workflow.md).

## `cargo test` Is Not This Repo's Test Gate — `--all-targets --no-fail-fast` Is

See [Git and Build Workflow](conventions/git-and-build-workflow.md).

## Bump `INDEX_VERSION` Whenever `lexical::tokenize` Changes

See [Code Style and Structure](conventions/code-style-and-structure.md).

## Never Read the Daemon Credential Files

See [Git and Build Workflow](conventions/git-and-build-workflow.md).

## A Function Called From `$(...)` Cannot `exit` to Block Its Caller

See [Git and Build Workflow](conventions/git-and-build-workflow.md).

## Version and Release Identity

See [Code Style and Structure](conventions/code-style-and-structure.md).

## Web Dashboard Typography

Controls, cues and status words in the dashboard use the body face in sentence case; monospace only for ids and code, uppercase only via `eyebrow`, `.stage-tag` and `.rank-caption`. See [Web Dashboard Typography](conventions/web-dashboard-typography.md).
