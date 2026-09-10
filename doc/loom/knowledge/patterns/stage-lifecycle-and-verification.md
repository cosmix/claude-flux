---
---
# Stage Lifecycle And Verification

> Stage/session states, locked writes, acceptance & verification layers.

## State Machine Pattern

Stage has 13 states: WaitingForDeps -> Queued -> Executing -> Completed (terminal). From Executing: Blocked, NeedsHandoff, WaitingForInput, MergeConflict, CompletedWithFailures, MergeBlocked, NeedsHumanReview, and NeedsAdjudication. Skipped is terminal. **Critical invariant**: dependents become Queued only when deps have `status == Completed AND merged == true`. Session has 6 states: Spawning -> Running -> Completed/Crashed/ContextExhausted, plus Paused<->Running. All transitions validated via `try_transition()`.

## File-Based State Pattern

All state persisted to `.work/` as markdown with YAML frontmatter. Benefits: git-friendly diffing, human-readable, crash recovery via file re-read. Stage files named with topological depth prefix (e.g., `01-knowledge-bootstrap.md`).

**Concurrency is NOT single-writer.** The orchestrator loop, daemon IPC handlers, and agent-run CLI commands all mutate stage files. Existing-record changes must use the canonical locked `update_stage` transaction; crash-atomic replacement alone does not prevent stale logical writes. See the Locked Stage Read-Modify-Write Pattern below.

## Locked Stage Read-Modify-Write Pattern (A-5)

`locked_read`/`locked_write` serialize _individual_ reads/writes, but the load → mutate → save flow releases the lock between load and save. Each `save_stage` serializes the **entire** `Stage`, so a writer that loaded the stage minutes earlier (e.g. `loom stage complete` holding a stage across a multi-minute acceptance run) reverts any field a concurrent writer changed in the gap — a lost update (status reverted, `dispute_count`/`retry_count`/`close_reason`/`session`/amended `acceptance` clobbered).

**Fix — `verify::transitions::update_stage(stage_id, work_dir, |s| { ... })`:** holds the `stages/` directory lock across a _fresh_ on-disk read, the closure, and the crash-atomic write. The closure mutates the **current** persisted `Stage`, so it only touches the fields the operation owns; a concurrent writer's other fields survive. Returns the written `Stage`. The file must already exist; creation uses `create_stage` (`save_stage` is a creation-only compatibility alias and refuses overwrites). A closure `Err` leaves the file untouched.

```rust
// Re-read under the lock, apply only the operation-owned delta:
update_stage(stage_id, work_dir, |s| {
    s.dispute_count = s.dispute_count.saturating_add(1); // incremented from on-disk value
    s.try_request_adjudication(reason)                   // status transition validated on-disk
})?;
```

