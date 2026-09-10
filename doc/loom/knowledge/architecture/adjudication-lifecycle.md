# Adjudication Persistence and Stage Resumption

> How a dispute becomes a durable verdict, how each verdict changes stage state, and exactly what a fresh successor receives.

## Durable Artifact Chain

Paths below are relative to `.loom/work/`, the run's state directory:

| Path | Authority | Purpose |
| --- | --- | --- |
| `disputes/<stage>/<n>/request.md` | daemon on behalf of the stage agent | Original criterion index, reason, evidence commit, captured failure output, fix-attempt count, and timestamp |
| `disputes/<stage>/<n>/verdict.json` | adjudication session | Draft passed to `loom stage adjudicate`; not authoritative |
| `disputes/<stage>/<n>/verdict.md` | guarded adjudication record path | Immutable YAML-frontmatter record containing the verdict, reasoning or questions, citations, plan patch, model, attempt count, timestamp, and judge session id |
| `disputes/<stage>/<n>/.applying` | daemon | Temporary crash-recovery marker while a verdict is being applied |
| `disputes/<stage>/<n>/applied.marker` | daemon | Zero-byte idempotency sentinel proving that the verdict was applied |
| `disputes/<stage>/<n>/attempts` | daemon | Adjudication-session respawn budget, spent when work is handed out |
| `disputes/<stage>/feedback.md` | daemon | The one adjudication message that may be injected into a later stage signal |

`loom stage adjudicate` validates the draft `verdict.json` and records it as
`disputes/<stage>/<n>/verdict.md`. The daemon's
`orchestrator/adjudication/apply.rs::apply_pending_verdicts` scans verdicts without an
`applied.marker`, applies them idempotently, updates the stage file under `stages/`, then writes
the marker. A daemon restart can therefore resume an interrupted apply from the same record.

## Retirement and Fresh-Session Boundary

Before applying a verdict, `orchestrator/core/event_handler/stage_takedown.rs::retire_disputing_agents`
writes a `HandoffOrigin::Retired` handoff, kills and confirms the death of every non-adjudication
agent attached to the stage, and clears `stage.session`. If any writer survives, verdict application
is deferred. This prevents the old, idle agent from being adopted after the criteria change.

Once all sibling disputes have verdicts, an `Accept` or `NeedsMoreEvidence` result moves the stage
to `Queued`. The normal executor then creates a fresh session in the existing worktree. Its signal
contains the updated `Stage`, the eligible predecessor handoff from `handoffs/`, the latest stage
memory, and—when present—`disputes/<stage>/feedback.md`.

The successor does **not** load `disputes/<stage>/<n>/request.md` or `disputes/<stage>/<n>/verdict.md`
directly, and it does not inherit the judge's conversation. Verdict application must materialize
every fact the successor needs into one of the signal inputs above.

## What Each Verdict Delivers

| Verdict | Durable effect | What a later stage session sees |
| --- | --- | --- |
| `Accept` | Applies the verdict's patch to `acceptance` or `wiring`; updates the active plan and stage file; writes `plan_versions/<n>.md` and a `plan_versions/audit.md` row; clears stale feedback | Amended acceptance/wiring rendered by the normal signal, plus the predecessor handoff and stage memory. The full adjudicator reasoning remains in `disputes/<stage>/<n>/verdict.md`; it is not injected into the successor signal. |
| `NeedsMoreEvidence` | Writes the judge's questions to `disputes/<stage>/feedback.md`, increments `evidence_rounds`, and re-queues unless another dispute is unanswered or the evidence cap is exhausted | The questions appended at the end of the signal under `## Adjudicator Feedback (from your prior dispute)` |
| `Reject` | Writes reasoning and citations to `disputes/<stage>/feedback.md` and moves the stage to `NeedsHumanReview` | No automatic rerun. If a human later approves a fresh attempt, the persisted rejection feedback is available to its signal. |

An accepted amendment's audit row stores the dispute id and the optional reason inside
`plan_patch`; that reason is not necessarily the verdict's full `reasoning` field. For the complete
adjudication record, use `disputes/<stage>/<n>/verdict.md`.

## Source Path

1. `commands/stage/adjudicate.rs::record_verdict` establishes the guarded verdict record.
2. `orchestrator/adjudication/session.rs::persist_verdict` writes `disputes/<stage>/<n>/verdict.md`.
3. `orchestrator/core/verdict_apply.rs::apply_pending_verdicts` retires the old agent, then delegates application.
4. `orchestrator/adjudication/apply.rs::persist_verdict_result` writes the materialized stage state and `applied.marker`.
5. `orchestrator/adjudication/feedback.rs` owns the transient successor-facing feedback file.
6. `orchestrator/core/stage_executor.rs::start_stage` selects the handoff and calls `orchestrator/signals/generate.rs::generate_signal_with_skills`, which appends feedback last.
