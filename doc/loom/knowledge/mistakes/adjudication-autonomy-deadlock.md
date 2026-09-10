# Adjudication Autonomy Deadlock

> An accepted verdict deadlocked the run: adoption by stage_id alone, requeue with unanswered disputes, a live disputing agent, and no watchdog.

## Adoption Matched Sessions by stage_id Alone

**What happened:** The adjudication session for stage `loom-dir-migration` carried the stage's own `stage_id` (`loom/src/models/session/methods.rs:59-65`), and the daemon's session-adoption path looked up live sessions for a stage by `stage_id` alone. The adjudicator, spawned after the real stage agent and therefore newer by `created_at`, was adopted as the stage's worker in place of the actual agent.

**Why:** `live_sessions_for_stage` filtered only on `stage_id` and a running/spawning status, never on `session_type`. Adjudication, knowledge, and merge-resolution sessions all carry the target stage's `stage_id` by design, so any of them is a first-class match for "the stage's worker" from that lookup's point of view.

**Prevention:** Wherever a stage's worker is looked up, filter by session kind. `stage_id` is not identity — a session belonging to a stage is not necessarily that stage's *worker*.

**Fix:** Added a typed lookup, `live_sessions_for_stage_of_type` (`loom/src/orchestrator/session_registry.rs`), driven by `worker_session_type` (`loom/src/orchestrator/coherence.rs`), which names the `SessionType` that counts as a stage's worker. `session_is_current_for_stage` (`loom/src/orchestrator/core/orphan_adoption.rs`) now excludes `Adjudication` explicitly.

## A Verdict Re-queued the Stage While Another Dispute Was Unanswered

**What happened:** Two disputes were filed against `loom-dir-migration` seconds apart. Answering the first dispute re-queued the stage unconditionally, which made the second dispute's request file (`.loom/work/disputes/<stage>/<n>/request.md`) permanently unschedulable — nothing ever put the stage back into `NeedsAdjudication`.

**Why:** `job_for_dispute` only hands out an adjudicator while `stage.status == NeedsAdjudication`, and `apply_accept` transitioned the stage straight to `Queued` on every accepted verdict without checking whether a sibling dispute still lacked its own verdict file at that same path.

**Prevention:** A stage can carry more than one unanswered dispute at a time. Any verdict-apply path must count the stage's remaining unanswered disputes before deciding where to transition, not assume its own verdict is the last one.

**Fix:** `requeue_or_hold_for_remaining_disputes` (`loom/src/orchestrator/adjudication/apply.rs`) counts request files (`.loom/work/disputes/<stage>/<n>/request.md`) with no sibling verdict file and only re-queues once none remain; a Reject verdict's `NeedsHumanReview` state is never overwritten by a later sibling verdict.

## The Disputing Agent Was Never Retired

**What happened:** Filing a dispute changed stage state but left the disputing agent's session running and idle. When the verdict later re-queued the stage, adoption picked that same idle session back up as the worker — so even with stage_id-only adoption fixed, a stage would still be bound to an agent with no idea the criteria changed.

**Why:** `handle_dispute` incremented `dispute_count` and requested adjudication but never touched the filing agent's session. Nothing retired it, and nothing woke it with the amended criteria either.

**Prevention:** A verdict-apply path needs an explicit answer for "what happens to the agent that filed the dispute?" Leaving it alive and re-queueing the stage hands its successor a stale, indistinguishable-looking session instead of a fresh start.

**Fix:** `retire_disputing_agents` (`loom/src/orchestrator/core/event_handler/stage_takedown.rs`) runs before every verdict apply: writes a `HandoffOrigin::Retired` handoff, kills the session, confirms it is gone, and clears `stage.session`. Verdict application defers if any such agent survives the takedown.

## Failed Adoption Mutated the Stage and Returned "Handled"

**What happened:** Adoption wrote the wrong session into `stage.session` and walked the stage to `Executing` before checking whether the in-memory active-session slot could actually accept it. The registration then refused (an ERROR log line, nothing else), and the caller read "adoption attempted" as "handled" and skipped spawning a real agent, leaving the stage mutation in place.

**Why:** The mutation and the registration check were split across two steps with no rollback between them, and the registration's failure signal (a log line) had no path back to its caller's return value.