Underlying primitives (`fs/locking.rs`): `locked_dir_update(dir, f)` locks a directory inode for the duration of `f` (for find-read-write when the file's exact prefixed path is unknown); `atomic_write_locked(path, content)` is the temp+rename write used _inside_ a held lock.

**Field-ownership rule (the judgment-heavy part):** for a long operation, re-apply only the fields that operation owns and leave every other field at its freshly read on-disk value. Progressive merge owns completed commit/merge/status fields; merge retry owns `fix_attempts` and its merge transition; dispute/adjudication owns its review counters, status, and amendment fields; plan amendment owns only the amendable verification policy.

**Long-op shape:** run the slow work (git merge under its own `MergeLock`, acceptance commands) OUTSIDE the stages-dir lock, then apply the owned fields in a SHORT `update_stage` closure — never hold the stages-dir lock across git/subprocess work.

**Invariants preserved:** never write `merged=true` without ancestry verification (the `merged=true` writes in `merge.rs`/`merge.rs --resolved` follow a real merge or a `verify_or_derive_completed_commit` ancestry check, both done before the closure); `route_complete_for_conflicts` stays a pure read-only seam (no early whole-`Stage` save before its decision); status transitions still go through `try_*`/`force_status_with_reason`.

**No orchestrator exemption:** although the scheduler loop is single-threaded, daemon IPC and CLI writers are concurrent. Recovery, merge, crash, completion, and event handlers therefore apply the same short locked deltas as commands. Whole-record persistence is reserved for actual stage creation.

## Stage Completion Pattern

**Regular stages**: Load stage, run acceptance criteria (unless --no-verify), sync worktree permissions, run task verifications, progressive merge, mark Completed, trigger dependents. **Knowledge stages**: No worktree, commits required (directly to main), auto merged=true, skips merge. Acceptance commands: 5-min timeout, support `${WORKTREE}`, `${PROJECT_ROOT}`, `${STAGE_ID}` variables.

## Field Propagation Checklist

When adding new fields to StageDefinition: (1) plan/schema/types.rs, (2) models/stage/types.rs + Default, (3) commands/init/plan_setup.rs mapping, (4) plan/schema/tests/mod.rs make_stage(), (5) ALL test files constructing Stage, (6) validation.rs rules, (7) fs/stage_loading.rs, plan/graph/tests.rs, models/stage/methods.rs.

## NeedsHumanReview Orchestrator Handling Pattern

For new `NeedsAdjudication` state, mirror the existing `NeedsHumanReview` pattern:

1. `orchestrator/monitor/detection.rs:87-92` — emit `MonitorEvent::StageNeedsHumanReview` on transition detection
2. `orchestrator/core/event_handler.rs:142-158` — print banner + notify
3. `orchestrator/core/recovery.rs:814` — `StageStatus::NeedsHumanReview => continue` (skip auto-retry)
4. `orchestrator/core/recovery.rs:515-526` — sync status to in-memory graph

Add parallel handling for `NeedsAdjudication` that fires the worker thread instead of continuing.

## Session Identity: Setter + Clearer Must Travel Together

Every field group on `Session` that represents a runtime resource identity requires a matching setter AND clearer method.

| Field group | Setter      | Called after    |
| ----------- | ----------- | --------------- |
| `pid`       | `set_pid()` | Session spawned |

**Rule:** Any caller that releases a runtime resource must call the matching clearer before persisting the session file.

## Goal-Backward Verification Pattern

Four verification layers: **Artifacts** (files must exist, stub detection blocks TODO/FIXME/unimplemented\!/todo\!/pass/raise NotImplementedError). **Wiring** (grep patterns verify code connections). **Wiring Tests** (runtime commands with success criteria). **Dead Code Check** (command + fail/ignore patterns).

**Truths is NOT a layer here.** It was removed as a standalone goal-backward layer and unified into the acceptance field as `AcceptanceCriterion::Extended(TruthCheck)`. A duplicate section claiming "Three verification layers: Truths, Artifacts, Wiring" was deleted from this file on 2026-07-30. Required for `stage_type: standard` and `integration-verify` — must have acceptance OR goal-backward checks.

Validation limits (`plan/schema/validation.rs`): max 100 artifacts; max 20 `before_stage` checks; max 20 `after_stage` checks.

Before/after stage checks: before_stage runs AFTER worktree creation, BEFORE Executing — blocking (failed check → stage Blocked, no session spawned), and only while the workspace is pristine (skipped once the stage branch/worktree holds prior work; see architecture.md). after_stage runs in complete.rs (blocking). Both use TruthCheck definitions via `verify_truth_checks()` in truths.rs.

Regression tests: `bug_fix: true` requires `regression_test` with file path and must_contain patterns. Bidirectional validation.

Advisory stderr warning detection: `detect_stderr_warnings()` in runner.rs scans for 9 suspicious patterns (connection refused, blocked, EACCES, etc.) after acceptance. Warnings only, no pass/fail change.

## AcceptanceCriterion Design Pattern

Uses `#[serde(untagged)]` enum with two variants:

- `Simple(String)` — plain shell command, deserializes from YAML string
- `Extended(TruthCheck)` — output validation, deserializes from YAML object with `command` field

Serde tries variants in order: strings match Simple first, objects fail Simple then match Extended. Error messages for malformed objects are poor (inherent untagged limitation). helper methods: `command()`, `is_extended()`, `Display` delegates to `command()`.

## AcceptanceCriterion Untagged Enum

`AcceptanceCriterion` in `plan/schema/types.rs` is a `#[serde(untagged)]` enum:

```rust
#[serde(untagged)]
pub enum AcceptanceCriterion {
    Simple(String),        // YAML: - "cargo test"
    Extended(TruthCheck),  // YAML: - command: "cargo test"\n  exit_code: 0
}
```

**Serialization**: serde tries each variant in declaration order. A plain YAML string deserializes to `Simple`; a mapping with a `command` key deserializes to `Extended(TruthCheck)`.

**Accessing the command**: use `.command()` method — works for both variants.

**`TruthCheck`** fields: `command`, optional `exit_code` (default 0), optional `stdout_contains`, optional `stderr_empty`.

**Why untagged**: avoids requiring a `type: simple` / `type: extended` discriminator in user-authored YAML. The trade-off is that serde error messages on malformed input are less precise.

## Extract the Decision When the Failure Mode Is Not Reproducible in CI (2026-08-08)

**Problem shape:** a rule guards an OS failure that no CI runner can produce. The test written against
the real command passes _for the wrong reason_ and would still pass with the rule deleted.

Concretely: the e2e case meant to pin tmux's "exit 0 but stderr non-empty is a failure" rule used an
unwritable socket parent — which makes tmux exit **1**, so the plain exit-code check alone satisfied
it. The genuine condition needs the socket dir to exist while socket _creation_ is denied.

**Pattern:** split the rule into a pure decision fn over already-gathered inputs
(`evaluate_new_session(socket, status_success, stderr)`) and unit-test _that_. The impure caller keeps
only the gathering. Applied again in `build_overview_argv`, which takes the viewer socket and
`(socket, tracking_key)` pairs as parameters rather than deriving them, so the whole argv sequence is
assertable without tmux.

**Rule:** when an OS failure mode is not reproducible in CI, extract the decision and test it directly.
Never settle for a test that passes for the wrong reason — see `mistakes/tests-that-cannot-fail.md`.

## Matched Positive and Negative Controls for a Boundary Test

`verify/criteria/tests/confine_tests.rs` ships
`confined_shell_command_does_not_see_ambient_secret` **and**
`inherited_shell_command_does_see_ambient_secret`. The pair distinguishes "the scrub works"
from "the canary was never set", which a single negative assertion cannot. `process/environment.rs:92`
does the same at unit level by exec'ing `/usr/bin/env` and asserting the canary is absent
from real child output rather than inspecting a `Command` struct.

**A boundary test needs the allow case asserted alongside the deny case, or it cannot fail
when the boundary silently stops applying.** This is the positive form of
`mistakes/tests-that-cannot-fail.md`.

## Prove a Regression Guard Guards by Reverting It and Watching Red

A green full suite is not evidence a fix is protected — a fix can ship with zero tests reaching it
and a 3950-test suite stays green if reverted. The check: copy the file aside, restore the pre-fix
body, run the narrow test target for that module, confirm it goes red, restore the file
byte-identically (`diff -q`), re-run to confirm green again. Cheap (about two minutes) and worth doing
for every regression guard accepted from a subagent rather than trusting its own claim to have
checked. A module with no `#[cfg(test)] mod tests` at all is the tell that nobody has ever had to
test it. See [Status Broadcast Hardening](../mistakes/status-broadcast-hardening.md) for the case this
caught.

## A Sandboxed Worktree Session Can Prove the Static Status Renderer End to End

`mktemp -d` a fixture dir, `mkdir .loom/work`, copy the real `stages/`, `sessions/`, and config
files into it with `cp -rL` (the `.loom/work` symlink needs `-L`), delete `orchestrator.sock` and
`orchestrator.pid`, then run the built binary with that dir as cwd. It renders the full dashboard,
the legend line, and the Requires Attention blocks (exercising `attention_entries` via
`commands/status/render/attention.rs`) and reports "daemon stopped" — no shared state is touched. The live `--live`
TUI still cannot be proven this way: it needs both a daemon socket and a TTY. Reach for this fixture
instead of skipping functional verification of the static path.

## Degraded Modes Are Reported, Never Silent

`FileCoverage` gives every file a node even when it could not be parsed — `LexicalOnly`,
`Oversized`, `ParseError` — so a consumer can tell "no symbols here" from "not analysed".
When adding a new extractor or analyser, the degraded paths are the ones to test: the happy
path fails loudly, the degraded paths fail silently.
