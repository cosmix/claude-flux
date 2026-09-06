# Phantom Merges

> Eight lessons on loom's merge machinery — writing merged=true without verifying git ancestry (the costliest recurring failure class in loom), the preflight guards and session lifecycle around it, and the silent resting state a failed auto-merge once left behind.

## Phantom Merges: merged=true Without Verification

**Mistake:** `try_auto_merge()` set `merged=true` without verifying the commit was in target branch history. Merge verification errors fell through to `merged=true` fallback. Agents also edited `.work/` files directly.
**Fix:** Use `is_ancestor_of()` to verify merge before setting `merged=true`. Treat verification errors as `MergeBlocked`. Never edit `.work/` files directly.

## Phantom Merges from Defensive "Assume Merged" Branches (2026-04-15)

**What happened:** Seven daemon-side code paths wrote `merged: true` to escape an earlier respawn-loop bug without verifying git ancestry. A user lost real work: stage `oauth-hardening` was marked merged but its commits stayed stranded on `loom/oauth-hardening`; a downstream stage then worktreed off main and produced overlapping, incomplete code. Smoking gun log: `Completed stage has no completed_commit, assuming merged stage_id=integration-verify`.

**Misleading signal:** The original respawn-loop bug (commit `1af9827`, see `doc/merge-resolve-bug-notes.md`) was patched by force-writing `merged=true` when stage was already Completed — the rationale "stage's work is done, don't revert to MergeBlocked" looked defensible. Similarly, seven separate sites used "assume already merged" / "legacy stage" / "avoid stuck-in-MergeBlocked loops" as justification for lying about merge state.

**Why it broke:** `merged=true` is a contract with the dependency scheduler. Dependents satisfy their deps by reading `dep.merged`. Lying about it silently propagates broken state across the DAG: dependents spawn with a wrong base branch, their commits overlap partially with the unmerged dep, progressive merge fails downstream.

**Prevention — INVARIANT:** **Daemon-side automated paths MUST NEVER write `merged: true` without git ancestry verification (`is_ancestor_of` returning `Ok(true)`).** The only exemptions are explicit user intent: `loom stage complete --force-unsafe --assume-merged`, `loom stage merge --resolved`, knowledge stages (no branch by design), and `loom worktree remove` cleanup.

**Detection rules for future work:**

- Any `stage.merged = true` write outside the exemption list is a phantom-merge candidate. Must be preceded by a git-verified `is_ancestor_of(completed_commit, target_branch)` returning `Ok(true)`.
- "Stage is Completed (terminal), can't go back" is NOT a license to write merged=true. It is not a license to stay silent either: since 2026-09-06 a failed auto-merge is forced to `MergeBlocked` with the error in `failure_info` (last entry in this file). `Completed + !merged` is the resting place only when auto-merge is disabled. The respawn loop that once argued for leaving a Completed stage alone is capped by `MAX_MERGE_RESOLVER_ATTEMPTS`.
- Dependency scheduling must cross-check ancestry (`are_all_dependencies_satisfied` in `verify/transitions/state.rs`), not trust the `merged` flag alone. Knowledge stages are the only exemption.
- `loom repair` catches stages with `merged: true` whose commit is not in the target branch — run on suspected phantom merges.

**Fix (implemented in this change):** Seven writer sites (recovery.rs, merge_handler.rs × 5, progressive_complete.rs) now leave `Completed + !merged` as the resting state instead of lying. `check_merge_state` returns `Unknown` for non-knowledge stages whose merged flag can't be ancestry-verified. `are_all_dependencies_satisfied` cross-checks ancestry per dep. `start_stage` adds a spawn-time defense-in-depth check. A one-shot retry on daemon start handles the `--no-verify`-then-restart case. `loom repair` detects and reverts phantom merges. Status UI renders `Completed + !merged` as yellow "unmerged" with a hint to run `loom stage merge <id>`. Superseded in part on 2026-09-06: the silent resting state, and a hint that named a command which refused Completed stages, are the subject of the last entry in this file.

## Phantom Merges from `--force-unsafe` Shortcuts (2026-04-27)

**What happened:** `loom stage complete --no-verify --force-unsafe --assume-merged` (and a related `--force-unsafe` alone path) wrote `merged: true` without ever verifying git ancestry. Three concrete failure modes:

1. **Phantom merge via `--assume-merged`.** `complete.rs::handle_force_unsafe_completion` set `merged = true` regardless of git reality, re-introducing the phantom-merge class via a user shortcut.
2. **Stuck `Completed + !merged` with active merge.** With `--force-unsafe` alone after a previous resolver session died mid-merge (`.git/MERGE_HEAD` set), the daemon retry called `merge_stage`, which failed; the next resolver ran `get_conflicting_files_from_status`, which destructively `git merge --abort`ed the existing active merge.
3. **`loom stage complete` on a `MergeConflict` stage.** Ran the full acceptance + goal-backward + progressive-merge pipeline, none of which is the resolver's job.

**Misleading signal:** Both `--force-unsafe` shortcuts looked defensible because they were "explicit user intent". But `--force-unsafe --assume-merged` made `merged: true` a contract violation: the dependency scheduler reads `dep.merged` and queues dependents as if the work landed. Cross-references the existing 2026-04-15 `Phantom Merges from Defensive "Assume Merged" Branches` entry — this is the user-shortcut variant of the same class.

**Why it broke:** Three preconditions all had to be wrong simultaneously: (a) no attribution check tied `MERGE_HEAD` to a specific stage, (b) `--assume-merged` skipped ancestry verification, (c) helpers that mutate git state (`merge_stage`, `get_conflicting_files_from_status`) had no guard against running over an in-progress merge. Together they made the active merge invisible to recovery.

**Prevention — Routing-and-Attribution INVARIANT:** _An active merge on disk may block or guide recovery, but it must not mutate a stage unless loom can attribute that merge to that stage._

- `MERGE_HEAD` in the main repo is global. Every state-machine mutation triggered by detection must come with proof of attribution: orphaned `SessionType::Merge` metadata, `MERGE_HEAD` commit matching `loom/<stage-id>` HEAD, or `completed_commit` match. Without attribution, refuse — never mutate.
- `--force-unsafe --assume-merged` must verify ancestry via `verify_merge_succeeded` before writing `merged=true`.
- `--force-unsafe` alone must refuse if an attributed active merge exists for THIS stage (would orphan MERGE_HEAD).
- Routing must be a pure read-only function (`route_complete_for_conflicts`) — persistence happens only on the success path so refusal preserves stage state.

**Fix (this change):**

- New module `git/merge/in_progress.rs` is the single source of truth for `MERGE_HEAD` detection.
- New module `orchestrator/merge_attribution.rs` ties active merges to specific stages via session metadata, branch HEAD, or `completed_commit`.
- `route_complete_for_conflicts` (in `commands/stage/complete.rs`) is the new pure routing seam — read-only, never mutates.
- `merge_verify::verify_or_derive_completed_commit` shared helper enforces ancestry for `--assume-merged` and `loom stage merge --resolved`.
- Daemon recovery runs `reconcile_main_repo_active_merge` BEFORE `sync_graph_with_stage_files` and BEFORE `recover_orphaned_sessions` so attribution sees session metadata before recovery deletes it.
- `sync_graph_with_stage_files` re-verifies `Completed + merged=true` non-knowledge stages, deriving from branch HEAD when missing and reverting `merged=false` when unverifiable.

## Helpers That Abort Active Merges (2026-04-27)

**What happened:** `merge_stage` and `get_conflicting_files_from_status` both ran `git merge --abort` on the repo as part of their normal flow (cleanup after success, abort the test merge). When invoked while a real merge was already in progress, they destroyed the user's resolution work.

**Misleading signal:** Both helpers acquire `MergeLock` at entry, so concurrent loom-driven merges are serialized. The bug is not concurrency — it's that the helpers don't distinguish "no merge in progress" from "a merge IS in progress that I didn't start".

**Prevention:** Helpers that mutate git merge state MUST refuse with `require_no_active_merge` when `MERGE_HEAD` is set on the repo path they're running in. Never silently `git merge --abort`. Defense in depth: even if attribution misses an active merge upstream, the guard surfaces an error instead of corrupting state.

**Fix:** Added `require_no_active_merge(repo_root)` helper in `git/merge/mod.rs`; called from `merge_stage` and `get_conflicting_files_from_status` after acquiring the merge lock. Both bail with a distinct error pointing at the path where the merge is in progress.

## Merge Probe Preflight Counted Untracked Files as "Dirty" (2026-08-17)