**Prevention:** An adoption that cannot complete must not leave the stage claiming it did. Check before mutating, or roll the mutation back on failure, and escalate rather than leaving a silent no-op that looks identical to success.

**Fix:** the incumbent check now happens before mutation and escalates to `Blocked` on failure (`loom/src/orchestrator/core/session_adoption.rs`).

## No Watchdog Covered "Executing With Nobody Working"

**What happened:** The stage sat `Executing`, pointed at a non-worker session, for over ninety minutes. Neither of the daemon's two watchdog paths could fire: hung-session detection for the real agent's session compared against `stage.session`, which now named the adjudicator, so the ids never matched; the adjudicator's own session had no heartbeat file of its own to go stale.

**Why:** Both watchdogs keyed off `stage.session` or a per-session heartbeat file, and both assumed that field always names a live, correctly-typed worker. A corrupted pointer defeated both simultaneously, and the daemon logged only a benign-looking `Failed to sync graph status` warning every five seconds.

**Prevention:** Every non-terminal stage state needs either a live owner or a poll-time coherence check independent of that owner's own bookkeeping — never rely solely on paths that assume the pointer they are keyed on is already correct.

**Fix:** `reconcile_executing_stages` (`loom/src/orchestrator/core/coherence.rs`) runs each daemon tick and is also exposed through `loom repair`, checking directly whether an `Executing` stage's `stage.session` names a live, correctly-typed worker.

## The Completion Bridge Parsed a Wrapper the Harness No Longer Sends

**What happened:** For a 17.1KB verify output, the harness's actual hook input carried the full path in a structured `tool_response.persistedOutputPath` field, with `stdout` truncated to a prefix and no "Full output saved to:" wrapper text anywhere. `hooks/loom-control-complete.sh`'s persisted-output recovery only looked for that wrapper text, found none, and reported the verification marker missing — an operator had to complete the stage manually with `--no-verify`.

**Why:** The hook's fixture-driven recovery logic was written against an assumed shape of the harness's tool-output payload rather than a shape captured from a real transcript, and the harness's actual behavior had already diverged from it.

**Prevention:** Hook fixtures must be recorded from real transcripts (`~/.claude/projects/<project>/<session>.jsonl`, the `toolUseResult` field), never hand-written against an assumed shape.

**Fix:** `hooks/loom-control-complete.sh` now derives `PERSISTED_PATH` first from `tool_response.persistedOutputPath` / `tool_result.persistedOutputPath` via `jq`, falling back to the wrapper-text `sed` extraction only when that structured field is empty. Every downstream validity check on the resolved path — absolute, under `$HOME/.claude/projects/`, a `/tool-results/` segment, no `..` segment, a regular file, not a symlink — is unchanged.

## Four Silent Guards Made a 90-Minute Deadlock Invisible

**What happened:** Each of the four defects above declined to act without saying so anywhere an operator would see it: adoption logged an ERROR and returned success, the re-queue silently orphaned a dispute, the retired-agent gap left a session merely idle, and the watchdog compared against a pointer it had no way to know was wrong. `loom status` rendered the stage as a green, spinning, in-progress row the entire time, showing the adjudicator's own PID as if it were the worker's.

**Why:** None of the decline-to-act branches had a required "say so" path, and the status renderer had no way to distinguish "a stage-type worker is active" from "the stage's session field happens to be populated."

**Prevention:** Every decline-to-act branch in daemon logic should print to the daemon console rather than fail silently or log only at a level nobody watches live.

**Fix:** `loom status` now renders session kind and prefixes an incoherent `Executing` state with `INCOHERENT:` (`loom/src/commands/status/render/graph.rs`).

## The Judge Was Never Terminated

**What happened:** An adjudication session recorded its verdict with `loom stage adjudicate`, its prompt told it to stop, and the Claude Code process then sat open at its prompt until an operator closed the window by hand. Stage sessions are killed by the daemon on completion (`loom/src/orchestrator/core/completion_handler.rs`); judges had no equivalent.

**Why:** The verdict record carried no session id, so the daemon could not tell which judge wrote which verdict, and nothing in the apply path addressed the judge at all. While an idle judge lingers, `claim_session_slot` (`loom/src/orchestrator/adjudication/mod.rs`) refuses to spawn a judge for any further dispute on that stage.

