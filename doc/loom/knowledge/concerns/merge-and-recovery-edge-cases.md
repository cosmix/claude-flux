# Merge And Recovery Edge Cases

> Merge/retry/completion edge cases: phantom merges, stale started_at, nonces

## BranchMissing Phantom-Merge Risk in merge_handler.rs (2026-04-16)

`handle_merge_session_completed` at line 97-103 treats `MergeState::BranchMissing` as a successful merge by calling `finalize_merge_resolution` which unconditionally sets `merged=true`. This violates the project invariant that daemon-side paths must never write `merged=true` without git ancestry verification.

Scenario: merge session dies, `check_merge_state` returns Conflict/Unknown, branch was deleted without being merged (e.g., manual `git branch -D`), code assumes "branch missing = cleaned up after merge."

Pre-existing issue, not introduced by the merge conflict session lifecycle fix. The `ProgressiveMergeResult::is_success()` method also still classifies `NoBranch` as success, inconsistent with `progressive_complete.rs` treating it as `Blocked`.

## BaseConflict Carve-out is Heuristic (2026-04-27)

`attribute_main_repo_merge` carves out `loom/_base/*` merges with a heuristic on the current branch name and on `SessionType::BaseConflict` session metadata. If a base-merge ever runs from a non-`loom/_base/*` branch (manual flow, future refactor) and no `BaseConflict` session is alive, attribution would tie the active merge to the stage whose branch HEAD shows up in `MERGE_HEAD` — leading to a spurious revert.

**Hardening path:** Tag base merges explicitly via session metadata (e.g., a marker file or distinct `SessionType::BaseConflict` always present during the base-merge window) and key the carve-out off that signal alone, not the current branch name. Until then, the heuristic is documented here so future work knows where to look.

## Recovery: `retry --force` races daemon orphan-recovery on existing worktree (2026-05-13)

**Observed:** `loom stage retry --force --context "..."` correctly set `integration-verify` to `Queued`, but on the next daemon poll the orphan-recovery routine in `orchestrator/core/recovery.rs:638-705` saw the (now-stale) session_id, found commits-ahead-of-base on the worktree branch, and immediately re-routed the stage to `NeedsHandoff` (commits_ahead path at `recovery.rs:668`). To the user, the stage looked stuck — they typed `retry`, it was ready for a second, then back to a handoff state with no agent activity.

This is a logically defensible design (commits exist, don't burn tokens redoing them), but the user-visible interaction is confusing. The "fix" — using `retry --force` a _second_ time after acknowledging the handoff — is undocumented in the recovery flow.

**What's needed (pick one or both):**

- `retry --force` should clear `stage.session` before saving, so subsequent orphan recovery doesn't treat the prior session as live and doesn't rerun its decision tree.
- Orphan-recovery should respect a recently-saved "retry intent" marker (e.g., a timestamp on the stage indicating user-driven retry within the last poll interval) and skip its commits-ahead reroute for those.

**Where to look:**

- `commands/stage/skip_retry.rs` (the `retry` command sets Queued at line 122 but leaves `stage.session` populated)
- `orchestrator/core/recovery.rs:633-707` (the orphan-recovery decision tree that re-routes to `NeedsHandoff`)

## Status Dashboard: `started_at` not refreshed on retry, stage appears "stale/orphaned" (2026-05-13)

**Observed:** After a successful `loom stage retry --force` that spawned a fresh session, the status dashboard rendered `integration-verify` as `19h4m · 🔄 · orphaned (stale)` for the duration of the new attempt. The number came from the original (long-dead) `started_at`; the new session was actually `Up About a minute` in podman and actively making tool calls.

**What's needed:** `stage_executor`'s spawn path (or `retry`) should reset `stage.started_at` to `Utc::now()` when a new session is created. The dashboard's "stale" heuristic should key off the new attempt, not the cumulative duration.

**Where to look:**

- `commands/stage/skip_retry.rs::retry` (where retry mutates stage fields)
- `orchestrator/core/stage_executor.rs:291-293` (`begin_attempt(Utc::now())` is already called here — confirm it's the only writer of `started_at` and that it's reached on retry).
- The "stale" indicator emitter — likely in `commands/graph/indicators.rs` or a dashboard renderer.

## Completion Broker: Nonce Burn After Transition Can Return Err for a Landed Completion (2026-08-09)

`daemon/server/control_complete.rs::handle_complete_stage` consumes the replay nonce AFTER
`update_stage(...).try_complete(...)`. This ordering is deliberate: a daemon crash between the
transition and the burn is benign — a replay of the same nonce is rejected by
`validate_active_identity` because the stage is no longer `Executing`, so no completion can be
duplicated, and (unlike the old burn-first ordering) none can be lost to a pre-effect burn.

The residual edge introduced by the reorder: a genuine IO failure on the replay-marker directory
(disk full, permissions) occurring after the transition makes the handler return `Err` for a
completion that durably landed. Self-healing in practice — the stage file is `Completed` on disk
and the daemon reads state from disk — but the caller observes a false negative for a succeeded
operation. If this is ever observed in the wild, split the response so callers can distinguish
"completed, replay marker failed" from "not completed".

## Merge Path Follow-Ups After the Silent-Unmerged Fix (2026-09-06)

Found while fixing the silent `Completed + !merged` outcome (`mistakes/phantom-merges.md`, last entry). Each is a separate change and was left as is.

- `spawn_merge_resolution_sessions` (`orchestrator/core/merge_handler.rs`) still exempts probe failures from `MAX_MERGE_RESOLVER_ATTEMPTS`. A non-final stage now reaches `MergeBlocked` after a failed auto-merge, so a permanently dirty main checkout produces a "Failed to spawn merge resolution session" warning every 5 s until the daemon exits. The 2026-08-17 entry in phantom-merges.md already asks for a cap or an escalation path.
- `loom stage merge` requires the cwd to be inside `.worktrees/` (`commands/stage/merge/preflight.rs::resolve_worktree_paths`). A stage whose worktree is gone but whose branch is unmerged has no loom command that merges it, and the hints printed by the daemon and `loom status` do not say to cd first.
- `verify_merged_true_or_revert` (`orchestrator/core/recovery.rs`) treats a git error from `verify_merge_succeeded` as "not verified" (`unwrap_or(false)`) and reverts `merged` to false, so a transient git failure can flip a merged stage to unmerged.
- `merge_stage` (`git/merge/mod.rs`) checks out the target branch in the operator's main checkout and, on success, leaves it there; only the failure paths restore the original branch.
- `try_auto_merge` is 228 lines against the 50-line function cap and is ledgered at that size.
