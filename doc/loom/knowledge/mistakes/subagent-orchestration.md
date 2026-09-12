# Subagent Orchestration

> Liveness signals for subagents, when a missing report is not a missing result, and the one-background-watch doctrine.

## A Missing Report Is Not a Missing Result

**The single most expensive orchestration belief in this repo.**

In one source-graph session, five worker subagents and two `loom-code-reviewer`
subagents all EDITED FILES CORRECTLY and **never delivered a final report** — no task
notification arrived for over 90 minutes for the first batch. One worker demonstrably
acted on a `SendMessage` (it fixed the exact function named, 9 seconds before the next
`stat`) yet still sent no reply. The knowledge-distillation stage of the same plan then
spawned six read-only gatherers and received zero reports from any of them.

**Prevention:** do not treat a missing report as a missing result. **Verify the WORK
directly** — run the gate, `stat` the files, read the diff — and absorb the outcome from
the tree. An orchestrator that waits on reports alone will hang forever here: budget one
real blocking wait, then verify and move on.

**Corollary for read-only agents.** A gatherer whose entire deliverable IS its report
has no work to verify, so this failure mode destroys its output entirely. When the task
is "read this and summarise", prefer doing it yourself, or have the agent write its
findings to a file inside the worktree that you can read regardless of whether it
reports.

## Liveness Is mtime, Never Size

File LINE COUNTS are a **false negative** for liveness: a worker rewriting a function in
place holds `wc -l` steady for many minutes while working hard, which reads exactly like
a dead agent. One session almost took over five live workers' files after roughly ten
consecutive stable-line-count checks; a `stat` showed the newest write had landed 9
seconds earlier.

**Rule: liveness = mtime moved, never size moved.** Use `stat -c %y <files>` or
`fd --changed-within 5min`. Restarting live work forfeits every token it has spent and
sets two agents writing the same files.

**Superseded as the PRIMARY signal (2026-08-27), not deleted.** `loom subagents
list`/`watch` read each subagent's own transcript and report `done`, `tool-wait`,
`generating`, or `unknown` — a distinction mtime cannot make, since a file sitting still
is consistent with both "finished" and "deep in one long tool call". Prefer `loom
subagents` first; mtime stays true as a fallback and as the reason NOT to trust file
size.

## File Exclusivity Is a Property of ALL Live Agents, Not of One Wave

A second subagent was spawned whose file set overlapped a still-running one: the
orchestrator asked a fixer for one extra test file, then — before it reported — spawned a
refactorer whose brief also covered `commands/knowledge/context.rs`. The fixer was wiring
a test module into that file while the refactorer extracted a helper out of it; whichever
wrote last would silently drop the other edit.

**The misleading signal:** the fixer LOOKED finished, because its seven earlier fixes had
been verified on disk and its earlier report had arrived — but it had just been handed
more work, so it was live again.

**Prevention:** before spawning any agent, list every agent that has not reported SINCE
ITS LATEST assignment and diff the file sets. **Asking a finished agent for one more thing
makes it live again and re-arms the conflict.** Detection used: grep the specific wiring
line the earlier agent added, both before AND after the later agent finishes.

## Never Hand Over a Proving Command You Have Not Run

`cargo test --lib a:: b:: c::` was written into a subagent brief. **Cargo accepts exactly
ONE testname filter** and rejects the extras with "unexpected argument" BEFORE compiling,
so zero tests ran. The error mentions the argument, not the arity, so it reads like a typo
in a test path. This bit both the orchestrator and a subagent in the same session.

**Rule:** any proving command handed to a subagent must be one you have actually run
yourself — a subagent that cannot run it reports "no tests ran" and you learn nothing
about its slice. Use ONE common prefix (`cargo test --lib context::`) or separate
invocations chained with `&&`.

Related: test module paths are part of the filter. The delivery unit tests live at
`context::tests::delivery`, so `cargo test --lib -- context::delivery` matches ZERO tests.

## "I Wrote My Own Because I Could Not Reach Yours" Is a Defect Report

When a worker says it reimplemented something rather than calling it, that is a
duplication defect to fix at integration — **never a resolved decision**. In one round it
produced a second copy of a security-critical untrusted-content fence renderer, which is
exactly the rule that must not drift between surfaces. See
`mistakes/visibility-and-reachability.md` for why the worker could not reach the original.