**Prevention:** Every session kind the daemon spawns needs a recorded completion signal and a teardown that acts on it. Recording the verdict is the judge's completion signal; the record must name the session that produced it.

**Fix:** `DisputeVerdictRecord.session_id` (`loom/src/models/dispute.rs`) is filled from `LOOM_SESSION_ID` by `loom stage adjudicate`; after applying a verdict, `retire_adjudicator` (`loom/src/orchestrator/core/verdict_apply.rs`) kills that session, declares its record `Completed`, and removes its signal. A record without an id closes idle judges only when the stage has no unanswered dispute left.

## A Silent Judge Had No Watchdog

**What happened:** Two operator restarts today were needed for adjudication to proceed. After those root causes were fixed — a verdict never applied, a judge never closed — the remaining way a dispute could stall forever was a judge that stays alive but never records a verdict. Nothing detected it: hung detection (`loom/src/orchestrator/monitor/detection.rs`) keys on the stage's own heartbeat file, which names the stage's worker session, so a judge always read as `NoHeartbeat` and was skipped. Judges wrote no heartbeat because `hooks/post-tool-use.sh` refuses to write unless the stage file's `session:` field names the writer, which is never the judge. The attempt budget (`MAX_ADJUDICATION_ATTEMPTS = 3`, `adjudication/session.rs`) only bounded judges that had already died, since `live_adjudication_session` refuses to spawn a second judge while one is alive.

**Why:** Every watchdog assumed the only session worth watching was the stage's worker, and every completion signal for a judge was its verdict; a judge that never produced one had no signal to go stale.

**Prevention:** Every session kind the daemon spawns needs its own liveness signal and its own idle budget, independent of the stage's worker bookkeeping.

**Fix:** The session wrapper exports `LOOM_SESSION_TYPE` (`terminal/native/wrapper.rs`); the PostToolUse hook writes a judge heartbeat to `heartbeat/<stage>.adjudication.json` for `LOOM_SESSION_TYPE=adjudication`, skipping only the ownership gate; `HeartbeatWatcher` keeps judge heartbeats in a separate map (`monitor/heartbeat.rs::judge_heartbeat`); `Detection` emits one `MonitorEvent::AdjudicatorStalled` per judge whose last activity (heartbeat, else `created_at`) exceeds the stage's `subagent_timeout_secs` idle budget while the process is alive (`monitor/hung_latch.rs`); the handler (`core/event_handler/stalled_judge.rs`) closes the judge through the shared `Orchestrator::close_adjudication_session` (`core/judge_close.rs`, also used by `retire_adjudicator`) and leaves the stage in `NeedsAdjudication`, so the next tick respawns a judge or `escalate_attempt_cap` sends the stage to `NeedsHumanReview`. A vanished judge whose verdict is already on disk is now recorded as `Completed`, not crashed (`AdjudicatorRegistry::verdict_written_by`, `monitor/session_events.rs`).

## The Verdict Hold Was Logged as a Forced Transition

**What happened:** Every verdict apply logged `Forced stage status assignment bypassing transition validation` at ERROR, including the no-op hold `NeedsAdjudication -> NeedsAdjudication`, because `adjudication/apply.rs::persist_verdict_result` forced the status unconditionally.

**Why:** The force was a crash-recovery safety net applied to the routine path.

**Prevention:** A no-op status write should be skipped, and a transition the table already validates (`NeedsAdjudication -> Queued` is in `models/stage/transitions.rs`) should go through `try_transition`; force is the fallback only for a transition the table does not know.

**Fix:** `persist_verdict_result` now does exactly that.

## The Per-Tick Graph Sync Warning Buried the Adjudication Lines

**What happened:** `recovery.rs` called `ExecutionGraph::mark_executing` on every tick for an Executing stage; the graph refuses it once the node is already Executing, so the daemon logged `Failed to sync graph status ... is not ready (status: Executing)` every 5 seconds — 1441 lines around 14 adjudication lines in one daemon log.

**Why:** The sync path did not check the node before re-marking it.

**Prevention:** Consult `graph.get_node` before a transition that is only valid from `Queued`.

