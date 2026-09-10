# Codex Heartbeat Starvation

> Heartbeat starvation from long codex runs; stale-badge constant mismatch

## Long Codex Runs Starve the Loom Heartbeat (2026-08-07)

A foreground codex-lane run (`loom-codex-forwarder`) is ONE Bash tool call that blocks until codex returns. The
session heartbeat (`.work/heartbeat/<stage-id>.json`) is refreshed by three writers, all shell
hooks — `hooks/session-start.sh:61-72` (initial), `hooks/post-tool-use.sh:66-91` (after every tool
use), and `hooks/subagent-stop.sh:158-179` (after every `SubagentStop`) — registered at
`loom/src/hooks/config.rs:49-57` (script-name mapping) and `:228-241` (the `SubagentStop` hook
rule itself). No Rust production code writes a heartbeat (`write_heartbeat`,
`monitor/heartbeat.rs:264`, has only test callers). PostToolUse cannot fire until the Bash call
returns, so a codex run longer than the stage's budget makes the daemon print `appears hung` for a
stage that is perfectly healthy.

**Update (2026-08-27) — partly closed, not fully.** `hooks/subagent-stop.sh` is new: it refreshes
this same heartbeat file on every `SubagentStop`, with `activity: "subagent <agentId> finished"`.
**Closed:** the window where a parent session blocked on Task-tool subagents ran no tools of its
own, went silent on PostToolUse, and got reported `appears hung` while behaving perfectly — each
subagent completion now refreshes the heartbeat. **NOT closed:** the codex case this section is
named for. A foreground codex forward is still ONE blocking Bash call with no subagent underneath
it — neither PostToolUse nor SubagentStop can fire until it returns, so a single codex run longer
than the stage budget still produces a spurious `appears hung`. The section's headline finding
stands for that case; only the Task-subagent-wait case above it was fixed.

Budget: `DEFAULT_HUNG_TIMEOUT_SECS = 300` (`monitor/heartbeat.rs:21`), overridable per stage with
`subagent_timeout_secs` -> `Stage::effective_subagent_timeout_secs()` (`models/stage/methods.rs:107-110`),
resolved at `monitor/detection.rs:475-488`. `MonitorConfig::hung_timeout` (`monitor/config.rs:17,29`)
is only the fallback for a session whose stage cannot be resolved by id.

**`MonitorEvent::SessionHung` is ADVISORY ONLY.** One emit site (`monitor/detection.rs:505-511`),
one match arm (`orchestrator/core/event_handler.rs:187-209`) that is a `clear_status_line()` plus a
single `eprintln!` — the code carries the comment _"ADVISORY ONLY: nothing is killed and nothing is
retried."_ It warns ONCE per session (dedupe set `reported_hung_sessions`, `detection.rs:48`,
cleared on a fresh beat at `:456-457` and on `Healthy` at `:521`). Contrast the siblings that DO
act: `SessionCrashed` (`event_handler.rs:153`), `SessionNeedsHandoff` (kills + re-queues, `:110`),
`BudgetExceeded` (`:218`). Nothing kills, retries, or transitions a stage on SessionHung — the
warning is noise, not damage.

**Mitigation is doctrine, not a monitor change.** Keep each codex task bounded, and set
`subagent_timeout_secs` on stages that legitimately block for longer. CLAUDE.md Rule 6 ("Checking
on subagents") routes the check through the one-background-watch pattern — it blocks until
every subagent settles or the timeout fires, exits 0 vs. 2, and states which branch fired, which
alone satisfies the bounded-check rule — and tells the orchestrator to keep waiting while the subagent
reports `tool-wait`/`generating`: takeover or re-assignment needs positive evidence of death
(idle past the budget with NO transcript growth), never elapsed time alone (revised 2026-08-14;
the earlier wording told orchestrators to take work over at the deadline, which duplicated live
work). **Revised again 2026-08-27:** the doctrine now names an actual evidence channel —
`loom subagents list`/`harvest` report per-subagent state (`done`, `tool-wait`, `generating`,
`unknown`) read from each subagent's own transcript, so "positive evidence of death" is no longer
a judgment call the orchestrator has to make from silence.

**Deliberately OUT OF SCOPE: raising `MonitorConfig::hung_timeout`.** A global raise would blind the
monitor to genuinely dead sessions on every other stage in order to silence a cosmetic warning on
one lane, and the per-stage override already covers the real case. Do NOT "fix" this by editing the
default — it was considered and rejected as disproportionate.

## `loom status` "Stale" Badge Is Not Stage-Aware (2026-08-07)

Two independent 300s constants with different consumers:

- **detection** — `orchestrator::monitor::heartbeat::DEFAULT_HUNG_TIMEOUT_SECS`
  (`monitor/heartbeat.rs:21`), per-stage overridable via `subagent_timeout_secs`.
- **display** — `models::constants::STALENESS_THRESHOLD_SECS` (`models/constants.rs:37`), hardcoded
  at `commands/status/data/collector.rs:49` and `commands/status/render/activity.rs:21`.

`subagent_timeout_secs` reroutes only the detection one. A stage with `subagent_timeout_secs: 900`
stays healthy to the orchestrator until 900s but renders `Stale` / "session may be hung" in
`loom status` from 301s — which can push an operator into intervening on a healthy stage.

Flagged twice (implementation, then confirmed by integration-verify) and NOT fixed on purpose:
`determine_activity_status(session, staleness_secs)` takes no `Stage`, so making it stage-aware
means threading the effective timeout through that call site AND the render path — a status
subsystem change with its own test surface, unrelated to the codex lane. Fix if picked up later:
pass `Stage::effective_subagent_timeout_secs()` into `determine_activity_status` and the activity
renderer instead of the constant.