**What happened:** Stage `containment` sat in `merge-conflict` for hours with no resolver session. The daemon was trying every ~5s poll cycle and failing at the same place: `Failed to spawn merge resolution session for 'containment': merge probe infrastructure failure during cleanliness check: repository has uncommitted changes: ?? .codex ...`. The repo had 75 untracked files (plan drafts, scratch notes) and **zero** tracked modifications.

**Why it broke:** `require_clean_repository` in `git/merge/probe.rs` ran `git status --porcelain=v1 --untracked-files=all` and rejected _any_ non-empty output. Untracked entries are `??` lines, so a repo that was clean in every way that matters to a probe still failed the gate. `spawn_merge_resolution_session` (`orchestrator/core/merge_handler.rs`) calls the probe to enumerate conflicting files for the signal, so the resolver could never be spawned at all.

**Misleading signal:** The failure was classified `Infrastructure`, and `spawn_merge_resolution_sessions` deliberately does **not** count probe failures against `MAX_MERGE_RESOLVER_ATTEMPTS` — the reasoning being that probe failures are "transient operational errors, not failed resolver sessions." A dirty working tree is not transient. The stage therefore never escalated to `NeedsHumanReview` either; it just warned forever. A permanent precondition failure dressed as a transient one produces an infinite silent loop with no escalation path.

**Prevention:**

- A preflight gate must test the precondition the operation actually has. The probe only does `checkout_branch(target)` + `git merge --no-commit --no-ff`; neither touches untracked paths, so untracked files are irrelevant to it. Use `--untracked-files=no` and let git filter, rather than hand-parsing `??`.
- Do not exempt a failure class from a retry cap unless it is genuinely transient. If a failure can be permanent, it needs either a cap or an escalation path — otherwise the daemon loops forever and `loom status` shows a stage that looks alive but can never progress.
- Symptom to recognise: the same `Warning:` line repeating in `.work/orchestrator.log` at the poll interval, with a stage stuck in a non-terminal status and `session: null`.
- A merge that _would_ overwrite an untracked file is still caught — git refuses and `run_probe` surfaces it as an `Infrastructure` error carrying git's stderr. No extra preflight is needed for that case.

**Fix:** `git/merge/probe.rs` — `require_clean_repository` now uses `--untracked-files=no` and reports "uncommitted tracked changes"; regression test `untracked_files_do_not_block_the_probe` in `git/merge/probe/tests.rs`. The pre-existing test `dirty_repository_is_an_infrastructure_failure_without_mutation` had encoded the bug (it dirtied an _untracked_ `dirty.txt`) and now dirties the tracked `file.txt` instead.

**Diagnosis tip:** `git merge-tree --write-tree --name-only <target> <source>` probes a merge with zero mutation to the working tree or HEAD — safe to run while a daemon is live, unlike loom's own checkout-based probe.

## Merge Handler: Inline Branch Names

**Mistake:** 8 instances of `format!("loom/{}")` instead of `branch_name_for_stage()`.
**Fix:** Always use `branch_name_for_stage()` for branch name construction.

## Merge Conflict Session Lifecycle: Original Session Continued Running (2026-04-16)

**What happened:** When `loom stage complete` detected a merge conflict during progressive merge, the original execution session continued running instead of exiting. Three coordinated issues prevented clean handoff to the resolution session:

1. `complete_with_merge()` returned `Ok(false)` on merge conflict, which propagated back to `complete.rs:623` without error — the session stayed alive
2. `commit-guard.sh` (Stop hook) set `stage_incomplete=1` for `MergeConflict` status, blocking the session from exiting even if it tried
3. `spawn_merge_resolution_sessions()` didn't kill the stale original session, leaving a zombie process that blocked merge resolver spawning

**Why:** The `Ok(false)` return was designed for "merge didn't succeed but keep running" — wrong mental model. Merge conflict means "your work is done, hand off to resolver." The commit-guard didn't distinguish between "stage still executing" and "stage waiting for merge resolution." And session cleanup assumed sessions would exit on their own.

**Prevention:**

- When adding new terminal/handoff stage statuses, always update: (1) `complete_with_merge` return behavior, (2) `commit-guard.sh` case statement, (3) `detection.rs` normal-exit matches, (4) `spawn_merge_resolution_sessions` cleanup logic
- Use `bail\!()` not `Ok(false)` when the session MUST exit — `Ok(false)` leaves the caller alive
- Test the full lifecycle: stage completes → merge conflicts → original session exits → resolver spawns → resolver resolves