## Verify a Delegation Before Rejecting It

The installed `codex-forward.sh` did not emit the evidence trailer the codex doctrine says
to require, and treating that absence as proof the wrapper self-implemented would have been
wrong. Confirm a forward two other ways: the stdout carries the codex thread transcript
(`[codex] Starting Codex task thread.`, `[codex] Applying N file change(s).`), and a job
record exists under `~/.claude/plugins/data/codex-openai-codex/state/<worktree>-<hash>/jobs/*.json`
with a recent mtime. A forwarder that refuses to invent a trailer it never saw is behaving
correctly.

**And read what codex touched.** A codex forward that edits a small list-style file can
mangle it: `loom/tests/integration/mod.rs` came back with four lines spuriously indented
and the new entry out of alphabetical order. `cargo fmt` does NOT fix a mod list's
indentation or ordering, so it survives the format gate and only shows up in review.
"Check what it touched" means READ the diff of any small structural file (`mod.rs`,
`Cargo.toml`, a registration list) — whitespace damage passes every automated gate.

## Apply the Cheapest-Capable Rule to UNPLANNED Spawns Too

Mid-stage integration tasks (test-expectation fixes, a module split) were defaulted to
sonnet without deliberation on a stage that licensed codex, because the plan's EXECUTION
PLAN named codex for the PLANNED workers and lane choice felt already decided. Unplanned
work silently inherited the session default instead of getting its own lane decision. The
user called it out.

**Test to apply per spawn:** if you can name the exact files, the exact target shape, and
every signature that must survive, it is codex terra work. Route to sonnet when the work
needs open-ended judgment — not because codex cannot look things up.

**Superseded claim, corrected:** this section used to say "codex has no Read tool and
pages files through the shell in ~160-line chunks (measured 9m45s unscoped versus 54s
scoped)," and concluded that open-ended repo exploration must go to sonnet for that
reason. That measurement came from codex being told to read CLAUDE.md and sweep
`doc/loom/knowledge/` wholesale — not from exploration being inherently slow for codex.
Loom's forwarding wrapper now hands every codex prompt a source-graph navigation kit
(`loom map --find-all` / `--outline` / `--impact`, `loom knowledge context --query`), so
codex navigates by querying loom's index instead of paging files
(`architecture/codex-plugin.md#the-navigation-kit`). Codex is no longer excluded from
exploration on that basis; the cheapest-capable test above still governs the lane choice.

## Related

- `mistakes/verification-harness.md` — when every check fails at once, suspect the harness.
- `patterns/subagent-hierarchy.md` — choosing flat fan-out, a coordinator hierarchy, or a team.

## Before Its First Write, an Agent Had NO Liveness Signal At All (2026-08-17; CLOSED 2026-08-27)

The mtime rule above assumes the agent has already written something. Between spawn and
first write there used to be **no negative evidence available** — not an empty `git
status`, not an absent file, not `ListAgents`. An interactive session re-dispatched a
second agent onto a live one's file set on exactly that reasoning ("no changes on disk,
and `ListAgents` reports nothing reachable, so it must be dead"). It was
mid-investigation and had simply not typed yet. The two agents then wrote competing test
layouts, a third was sent in on a stale snapshot, and at the worst moment one was
deleting another's files while the suite sat at 5 red. Roughly 20 minutes and three
agents of work were burned; the production fixes had been correct the whole time.

**`ListAgents` returning "No reachable agents" is NOT evidence of death.** It went on
returning that while three spawned agents were actively editing files, and it said it
about agents that had already delivered final reports minutes earlier. Treat it as
"cannot tell", never as "gone".

**The blind spot is CLOSED: Claude Code writes every subagent its own JSONL transcript
from its FIRST TURN, before it writes any file.** It lives at
`~/.claude/projects/<project-slug>/<session-uuid>/subagents/agent-<agentId>.jsonl`
(`<project-slug>` is the absolute cwd with every `/` and `.` replaced by `-`), one JSON
entry per line carrying `agentId`, `timestamp`, `type` (`assistant`/`user`), and
`message.content`. `loom subagents list` reads this, so a spawned agent has a liveness
signal immediately — the incident above could not recur today.