**Fix:** The Executing arm now skips `mark_executing` when the node is already Executing.

## The Judge Never Ran the Heartbeat Hook

**What happened:** the judge watchdog added earlier today (`heartbeat/<stage>.adjudication.json`, written by `hooks/post-tool-use.sh` when `LOOM_SESSION_TYPE=adjudication`) never saw a heartbeat from a live judge: the wrapper exported the variable, the installed hook handled it, an isolated reproduction wrote the file, yet the real judge made thirteen tool calls and the heartbeat directory's mtime never moved. The hook was never registered for the judge. Stage sessions run in a worktree whose `.claude/settings.local.json` is generated by `hooks::generator::setup_hooks_for_worktree` and carries the session lifecycle hooks; a judge runs in the main repository, and `session_capsule` (`orchestrator/terminal/native/capsule.rs`) resolved `<cwd>/.claude/settings.local.json`, the operator's loom-managed file, whose PostToolUse set is `ask-user-post.sh` and `loom-control-complete.sh` only.

**Why:** the hook set a session received was tied to WHERE it runs (worktree or main repo), not to WHAT it is. Every session kind except stage inherited the main repo's file and therefore no lifecycle hooks.

**Prevention:** when adding a per-session signal that a hook produces, verify the hook is registered for every session kind that must produce it, by reading the settings file the capsule actually passes with `--settings`, not by running the hook by hand. A passing isolated reproduction proves the hook, not the wiring.

**Fix:** `orchestrator/terminal/native/session_settings.rs` builds `<work_dir>/capsules/<session-id>.settings.json` for adjudication sessions from the cwd's `settings.local.json` plus the PostToolUse `post-tool-use.sh` entry (same shape `HooksConfig::to_settings_hooks` emits), scrubs stale `env` keys, and `capsule_for` passes it through the capsule; `close_adjudication_session` removes it. Other session kinds resolve the cwd's file exactly as before.

## Approve Assumed the Agent Was Still There

**What happened:** `loom stage human-review <stage> --approve` moved the stage `NeedsHumanReview -> Executing` and reset `fix_attempts`, assuming the stage's agent was alive and waiting. Every path into `NeedsHumanReview` is an adjudication escalation, and the daemon retires the disputing agent before applying any verdict, so there was no session; the coherence check found "Executing with no session assigned" and blocked the stage with an infrastructure error, which is not auto-retried. The operator had to run `loom stage retry`, which queues the stage for a fresh session, which is what approve should have done.

**Why:** the transition table had no `NeedsHumanReview -> Queued` edge; the approve command predates the adjudication flow and the agent-retirement rule.

**Prevention:** a command that resumes a paused stage must decide who does the work. If the stage's session may have been retired, the resume path is `Queued` (the daemon spawns or adopts), never `Executing`.

**Fix:** `models/stage/transitions.rs` allows `NeedsHumanReview -> Queued`; `Stage::try_approve_review` transitions there; `handle_force_complete` transitions to `Executing` directly and clears `review_reason` itself; the orphaned `try_force_complete_review` was removed.

## The Next Step Was Rendered Only Under --verbose

**What happened:** the operator saw `⏸` plus verdict prose in `loom status` and had no idea what to do. The "⚠ Requires Attention" block that names a next step (`commands/status/render/attention.rs`) existed but was rendered only under `--verbose`, and its hint for `NeedsHumanReview` named `loom stage resume`, a hook-only entry point. The daemon's `REVIEW NEEDED:` line and the desktop notification carried the prose only, and the reject reason pointed at `.loom/work/disputes/...`, a path that does not exist in a legacy `.work/` layout.

**Why:** guidance was added to the one surface an operator in trouble reads least, and each surface was written without checking that the command it named existed.

**Prevention:** when a state stops for a human, every surface that announces it (status row, attention block, daemon console, notification, the stored reason) names the exact command and the choices; test the hint text against the real subcommand.

**Fix:** the attention block renders whenever a stage needs a decision (`--verbose` now expands only the `Evidence:` listing); its `NeedsHumanReview` hint is `loom stage human-review <id>` followed by the three choices (`--approve` queues a fresh session, `--force-complete`, `--reject <reason>`); the daemon line and notification add `Next: loom stage human-review <id>`; the reject reason carries the absolute path to the verdict file (`.loom/work/disputes/<stage>/<n>/verdict.md`) and the command.

