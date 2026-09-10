# Merge Flow

> How a completed worktree stage reaches the target branch: the daemon writes Completed first, the sync loop runs the first merge attempt, and every failure outcome lands in a visible stage state.

## Merge Flow

How a completed worktree stage reaches the target branch, in the order the daemon runs it, and
what every outcome leaves on disk. Knowledge stages are outside this flow: they commit to the
base directly and `complete_knowledge_stage` sets `merged: true` with no branch.

### The stage is `Completed` before any merge runs

`loom stage complete` inside a worktree verifies only. The PostToolUse broker forwards
`CompleteStage` to the daemon, whose `handle_complete_stage` (`daemon/server/control_complete.rs`)
calls `stage.try_complete(None)` and writes the stage file: `status: completed`, `merged: false`,
`completed_commit: null`. No merge has happened yet, so every merge attempt that follows sees a
stage that is already `Completed`, a terminal state the transition table refuses to leave.

### Where the first merge attempt actually happens

The main loop (`orchestrator/core/orchestrator.rs`) runs `sync_graph_with_stage_files`
(`orchestrator/core/recovery.rs`) before `monitor.poll()` on every tick. The sync finds the fresh
stage, derives `completed_commit` from the `loom/<id>` branch head, runs the ancestry check (false,
nothing is merged yet), marks the graph node Completed, and queues the stage for the "Fix 11"
one-shot retry at the end of the same function. That retry calls `try_auto_merge`
(`orchestrator/core/merge_handler.rs`), which is the first and normal merge attempt. The
`StageCompleted` monitor event, handled next in the same tick by `handle_stage_completed`
(`orchestrator/core/completion_handler.rs`), calls `try_auto_merge` a second time; by then
`merged` is true and the call only runs `cleanup_already_merged`.

`try_auto_merge` in order: auto-merge enabled check (stage, then plan, then daemon default),
phantom guard (`commits_ahead_of` must be > 0), record `completed_commit` from the branch head,
`MergeLifecycle::reconcile_overlay`, then `attempt_auto_merge` (`orchestrator/auto_merge.rs`), which calls
`merge_stage` (`git/merge/mod.rs`): take `MergeLock`, refuse if `MERGE_HEAD` exists, `git checkout
<target>` in the main checkout, `git merge --no-ff -m "Merge loom/<id> into <target>"`.

### Outcomes

| Outcome | Stage file afterwards | Daemon |
| --- | --- | --- |
| Merge succeeded and `verify_merge_succeeded` proves ancestry | `Completed`, `merged: true`; `finish_verified_merge` reconciles the base and removes worktree and branch | continues |
| Conflicts | forced to `MergeConflict`, resolver session spawned | stays alive for the resolver |
| `git merge` or checkout failed (dirty main checkout, lock timeout, missing branch) | forced to `MergeBlocked`, `failure_info` type `InfrastructureError` with the git error as evidence | last stage: exits; `loom status` shows MERGE ERROR |
| Merge ran but ancestry cannot be verified | forced to `MergeBlocked`, reason in `failure_info` | same |
| Branch has zero commits beyond target | forced to `NeedsHumanReview`, `review_reason` says the agent never committed | exits; NEEDS REVIEW |
| Auto-merge disabled for the stage or plan | stays `Completed + !merged` by design | exits; `loom status` shows unmerged |

`persist_merge_blocked` is the single writer for the blocked outcomes and `route_to_human_review`
for the review outcome; both use `force_status_with_reason` because `Completed` has no legal exits.
Nothing on the daemon path ever writes `merged: true` without `is_ancestor_of` returning true.

### Recovery from every non-merged outcome

`loom stage merge <id>`, run from the stage worktree, accepts `MergeConflict`, `MergeBlocked`, and
`Completed + !merged` (`commands/stage/merge/preflight.rs::require_merge_state`). It re-runs
`merge_stage` and, on success, `try_complete_merge`, which tolerates an already-Completed stage.
Until 2026-09-06 the command refused `Completed` stages while the daemon log and the status UI
both pointed at it, which is why operators fell back to `git merge loom/<id>` by hand.

The merge runs in the operator's main checkout. Tracked modifications or untracked files that the
merge would overwrite make `git merge` refuse, and that is the most likely cause when a
knowledge-distill stage, which touches `doc/loom/knowledge/**` and `README.md`, ends up blocked.

## Merge Lock (git/merge/lock.rs)

`MergeLock` serializes loom-driven merges with an exclusive OS advisory lock (`fs2::try_lock_exclusive`) on the stable `.work/merge.lock` inode. The file is created once and never unlinked; the holder's pid and timestamp are written into it for diagnosis only. `acquire` polls every 100 ms up to the caller's timeout (30 s from `merge_stage` and the probe). Release is by `Drop` or process exit, so there is no stale-lock reclamation and a pid left in the file after a merge is not a held lock. An earlier version of this section named `progressive_merge/lock.rs` and a five-minute stale sweep; neither exists.