**Fix:** Four-part coordinated change:

- `progressive_complete.rs`: Changed `Ok(false)` to `bail\!()` for Conflict and Blocked arms, forcing session exit with clear message
- `commit-guard.sh`: Changed MergeConflict case to allow session exit (no longer sets stage_incomplete)
- `merge_handler.rs`: Added `kill_session()` call for stale Stage sessions before spawning merge resolver
- `merge.rs`: Added "Inherited Responsibilities" section to merge signal explaining resolver owns the stage

## Cleanup Inside "Merge" Destroyed the Evidence (2026-08-17)

A new route into this same failure class, worth reading in full because the fix is
structural rather than another check: `attempt_auto_merge` performed worktree and
branch cleanup inside its own success arms, so it deleted the branch that its caller
uses to derive a missing `completed_commit`. Nothing wrote a false `merged: true` —
the code simply removed the ability to verify, which is the same class arrived at
from the opposite side.

The repair introduced `orchestrator/merge_lifecycle.rs` as the single door to
post-merge cleanup, made `attempt_auto_merge` return an UNCLEANED outcome, and pinned
the order: overlay reconcile, merge, **verify merged ancestry**, base reconcile, mark
state and release dependents, then cleanup. Cleanup now refuses outright unless the
stage branch is provably contained in the target.

Full detail, including the detection rule ("after this returns, what can no longer be
verified?") and the two subsidiary rules about live-cwd deferral and derived-state
failure budgets: `mistakes/merge-cleanup-boundary.md`.

## Silent `Completed + !merged` After a Failed Auto-Merge (2026-09-06)

**What happened:** In projects running loom, the final stage of a plan (usually knowledge-distill) finished on its own and `loom status` showed it completed but "unmerged", the daemon gone, nothing under "Requires Attention". `loom stage merge <id>`, which the status hint and the daemon log both recommended, refused: `require_merge_state` accepted only `MergeConflict` or `MergeBlocked`. The operator merged the branch with `git merge` by hand.

**Why:** The daemon writes `Completed` (merged=false, no completed_commit) when `CompleteStage` arrives, before any merge. Every failure arm in `try_auto_merge` and `verify_and_finalize_merge` then hit `if stage.status == Completed { log; return false }`, and `persist_merge_blocked` early-returned for Completed stages, so a git refusal (dirty main checkout, lock timeout, missing branch), a failed ancestry check, or the zero-commits phantom guard all left `Completed + !merged` with a `tracing::error!` line as the only trace. `all_stages_terminal` counts a Completed node as terminal, so for the last stage the daemon exited on the same tick; a non-final stage kept its dependents waiting on `merged`. The 2026-04-15 fix above chose that resting state to avoid a resolver respawn loop that `MAX_MERGE_RESOLVER_ATTEMPTS` has since capped, so the guard's reason had lapsed while its cost, invisibility, remained. Two false signals hid it: the sync logged "Completed stage commit is not an ancestor of target branch; leaving as Completed + !merged" at ERROR on every normal completion right before merging, and `merged: true` appearing a second later made that line look like noise.

**Prevention:** A state the daemon can leave a stage in must be (1) visible in `loom status` with a reason and (2) actionable by the command the hint names. Check both when adding a failure arm: `commands/status/render/attention_model.rs` must produce an entry for the status, and the named command's preflight must accept it. A `return false` that only logs is a silent state. When a guard cites a reason ("avoid the respawn loop"), re-check the reason whenever the mechanism it names changes.

**Fix:** `persist_merge_blocked` now applies to Completed stages, records the error in `failure_info` (`InfrastructureError`, evidence = the git error lines), forces `MergeBlocked`, and prints the `loom stage merge` hint; all failure arms route through it. The empty-branch guard routes to `NeedsHumanReview` with a `review_reason` via `route_to_human_review`. `require_merge_state` accepts `Completed + !merged` (the auto-merge-disabled resting state) and `try_complete_merge` tolerates an already-Completed stage. The sync's routine "not yet in target" line is `debug`. Tests: `orchestrator/core/merge_handler_attempt_tests.rs::failed_auto_merge_moves_completed_stage_to_merge_blocked` and `::empty_stage_branch_routes_to_human_review`.
