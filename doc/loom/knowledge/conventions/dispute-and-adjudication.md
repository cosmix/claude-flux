---
---
# Dispute And Adjudication

> Dispute file authority split, adjudicator scope, budgets, and transport

## Dispute File Ownership Convention

`.work/disputes/<stage>/<n>/` — always split by authority:

| File | Authority | Notes |
| --- | --- | --- |
| `.work/disputes/<stage>/<n>/request.md` | agent-attestable | written by the daemon on behalf of the agent's RPC, or drained from the worktree spool |
| `.work/disputes/<stage>/<n>/verdict.json` | adjudication session | the session's DRAFT, not the record — it has no authority until recorded |
| `.work/disputes/<stage>/<n>/verdict.md` | daemon-only | the record `apply_pending_verdicts` reads |
| `.work/disputes/<stage>/<n>/applied.marker` | daemon-only | zero-byte idempotency sentinel |
| `.work/disputes/<stage>/<n>/attempts` | daemon-only | respawn budget, spent when an adjudication job is handed out |

Never collapse these into one file. The rule behind the split is unchanged: **if the party under
dispute can write the verdict, it can self-approve.**

What changed on 2026-08-31 is WHO writes the verdict, so the guard moved rather than disappearing.
The adjudicating session is a different session from the disputing stage agent, and it records
through `loom stage adjudicate`, which refuses:

- when `LOOM_WORKTREE_PATH` is set — a stage worktree session can never record a verdict, which is
  precisely the self-approval case;
- when the stage is not in `NeedsAdjudication`;
- when the named dispute was never filed;
- when a `.work/disputes/<stage>/<n>/verdict.md` already exists — a recorded verdict is not replaceable.

A degenerate verdict escalates rather than being recorded. An earlier version of this section said `verdict.md` was written by "a worker thread after an API call"; there is no worker thread and no API call.

## Adjudicator Scope Convention

The adjudicator amends ONLY:

- `acceptance: Vec<AcceptanceCriterion>` (plan/schema/types.rs:316)
- `wiring: Vec<WiringCheck>` (plan/schema/types.rs:336)

Never amends: `before_stage`, `after_stage`, `artifacts`, `dependencies`, `id`, `working_dir`, `model`, `sandbox`, `execution`. Use `AmendmentField` enum to enforce this at the type level.

## Dispute Budget Limits Convention

Per-stage caps to bound the autonomy loop:

- `dispute_count`: max 3 per stage (default)
- `evidence_rounds` (NeedsMoreEvidence iterations): max 2 before escalation to NeedsHumanReview
- `amendments_applied`: max 3 per stage (absolute, not percentage)
- `adjudicator_attempt_count` (worker crash retries): max 3

## Adjudication Attempt Budget Convention

Respawn of an adjudication session is bounded by an on-disk `attempts` file in the dispute
directory, incremented when a job is handed out — the same shape as the merge-resolver attempt
counter. `MAX_ADJUDICATION_ATTEMPTS` is 3.

**The `.inflight` marker this section used to describe is gone**, along with the worker thread it
guarded. An adjudication session's liveness is read from the session registry
(`live_sessions_for_stage` filtered to `SessionType::Adjudication`, PID identity with start-time
verification), so there is no marker to go stale and no 10-minute freshness window to tune. That
also survives a daemon restart, which a process-local marker did not.

**A bug worth remembering, found when this was rebuilt:** the old attempt cap never fired once. `current_attempt_count` derived the count by reading `verdict.md` — a file that by construction does not exist on the branch where the cap is checked — so it always returned 0, and only the `.inflight` marker's freshness limited respawn at all. A counter derived from an artifact that the counted operation has not produced yet is not a counter. Spend the budget when the work is HANDED OUT, not when it succeeds.

## Daemon-as-Filesystem-Writer Convention

For any operation where agent data must be persisted to `.work/` with authority separation: the CLI sends RPC to daemon; the daemon writes the file. Examples:

- `loom memory note` → daemon writes `.work/memory/<id>.md`
- `loom stage dispute-criteria` (after Stage 2) → daemon writes `.work/disputes/<stage>/<n>/request.md`

## Adjudicator Transport Convention

Adjudication runs in a **spawned loom session**, the same way merge conflict resolution does. There
is no API key, no HTTP call, and no headless subprocess anywhere in the path.

**Corrected twice on 2026-08-31, so read the history before reintroducing either shape.** This
section first described an `ANTHROPIC_API_KEY` gate: absent key at daemon startup disabled
adjudication for the whole run and every dispute escalated to `NeedsHumanReview` — which killed the
feature for anyone on a subscription rather than an API key. It was then briefly a headless
`claude -p` subprocess driven from a worker thread. Both are gone. The owner's call: a dispute is
judged by a real session with the full tool surface, because that is what lets it check the tree and
run things.

- **Spawn:** `adjudication::session::start_pending_adjudications` builds a `SessionType::Adjudication`
  session and spawns it through the same `TerminalBackend` every other session goes through. It runs
  in the MAIN REPO, not a worktree — `SessionType::Knowledge` is the closest precedent.
- **Model:** `resolve_model` reads `.work/config.toml::[adjudication].model`, defaulting to `opus`.
- **Briefing:** `orchestrator/signals/adjudication.rs` writes a signal whose body comes from
  `adjudication::prompt`, carrying the dispute, the stage's criteria, the evidence commit diff and
  the failure output.
- **The session RUNS the disputed criterion.** This is the point of using a session rather than a
  one-shot call. `prompt::execution_site` resolves the stage's worktree root joined with its
  `working_dir` — the same `EXECUTION_PATH` acceptance criteria resolve against — and the briefing
  tells the session to execute the criterion there and rank the observed exit code above the
  disputing agent's account of it. Running it from the wrong directory makes a sound criterion look
  broken and inverts the verdict, which is why the site is computed rather than guessed.
- **It judges, it does not fix:** no edits, no commits, never `loom stage complete`. The single
  authorized execution is the disputed criterion itself — the same command the stage already runs,
  under the same sandbox, so it is not new exposure.
- **Return path:** the session writes its verdict JSON to `.work/disputes/<stage>/<n>/verdict.json` and
  runs `loom stage adjudicate --stage <id> --dispute <n> --verdict-file <path>`. The daemon's
  existing `apply_pending_verdicts` applies it on the next tick. Nothing waits on the session; the
  daemon observes the state change, exactly as it does for a merge resolution session.
- **Liveness and budget:** a live adjudication session is detected through
  `session_registry::live_sessions_for_stage` filtered to `SessionType::Adjudication` (PID identity
  with start-time verification), so it is correct across a daemon restart with no in-memory state.
  Respawn is bounded by an on-disk `attempts` counter in the dispute directory, spent when a job is
  handed out.
- `sandbox/settings.rs`'s `SENSITIVE_ENV_KEYS` still filters `ANTHROPIC_API_KEY` from agent sandbox
  environments. That remains correct hygiene and has nothing to do with adjudication.
