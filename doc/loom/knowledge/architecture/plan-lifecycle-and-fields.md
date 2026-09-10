---
---
# Plan Lifecycle And Fields

> Plan field checklist, goal-backward layers, schema fields, amendment.

## Adding New Plan Fields Checklist

1. Add to StageDefinition (plan/schema/types.rs) with serde defaults
2. Add validation in validation.rs
3. Add to Stage model (models/stage/types.rs) with serde defaults
4. Copy in the canonical `Stage::from_definition()` builder (`models/stage/methods.rs`); init delegates to it
5. If goal-check: update has_any_goal_checks() in BOTH StageDefinition and Stage
6. If verification: add verify function in verify/goal_backward/ and call from run_goal_backward_verification()
7. Check ALL test files constructing Stage directly (src/ AND tests/ directories)
8. If the field reaches the agent: copy it onto `EmbeddedContext` (orchestrator/signals/types.rs) and
   emit it from BOTH format/sections.rs AND recovery_format.rs — the recovery signal embeds only the
   stable prefix, so a gated section missing there vanishes on any retry
9. Add the two backwards-compat tests (plan YAML without the key; legacy `.work/stages/*.md` without
   the key) — see [Additive Schema Fields](../conventions.md); `#[serde(default)]` is the only migration
10. If it is user-facing, add a row to the README Stage Fields table

## Goal-Backward Verification (verify/goal_backward/)

Four verification layers for standard stages. **`truths` is NOT one of them** — it was removed from goal-backward and merged into acceptance. A duplicate section listing `truths` as a goal-backward layer was deleted from architecture.md on 2026-07-30; if you see that claim anywhere else, it is stale.

- **artifacts** -- Files must exist with real implementation (stub detection: TODO, FIXME, unimplemented\!, todo\!)
- **wiring** -- Regex patterns verifying code connections in source files
- **wiring_tests** -- Runtime command-based integration verification
- **dead_code_check** -- Command + pattern detection for unused code

Acceptance criteria (verify/criteria/runner.rs) now handle both:

- **Simple** -- Plain shell command, 5min timeout, exit 0 = pass
- **Extended** -- TruthCheck struct with stdout_contains, stderr_empty, exit_code, 30s timeout

Returns: GoalBackwardResult::Passed | GapsFound | HumanNeeded. Storage: `.work/verifications/<stage-id>.json`.

Note: truths.rs module and verify_truth_checks() are retained for before_stage/after_stage verification (pre/post conditions), NOT for goal-backward.

## before_stage / after_stage / code_review Schema Fields — Execution Status

**Status as of 2026-06-15 (verified against stage_executor.rs:219-256, plan/schema/types.rs:261, and orchestrator/signals/generate.rs):**

| Field          | Schema Type                | Stored on Stage            | Executed   | Where                                             |
| -------------- | --------------------------- | --------------------------- | ---------- | -------------------------------------------------- |
| `before_stage` | `Vec<TruthCheck>`          | Yes (plan_setup.rs:280) | Yes     | stage_executor.rs:220-256 (pre-spawn)             |
| `after_stage`  | `Vec<TruthCheck>`          | Yes (plan_setup.rs:281) | Yes     | commands/stage/complete.rs:847-866                |
| `code_review`  | `Option<CodeReviewConfig>` | Yes                     | Signal  | signals/generate.rs renders it for IV signals     |

**`before_stage` execution (`stage_executor.rs::before_stage_gate_passed`):**

- Runs after worktree creation, BEFORE session spawn
- **Gated on a pristine workspace.** `verify::before_after::find_prior_stage_work(stage_branch, base_branch, repo_root, worktree_path)` runs first; if it finds commits on `loom/<stage-id>` beyond the resolved base, or non-scaffold changes in the worktree, the checks are SKIPPED (logged at `info`) and the spawn proceeds. `before_stage` is a delta-proof ("the feature does not exist yet"), which is only meaningful on the first attempt — re-running it on a re-spawn (orphan recovery, `loom stage retry`, crash retry) fails on the previous attempt's own work and blocks the stage before any session exists to finish it (unrecoverable loop; see mistakes.md 2026-07-27)
- Loom's own worktree scaffolding (`.work`, `.claude/`, root `CLAUDE.md`) is discounted via `git::worktree::is_worktree_scaffold_path` — it is present from the first spawn, and in repos that don't gitignore it, counting it would disable the gate entirely
- Calls `crate::verify::before_after::run_before_stage_checks(&stage.before_stage, &check_dir)`
- On failure gaps: stage → `Blocked` (FailureType::TestFailure), session NOT spawned. `TestFailure` is not auto-retryable (`should_auto_retry`), so the stage rests Blocked until an operator runs `loom stage retry`
- On errors (infrastructure): prints warning, continues anyway (advisory)
- TruthCheck timeout: 30 seconds (hardcoded in truths.rs:13)