**Prevention (revised 2026-08-30): spawn everyone, then run exactly ONE `loom subagents
watch --timeout <secs>` (3600 is normal) through the Bash tool's `run_in_background` —
never a hand-rolled poll and never a re-armed foreground watch.** The harness re-invokes
the session when the watch exits, and no request is made while it waits. Per-subagent
state is `done`, `tool-wait`, `generating`, or `unknown`: `done` but silent means harvest
from disk and proceed — a missing notification is not a missing result (see "A Missing
Report Is Not a Missing Result"); `tool-wait` / `generating` means genuinely alive, KEEP
WAITING on the background watch (do not re-arm it); only idle time past the budget with
NO transcript growth is positive evidence of death. Two measured numbers set that budget,
not estimates: a single tool call was clocked at 1,425s (23.8 minutes) in one real
transcript, which is why `tool-wait` must NEVER carry its own timeout no matter how long
it runs; and true intra-turn flush gaps (pauses between transcript writes within one
still-live turn) topped out at 137.7s across 8,808 sampled gaps, which is why a `done`
classification is debounced by 180s rather than trusted the instant output stops.
If a takeover ever does look necessary, `TaskStop` the original FIRST, confirm it
stopped, and only then dispatch a replacement — never leave two writers pointed at one
file set. Recovery from a collision is the same discipline: hard-stop every agent, take
one snapshot of the frozen tree, decide the target layout yourself, then send exactly one
agent to converge it.

**Corollary — a stale brief is worse than no brief.** Each replacement was briefed from a
snapshot that had already moved, so it was told to create files that already existed and
to fix problems already fixed. When dispatching into a tree others have touched, re-read
the state immediately before writing the brief, and tell the agent to STOP and report if
what it finds contradicts the brief. The one agent that did exactly that is the only one
that cost nothing.

## An Idle Notice Is Not a Report, and Absence of an Edit Is Not Refusal (2026-08-27)

**What happened:** in one interactive fan-out, two subagents were sent follow-up corrections via
their mailbox. Both emitted `idle_notification … "available"` shortly after. Reading the files
showed neither correction applied, so both were stopped and the remainders re-delegated to fresh
agents. The fresh agents then reported that the work was **already there** — the originals had
been mid-edit and finished in the window between the check and the stop. One re-delegation was
genuinely needed (that agent had written only the test, not the fix); the other was pure waste.

**Why:** `idle` means "not currently in a turn", not "processed your message". And a file read is
a point sample: an agent that has read the brief and is composing an edit looks byte-identical to
one that ignored it. Combining the two — idle notice plus no visible edit — feels like conclusive
evidence of non-compliance and is not.

**Prevention:**

- **Only a completion report closes a delegation.** Stop an agent on its own report, or on the
  liveness rules above (transcript not growing past a real budget) — never on "went idle and I
  don't see the change yet".
- **Verify against the tree, then wait, then verify again** before concluding an agent is not
  acting. The second sample is what distinguishes mid-edit from ignored.
- **What DID pay off: reading diffs instead of reports.** Two agents reported work as complete and
  clean that was neither — one had a path-traversal bypass in a security guard
  (`mistakes/untrusted-value-boundaries.md`), another left a status arm that rendered a stalled
  loop as healthy. Both reports were detailed and confident. Review the diff of anything
  load-bearing; a subagent's summary is a claim, not evidence.
- **A ledger or gate the agents cannot see will bite at commit time.** Five files broke the
  repo-wide line-count gate because every agent added regression tests and comments in good faith.
  Check the gate BEFORE fanning out, or expect a second round purely to satisfy it.

## A Connection-Error Notice Is Not Proof of Death (2026-08-29)

**What happened:** a subagent's idle notification arrived carrying
`failureReason: "API Error: Connection lost mid-response"`, with no result. It was read as death and
the whole assignment was respawned to a fresh agent. The original was alive and working. Both then
wrote the same four files for twenty minutes, each reporting the other's edits as a mysterious
concurrent writer that kept rewriting its test file and referencing methods it had never written.

**Why:** the notice describes a broken RESPONSE, not a stopped agent. The liveness rules above
already establish that elapsed time is not evidence of death; a transport error is not either. The
only real evidence is `ListAgents` showing the agent gone, or a transcript that stops growing past
a genuine budget.

**Prevention:** before respawning ANY assignment, call `ListAgents` and confirm the agent is
actually gone. If it is alive, message it instead of duplicating it. Once a duplicate exists, stand
one down explicitly rather than letting both finish, and inspect the surviving files for damage —
duplicate definitions, half-applied edits, a test file with one agent's imports and another's
tests — before trusting either report.

**What saved it, and would not always:** the module's API had been pinned in the brief, so both
agents built the same shape and the last writer's file was coherent. That was luck. A brief leaving
design latitude would have produced two incompatible halves of one module.

**Second occurrence of the gate lesson above, same session.** The line-count ledger bit again, for
exactly the reason already recorded: six agents each added tests and explanatory comments, pushing
three files and five functions over their limits, and clearing it cost a full extra refactor round
after the functional work was already green. The note was there and went unread. Read the gate
before fanning out, and if a file is already near its ceiling, say so in the brief.

## A Stop Condition Without Named Evidence Is an Unfalsifiable Exit (2026-08-31)

**What happened:** in one stage, five subagents in a row ended their turns reporting they had hit
a context ceiling, having written zero files between them. Their own transcripts were read
afterward: not one had ever received a ceiling message — no hook line, nothing from the lead.
Their real usage was 34,000 to 71,000 tokens against a 120,000 ceiling. The stage lost its entire
first wave of work and the lead spent its remaining context re-delegating.

**Why:** doctrine told a subagent to stop at the context ceiling without telling it what its
ceiling was or how it would learn it had reached one. That makes "I might be near the ceiling" an
exit available at any moment with nothing to falsify it, and a well-written report about stopping
reads like a result.

**Prevention:** a stop condition given to an agent must name the exact evidence that triggers it.
For the context ceiling that evidence is a hook line beginning `SUBAGENT CEILING REACHED:` in the
agent's own tool output, and nothing else counts. State the ceiling's value too, so "am I near it"
has an answer. Treat a turn that ends with zero files written, on a task that asked for files, as
a failed unit of work rather than a report.

**Fix:** BLOCK-D in `CLAUDE.md.template` and `orchestrator/signals/cache/blocks.rs` now carries
that rule, pinned byte-identical across surfaces by `tests_doctrine.rs`. The ceiling itself moved
from 150,000 to 800,000 for main agents and subagents alike, 0.80 of the 1M window both run.

## A Codex Forwarder Reports "done" While Its Codex Job Is Still Editing Files (2026-09-04)

**What happened:** a `loom-codex-forwarder` subagent's single Bash call exceeded the tool's
600000ms maximum, backgrounded under a Claude Code task id, and the forwarder's TURN ENDED —
so `loom subagents watch` printed "settled: every subagent is done" and `loom subagents list`
showed `state=done`, while `codex-companion.mjs status --all` showed the job still `running`,
phase `editing`, emitting "Applying N file change(s)".

**Why:** "done but silent -> harvest and proceed immediately" (the general rule for a
finished Claude subagent) is WRONG for the codex lane specifically: the forwarder's turn
ending only means the wrapper's Bash call returned control, not that codex finished writing.
Acting on it spawns the next wave into a tree codex is still mutating — the exact two-writers-
one-file hazard ownership tables exist to prevent.

**Prevention:** for any `loom-codex-forwarder` subagent, the authoritative liveness signal is
the companion job status, never `loom subagents`. Poll
`node <plugin>/scripts/codex-companion.mjs status --all` until the job leaves `running`
before spawning anything that touches its files. Companion at
`~/.claude/plugins/cache/openai-codex/codex/<ver>/scripts/codex-companion.mjs`; per-job log
at `~/.claude/plugins/data/codex-openai-codex/state/<worktree>-<hash>/jobs/<job>.log`.

## Subagent File Overlap Causes Lost Work

**Mistake:** Multiple subagents writing the same file leads to lost work (last writer wins).
**Fix:** Every subagent MUST have exclusive write access to its files. Use file ownership tables. If overlap is unavoidable, use one subagent or handle sequentially.

## Interactive Claude cannot be captured or made to auto-exit without risking API billing

**What happened:** A first cut of the `loom pressure` fixes assumed `claude -p` (or capturing Claude's stdout to a log) was the way to make Claude run one slash command and exit so the driver could proceed. Both are wrong for a subscription user.
**Why:** (1) `claude --help` states Claude runs in non-interactive mode "via -p, or when stdout is not a TTY, e.g. piped or redirected output" — so redirecting/capturing Claude's stdout flips it into the `-p` path, which can bill against pay-per-token API credits instead of the claude.ai subscription (known bug anthropics/claude-code#43333). (2) Feeding EOF on stdin (`< /dev/null`) does NOT let the task finish then exit — the REPL quits _before_ the agentic work completes (data loss), and an empirically-tested run also hit a workspace-trust dialog that `--permission-mode auto` did not skip. (3) There is no `--max-turns`/exit-when-done flag for interactive mode.
**Prevention:** For anything that must keep subscription billing, Claude's stdout MUST stay a real TTY (foreground, uncaptured). Do not reach for `-p` or output redirection to "automate" it. Only ONE process can own the foreground TTY, so anything running concurrently (e.g. Codex) must be backgrounded with captured output.
**Fix:** Mirror the loom daemon's own model — the daemon never relies on Claude self-exiting; it SIGTERMs the session (`event_handler.rs` → `NativeBackend::kill_session`) once the agent signals completion via `loom stage complete`. `loom pressure` does the analog: inject a "`touch <marker>` as your final action" instruction via `--append-system-prompt`, poll for the marker, then SIGTERM the idle foreground session (manual exit as fallback).

## Large-Scale Parallel Doc Editing: Whole-File Writes Fail, Self-Lint Reports Lie (2026-07-01)

**What happened:** During the 61-file skills/ overhaul (4 coordinators × ~6 workers), two recurring failures: (1) workers rewriting very large files (~2-3K lines, e.g. `skills/loom-react/SKILL.md`) with a single whole-file Write died repeatedly with "Connection closed mid-response" (0 tokens, files untouched); the same file succeeded when the worker was re-instructed to apply ~18 small targeted Edits instead. (2) Workers self-reported "markdownlint clean" but a single authoritative `markdownlint-cli2` pass at the gate found 35 residual errors across territories (MD032/MD056/MD038/MD034/MD028) — worker self-verification via ad-hoc greps does not implement markdownlint rules.
**Why:** A multi-thousand-line Write is one giant model response — long uninterrupted output maximizes exposure to connection drops, and a failure loses ALL of the work; incremental Edits checkpoint progress per tool call. Lint self-reports were grep-approximations, not the real linter (bunx was sandbox-blocked for workers: bun needs tempdir writes outside the sandbox allowlist).
**Prevention:** (1) When directing agents to rewrite files >~1000 lines, instruct them to transform via a sequence of targeted Edits, never one whole-file Write. (2) Never trust per-agent lint claims — run ONE authoritative `bunx markdownlint-cli2 "skills/**/SKILL.md"` at the merge/verify gate (no sandbox escape needed — point `TMPDIR` and `BUN_INSTALL_CACHE_DIR` at a writable scratch dir; recipe in `mistakes/testing-and-lint.md`); the repo `.markdownlint.json` is picked up from the root.
**Fix:** Re-spawned failed workers with the incremental-Edit instruction; ran the gate lint pass and fixed the 12 residual errors (main agent) + 23 (backend coordinator) directly.

## `loom pressure` codex "never starts" — it was invisible, not broken (2026-07-02)

**What happened:** The backgrounded Codex half of `loom pressure` was reported as "never starts (or starts and fails)". Investigation of the leftover logs (`/tmp/loom-pressure-codex-<pid>.log`) proved Codex ran fine in the recent runs: it triggered the `$pressure` skill, spent 170k–260k tokens, wrote its review next to the plan, and `/address` folded it in. Nothing was broken.
**Misleading signals:** (1) The driver printed NOTHING when codex spawned; the only codex UI was the wait-spinner, shown only when codex outlived the foreground Claude session — codex finishing first left zero terminal trace. (2) The codex report is deleted as final cleanup after all rounds, so no artifact survives a full run. (3) Every log contains a scary `ERROR rmcp::transport::worker … AuthorizationRequired` line even on successful runs — it is codex-side and non-fatal.
**Prevention:** Before diagnosing a `loom pressure` codex failure, read `/tmp/loom-pressure-codex-*.log` (one per driver invocation, overwritten per round) and check for `Wrote the pressure review to …` near the tail. Also note codex shares the driver's foreground process group: a Ctrl+C aimed at Claude SIGINTs codex too (`turn interrupted` in the log).
**Fix:** The driver now prints status lines — `→ codex review started in background (log: …)` at spawn, and after exit either `✓ codex review written → <report>` or a warning when codex exited cleanly without writing the report.

## A Fable Session Implemented the Fix It Had Just Diagnosed (2026-08-11)

**What happened:** a fable main agent investigated a bug, understood it, and then wrote the fix
itself instead of delegating — mainstream Rust edits at the most expensive tier available.

**Why:** the delegation rule was framed as "ORCHESTRATION IS ALWAYS OPUS / the orchestrator does
NOT implement." A fable session does not read itself as "the opus orchestrator," so the sentence
that should have bound it appeared to describe someone else. The fable _implementer_ tier also
listed "major bugs", and an agent that has just diagnosed a bug will classify it as major — the
exception swallowed the rule at exactly the moment the rule mattered.

**Prevention:** watch for the transition from "I now understand the bug" to the first Edit call.
That boundary is the delegation point, not a continuation of the investigation. Understanding the
fix is what makes a cheap subagent viable, so the cheaper the fix could now be, the stronger the
pull to type it yourself. Scope guidance to the SESSION's model, never to a model name assumed
from the role.

**Fix:** `CLAUDE.md.template` hard stop 6 (DELEGATION) plus Engineering Discipline E (Cheapest
capable agent) and a rewritten Rule 7 Model allocation: the rule is now stated model-independently
("the main agent never implements — whatever model it runs"), investigation is defined as ending
in a brief, the fable exception is narrowed from "major bugs" to "a bug that survived a delegated
fix attempt", and escalation requires evidence (a failed attempt), not a hunch.

**Follow-on (same day):** that template-only edit left every OTHER copy of the doctrine stale, and
the copies are not equivalent in how loudly they complain. `tests_doctrine.rs` pins BLOCK-A and
BLOCK-B byte-for-byte across `CLAUDE.md.template` and `skills/loom-plan-writer/SKILL.md`, so those
two failed the build immediately. The copies that say the same thing in DIFFERENT words are pinned
by nothing: the runtime signal prose in `orchestrator/signals/cache.rs` and
`signals/format/sections.rs`, the `Implementer::Claude` doc comment, and the knowledge summaries in
`patterns.md`. Those still told every spawning orchestrator "fable (major bugs, …)" — the exact
exception the change existed to close, on the surface an agent actually reads at run time. Editing a
doctrine block means `rg` for a distinctive phrase of the OLD wording across `loom/src`, `skills/`,
`agents/`, `hooks/` and `doc/loom/knowledge/` before committing; a green `tests_doctrine` proves the
two pinned surfaces agree, not that the doctrine is consistent.

## A Coordinator Wave for a Mid-Sized Feature Cost Forty Minutes (2026-09-12)

**What happened:** a configuration feature touching 35 files (fourteen config keys, two project sections, pressure flags, launch and dashboard wiring, docs) was run as one opus coordinator over five sonnet workers in two waves. Wall clock from spawn to a verified tree was about forty minutes; the user called that unacceptable for the size of the work. The coordinator itself spent turns briefing, the second wave could not start until the first returned, and the main agent still had to fix two seams the workers left (a key-level fallback that returned the built-in instead of the user tier, and an integration test that pinned the old dev-build notice).

**Why:** the two-level shape trades wall clock for context isolation. It pays off when the territory is too wide for one brief, not when the work is a handful of well-mapped files per lane. Here every lane was already mapped in the main agent's brief, so the coordinator added a hop without adding judgment.

**Prevention:** when the main agent has already mapped every file and signature, spawn the lanes directly as a flat fan-out and skip the coordinator, even if the user suggested one; say so and proceed. Sequence only what truly depends on a foundation, and give dependent workers the foundation's signatures up front so they start at once. Budget wall clock explicitly: a lane that needs more than fifteen minutes is a lane whose brief was too vague.

**Fix:** none in code. The feature shipped; the lesson is about shape.
