---
---
# Merge And Recovery

> Progressive merge, conflict recovery, attribution, dispute files.

## Progressive Merge Pattern

Dependencies merged to main before dependent stages execute: `Stage A completes -> Merge A to main -> Stage B starts`. Base branch resolution: no deps = init_base_branch or default; all deps merged = main; single dep not merged = dependency branch (legacy fallback). MergeLock prevents concurrent merges (30s timeout, 5min stale cleanup).

## Merge Anti-Respawn Pattern

When merge conflict session dies unresolved: session removed from `active_sessions`, signal file KEPT as anti-respawn guard. `spawn_merge_resolution_sessions()` checks `has_merge_signal_for_stage()` before spawning. Signal removed only when merge succeeds.

## Merge Recovery Flow [UPDATED 2026-04-27]

MergeConflict -> bail\!() forces original session to exit -> commit-guard.sh allows exit for MergeConflict status -> detection.rs recognizes as normal exit -> spawn_merge_resolution_sessions() kills any stale original session, then spawns resolver -> merge signal includes "Inherited Responsibilities" section explaining resolver owns the stage -> user directed to `loom stage merge <stage-id> --resolved`.

Key invariant: the original execution session MUST exit when merge conflict is detected. Three mechanisms enforce this:

1. `bail\!()` in `complete_with_merge()` propagates error and terminates the session
2. `commit-guard.sh` does NOT block exit for MergeConflict status
3. `spawn_merge_resolution_sessions()` actively kills stale sessions before spawning resolver

**Daemon ordering invariant (2026-04-27):** Reconciliation runs BEFORE `sync_graph_with_stage_files` AND BEFORE `recover_orphaned_sessions`. Recovery deletes orphaned merge session files; attribution depends on their metadata. Sync reads stage files into the graph; if reconcile flips disk state AFTER sync, the graph keeps the stale view and would queue dependents based on a phantom merge.

**Daemon-off CLI parity (2026-04-27):** `loom stage complete` on a `Completed + merged=true` stage with an active main-repo merge attributed to it triggers the same revert the daemon performs (`Completed → MergeConflict + merged=false + merge_conflict=true`) before spawning the resolver. The router's `RevertAndSpawnResolver` arm encodes this; persistence is the caller's responsibility, BEFORE spawn so `spawn_merge_resolver`'s status contract is satisfied.

## Attribution-Aware Recovery (2026-04-27)

`MERGE_HEAD` in the main repo is global state — only one merge in progress at a time across all stages. Stage-state mutation triggered by detecting it must come with proof of attribution; without proof, refuse rather than mutate.

Three attribution sources (first match wins):

1. **MergeSession metadata** — orphaned or live `SessionType::Merge` with matching `merge_source_branch`.
2. **Branch HEAD match** — a `MERGE_HEAD` SHA equals `loom/<stage-id>` HEAD.
3. **Completed-commit match** — a `MERGE_HEAD` SHA equals `stage.completed_commit`.

**BaseConflict carve-out:** When current HEAD is `loom/_base/*` (or any session has `SessionType::BaseConflict` matching it), return `GlobalUnattributed` even if the merge heads contain a stage branch's commit. Multi-dependency base merges check out their own branch and run a merge there; their MERGE_HEAD must NOT mutate stage state.

Single decision point: `attribute_main_repo_merge` in `orchestrator/merge_attribution.rs`. Both daemon recovery (`reconcile_main_repo_active_merge`) and the CLI router consume it.

## Pure Routing Helper (2026-04-27)

`route_complete_for_conflicts` is the canonical example: read-only function that returns `CompleteConflictRoute` without writing to disk. Persistence is the caller's responsibility on the success path only. This preserves the "refusal preserves stage file state" invariant — refusal always leaves the stage file untouched, which is critical for tests and for users investigating why a completion attempt was rejected.

Apply this pattern when adding routing/verification helpers: keep the function pure, return an enum of decisions, let the caller persist on the success branch.

## No Worker Thread for Adjudication (Retired Pattern)

**Retired 2026-08-31. Do not reintroduce this for adjudication.** This section described a
worker-thread + mpsc channel the adjudicator used to drive a blocking call off the orchestrator's
poll loop: `worker_completion_tx`/`_rx` on the `Orchestrator`, a `std::thread::spawn` per dispute,
and a drain in the main tick. All of it is deleted: the adjudication worker and
client modules were removed outright, so there is no thread to reintroduce it into.

Adjudication now spawns a real loom session through the `TerminalBackend` and observes the
resulting state change on the ordinary poll loop, the same way merge conflict resolution has always
worked. That removed the thread registry, the mpsc channel, the cooperative cancellation flag, the
`.inflight` staleness marker, and the retry/backoff loop in one go — a session is already a tracked,
restart-surviving, externally-observable unit of work, and the thread machinery was reimplementing a
worse version of it.

The general lesson, if you are reaching for a worker thread in the orchestrator: ask first whether
the work is a SESSION. Loom already knows how to spawn one, track its liveness across a daemon
restart, and notice when the state it was supposed to change has changed. A thread gets you none of
that and costs you a shutdown story.

## Dispute File Authority Split Pattern

Three-file trust boundary to prevent self-approval attacks:

| File             | Writer                             | Content                  | Rationale                                            |
| ---------------- | ----------------------------------- | ------------------------ | ---------------------------------------------------- |
| `.work/disputes/<stage-id>/<n>/request.md`     | Daemon (on agent's behalf via RPC) | Agent's evidence payload | Agent can read but never write directly              |
| `.work/disputes/<stage-id>/<n>/verdict.md`     | Daemon worker thread only          | Verdict + citations      | Stage agents never write here — daemon-authored only |
| `.work/disputes/<stage-id>/<n>/applied.marker` | Daemon only (zero-byte)            | Idempotency guard        | Prevents re-application on restart                   |

If the agent could write both request and verdict, it could pre-fill `verdict: Accept` and self-approve. The split enforces the trust boundary at the filesystem level.

## Plan Amendment Atomic Write Pattern

For amending the IN_PROGRESS plan file safely (Stage 3):

```text
1. Acquire .work/plan_versions/.lock  (file lock — serializes concurrent amendments)
2. Compute new plan content in memory
3. Atomic-write .work/plan_versions/<n>.md  (full snapshot)
4. Append to .work/plan_versions/audit.md  (O_APPEND — atomic for small rows)
5. Atomic temp+rename of IN_PROGRESS plan file to new content
6. Release lock
```

Recovery on crash: scan audit.md for latest amendment; verify plan file matches snapshot. If mismatch → restore from `<n>.md`. If `<n>.md` missing → discard audit row, use `<n-1>.md`.

Note: `plan/graph/loader.rs:60-86` PREFERS `.work/stages/` files over the plan file. Plan-file amendment MUST also update the corresponding `.work/stages/<stage_id>.md` for the change to be reflected in the running orchestrator graph.