**`after_stage` execution (commands/stage/complete.rs:847-866):**

- Runs during `loom stage complete`, AFTER acceptance criteria pass
- On failure: stage stays Executing, no merge, agent must fix and re-run

**`code_review` — PERSISTED AND WIRED FOR SIGNAL GENERATION:**

- Parsed by serde at schema level and copied by `Stage::from_definition()` onto the runtime `Stage`
- `orchestrator/signals/generate.rs` reads the persisted runtime field; it does not reparse the plan
- `render_review_dimensions()` emits a `## Review Dimensions` checkbox section in IV signals, honoring `require_all`
- It remains agent guidance rather than an acceptance or goal-backward verification primitive

## load_stage_definition_from_plan — Centralized Plan Lookup

Centralized in `plan/parser/mod.rs` (re-exported via `plan/mod.rs`). Previously lived in `commands/verify.rs`.

**Signature:** `load_stage_definition_from_plan(stage_id, work_dir) -> Result<Option<StageDefinition>>`

Reads `.work/config.toml` for plan path, calls `resolve_source_path()`, calls `parse_plan()`, finds stage by ID. Used by:

- `commands/stage/complete.rs` — after-stage execution

**Why plan/ layer:** completion needs the authoritative plan definition for checks that are intentionally evaluated from the active plan. Runtime policy copied onto `Stage` should not reparse the plan at its consumers.

## Plan Versioning / Runtime Amendment (Shipped — `plan/amendment.rs`)

Runtime plan amendment exists and is reachable from an `Accept` adjudication verdict. `.work/plan_versions/` is its audit trail:

- `.work/plan_versions/.lock` — `flock` (serializes amendments)
- `.work/plan_versions/<n>.md` — snapshot of full plan content after amendment n
- `.work/plan_versions/audit.md` — O_APPEND atomic rows (amendment log)

**The write set is TWO files, not one — this is the trap.** Under the lock, an amendment writes the snapshot, appends the audit row, replaces the live plan file via `safe_replace_outside_workdir`, **and rewrites the target stage's `.work/stages/<n>-<id>.md`**. The last step is not optional: `plan/graph/loader.rs` prefers `.work/stages/` over the plan file, so amending only the plan would leave `sync_graph_with_stage_files` serving the old criteria forever. (An earlier version of this section described a 6-step flow ending at "atomic rename plan file", omitting the stage-file write.)

The proposed value is deserialized into the **real** `AcceptanceCriterion` / `WiringCheck` types before anything is written, so a malformed patch fails fast rather than corrupting the plan. A per-stage cap (default 3, `loom.adjudication.max_amendments_per_stage`) bounds runaway adjudication.

**Recovery** — `verify_plan_versions_consistency()`, called from orchestrator startup, handles three divergences:

| On disk                                 | Action                                                             |
| ---------------------------------------- | -------------------------------------------------------------------- |
| Snapshot written, audit row missing     | Orphaned snapshot — removed so the next amendment can claim the id |
| Audit row appended, plan file still old | Re-apply the snapshot to plan + stage file (catch-up commit)       |
| Plan + audit in sync, stage file stale  | Re-apply just the stage-file update                                |

## Plan Immutability Invariant (Narrowed, Not Removed)

Plans are loaded ONCE at daemon startup via `build_execution_graph()` -> `ExecutionGraph::build()`; there is no general reload mechanism, and the in-memory `graph: ExecutionGraph` on `Orchestrator` holds all state. Amendment is the one sanctioned mutation path and it is deliberately narrow: **only the `acceptance` and `wiring` arrays on a single stage**. Stage IDs, dependencies, `working_dir`, DAG topology, and plan structure are never amendable — so the graph the daemon loaded at startup stays topologically valid for the life of the run.