## The Daemon Shut Down While the Last Stage Was Being Re-queued

**What happened:** `knowledge-distill`, the last incomplete stage of a plan, filed a dispute. The judge's verdict was applied, which moved the stage on disk from `NeedsAdjudication` to `Queued`, and the judge was closed, which emptied `active_sessions`. On that same tick, before `sync_graph_with_stage_files` had re-marked the graph node, `all_stages_terminal` (`loom/src/orchestrator/core/recovery.rs`) read the node's stale `NeedsAdjudication`, treated it as terminal, printed "All stages are in terminal state", and the daemon exited with the stage sitting in `Queued` and nothing to spawn it.

**Why:** the shutdown decision trusted the in-memory graph, which lags the stage file by up to a tick after any verdict, approve, retry, or amendment, and it counted `NeedsAdjudication` as terminal even though a stage waiting for a judge needs the daemon alive to spawn, watch, and close that judge.

**Prevention:** a decision to exit the daemon reads the stage files, never the graph, and no state that still needs the daemon (a judge to spawn, a queued stage to start) is terminal.

**Fix:** `all_stages_terminal` short-circuits only on graph `Completed`/`Skipped` and otherwise decides from the stage file through the pure `stage_file_is_terminal` (`recovery.rs`), an exhaustive match: `NeedsAdjudication`, `Executing`, `WaitingForInput`, `NeedsHandoff`, and an unheld `Queued`/`WaitingForDeps` are never terminal; `Blocked` is terminal only when no retry is pending; `NeedsHumanReview` and the merge-failure states stay terminal. `recovery_terminal_tests.rs` pins the incident shape (graph `NeedsAdjudication`, file `Queued`, no sessions).

## A Prompt Named a Rust Type as Its Schema and Every Judge Guessed the Same Wrong Shape

**What happened:** the adjudicator prompt described the accept-only field as
`"plan_patch": { ...AmendmentRequest JSON... }`. Three independent adjudication sessions (two
Opus, one Sonnet, judging the same criterion hours apart) all emitted the same FLAT shape —
`{"field", "op", "index", "value"}` — instead of the nested `{"field", "patch": {"op", ...}}`
shape `AmendmentRequest` decodes. Each accept verdict then failed to apply forever: the daemon
retried every ~5s (1,363 times in one run) logging only `plan_patch missing 'patch' object`,
while `loom status` still showed the stage as merely `ADJUDICATING`.

**Why:** three compounding causes, only the first of which is about the prompt.

- A schema stated as a type name is not a schema. The model cannot see `AmendmentRequest`, so it
  invents the shape the surrounding prose implies — and different models invent the SAME shape,
  which is why "the LLM got it wrong once" is the wrong reading of this class of bug.
- `validate_accept` checked only that `plan_patch` existed and was non-null, storing it as opaque
  JSON. The shape was first decoded at APPLY time, inside the daemon, where the only recourse is
  a `warn!`. Every other malformed verdict shape was already coerced to `NeedsMoreEvidence` and
  re-prompted; this one had no such path.
- The failing apply had no attempt cap, and `verdict_apply.rs` retires the stage's agents before
  every attempt — so a re-queued stage had its new session killed within ~5s. The wedge was
  unrecoverable without hand-editing `.loom/work`.

**Prevention:** an agent-supplied structure gets its shape validated at the TRUST BOUNDARY where
a re-prompt is still possible (verdict recording), never deep in the daemon where the only move
is a log line. Any daemon loop that retries a fixed input needs a counter and an escalation —
"retry forever" and "kill the stage's sessions each pass" compose into a silent permanent wedge.
And when a prompt asks for JSON, the prompt states the JSON, inline, with an example.

**Fix:** `orchestrator/adjudication/plan_patch.rs` normalises both shapes (so verdicts already
stuck on disk self-heal on the next tick); `validate_accept` decodes at record time and coerces
a malformed patch to `NeedsMoreEvidence`, storing the canonical nested form; `apply_verdict`
counts failures per dispute and escalates to `NeedsHumanReview` at `MAX_APPLY_ATTEMPTS`; the
prompt spells the schema out.
