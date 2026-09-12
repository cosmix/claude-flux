# Architectural Patterns

> Discovered patterns in the codebase that help agents understand how things work.
>
> **Related files:** [architecture.md](architecture.md) for system overview, [conventions.md](conventions.md) for coding standards.

## Table of Contents

Superseded by the generated index. Read the files directly, or run
`loom knowledge context --query "..."` for a targeted pull; open
[INDEX.md](INDEX.md) for the current tier-1 / tier-2 map — a hand-maintained
table of contents goes stale the moment a topic is added.

## State Machine Pattern

Stage has 13 states (WaitingForDeps → ... → Completed, terminal); dependents become Queued only once deps are `status == Completed AND merged == true`. All state persists to `.work/` as markdown+YAML, and concurrent writers (orchestrator loop, daemon IPC, CLI) must use the locked `update_stage` transaction rather than raw save. Stage completion, field propagation, and the four-layer goal-backward verification (acceptance criteria, artifacts, wiring, dead-code, plus testing conventions like matched positive/negative controls) live in the same topic.

→ [Stage Lifecycle & Verification](patterns/stage-lifecycle-and-verification.md)

## Progressive Merge Pattern

Dependencies merge to main before dependents start; `MergeLock` serializes concurrent merges. Covers the merge-conflict recovery flow (anti-respawn signal, attribution-aware recovery via `attribute_main_repo_merge`, the pure `route_complete_for_conflicts` routing helper), the three-file dispute authority split, and the plan-amendment atomic write.

→ [Merge & Recovery](patterns/merge-and-recovery.md)

## Daemon IPC Pattern

Unix socket IPC (`.work/orchestrator.sock`, mode 0600) admits bounded requests under a stable-file `flock`; the 5-second polling loop syncs the stage graph, spawns ready stages, and drains monitor events. Signal generation (Manus KV-cache prefix layout), heartbeat/context-health monitoring, session backend dispatch (native/tmux), and the memory-spool drain for sandboxed writes are all part of the same daemon loop.

→ [Orchestrator Daemon Loop](patterns/orchestrator-daemon-loop.md)

## Hook Patterns

Hooks read stdin JSON and gate via exit code / `permissionDecision`. Covers input validation (`validate_id`, `safe_filename`), shell/AppleScript escaping, self-update signature verification, sandbox config merging and `permission_mode` resolution, permission-file sync, and the untrusted-value flattening (`inline_safe`) shared across every agent- and operator-facing renderer.

→ [Security, Sandbox & Hooks](patterns/security-sandbox-and-hooks.md)

## Hook Command Matching

Hooks decide what a Bash command INVOKES by scanning argv tokens, not by regexing the command
string — a regex cannot tell an argument's _value_ from its _mention_, so quoted prose was read as
shell. Stripping heredoc and `-m` bodies is now the pre-step; the old regexes survive only as the
unterminated-quote fallback. Path checks key on whitespace, since a real path argument is a
whitespace-free word and a prose payload is not.

→ [Hook Content-Stripping](patterns/hook-content-stripping.md)

## CLI Subcommand Registration Pattern

Registering a CLI command needs Clap (3 files: `cli/types.rs`, `cli/dispatch.rs`, `commands/<newcmd>.rs`) AND the separate hand-maintained completion tables under `completions/dynamic/` — Clap alone leaves a command invisible to shell tab-completion. Also covers TUI rendering modes, the three-tier error-handling convention, the process wrapper/liveness pattern, `toml_edit`-only config writes, the two-tier `loom init`/`plan verify` validation split, the `loom pressure` foreground agent driver, and the confidence-ceiling / best-effort-contract engineering conventions.

→ [CLI, Process & Conventions](patterns/cli-process-and-conventions.md)

## Remote Control Capability/Preflight/Resolve Pattern (2026-05-14)

The three-phase shape for driving an external agent binary: detect capability, preflight the
environment, then resolve the concrete invocation. Keeps unsupported combinations failing
early with an actionable message instead of mid-run.

→ [Remote Control Pattern](patterns/remote-control.md)

## Subagent Hierarchy + Ultracode Guidance (2026-06-12)

When to fan out flat, when to use a 2-level coordinator→worker hierarchy, and when to reach
for agent teams; the model mix for each; and the file-exclusivity rule that makes parallel
subagents safe. Also carries the no-verify doctrine subagents inherit. Ultracode Workflow
fan-out is Claude-only — the codex lane runs outside it via normal `loom-codex-forwarder`
spawns — and a plan should prefer one ultracode stage over several parallel sibling stages
doing the same operation on disjoint file sets.

→ [Subagent Hierarchy](patterns/subagent-hierarchy.md)

## Doctrine Blocks, Fail-Safe Gates, and Shell Matchers (2026-07-28)

Guidance that must appear byte-identically on several surfaces needs an equality test, not N
greps; privilege lookups from state files must treat ambiguity as refusal; and shell-command
classification in a hook has a specific normalise-then-tokenise shape.

→ [Doctrine & Fail-Safe Patterns](patterns/doctrine-cross-surface.md)

## Tiered Knowledge Hierarchy (2026-07-28)

The knowledge base is **two-tier**. Tier-1 files (`architecture.md`, `patterns.md`, …) hold
short summaries that link out; tier-2 topics live at `<category>/<slug>.md` and hold the detail.
`INDEX.md` is generated and is the single layout predicate — a directory is hierarchical **iff**
`INDEX.md` exists.

Reading protocol: index first, then the tier-1 summary for your area, then only the tier-2
topics you actually touch. Writing protocol: a tier-1 section that grows past ~40 lines is spilled
into a topic and replaced by a 2-4 line summary plus a relative link.

A human-readable title pointing to `category/slug.md` in a **tier-1** file is the house convention:
relative, `.md` extension, no `./`, no anchor. No audit enforces it today; an earlier version of
this doc claimed two link-form checks required exactly this form, but no such checks exist in the
tree. See [Knowledge Hierarchy](architecture/knowledge-hierarchy.md) for the mechanics and the
`catalog::build` diagnostics that actually run.

## Base Layer Plus Per-Stage Overlay, Shadowing Wholesale

The pattern that lets parallel worktrees share derived state safely: an immutable base keyed
by the revision it was built from, plus a per-stage overlay holding only what that stage
changed, read as `overlay ∪ (base − overlay's files)`. An overlay entry shadows its base
counterpart **wholesale, never merges with it** — a partial merge produces a view describing
no revision that ever existed.

Reusable rules that come with it: the layout module owns layering and serialization only,
never building or write-timing; and the layering cannot express a DELETION without a
tombstone concept, so plan for one if deletions matter.
→ [architecture/context-retrieval.md](architecture/context-retrieval.md)

## One Door for an Irreversible Operation

`MergeLifecycle::cleanup` is the only path by which any caller may reach
`cleanup_after_merge`, and it refuses unless the stage branch is provably contained in the
target. Removing a side effect from a function is only durable if that side effect gains a
single owner — otherwise the next caller re-adds it locally. Pair it with the rule that a
destructive step which cannot verify its own precondition must decline rather than proceed.
→ [mistakes/merge-cleanup-boundary.md](mistakes/merge-cleanup-boundary.md)

## Stage-to-Daemon Channels

Three routes a stage agent uses to change its own stage state, picked by `DaemonReach`: a live
daemon's answer (refusals included, never routed around), a local fallback when nothing is
listening, and a worktree spool when the sandbox denies AF_UNIX outright. Covers why the socket
alone cannot reach a sandboxed stage, and why a spooled request carries no stage id.

→ [Stage-to-Daemon Channels](patterns/stage-daemon-channels.md)

## Fanning a UI Out by File: Give the Foundation Module the Shared Helpers First

When a plan splits UI work across parallel workers by disjoint file ownership, a width/truncation/
padding helper written by one worker is invisible to a sibling writing a different file at the same
time — the same defect class gets reimplemented independently in each file that needs it. Have the
worker who runs first and alone (the "W0" foundation module in a staged fan-out) own those shared
helpers explicitly, and name them by path in every later worker's brief. Absent that, budget an
orchestrator convergence pass after the fan-out returns rather than assuming disjoint files stayed
consistent. See [Ledger TUI Rendering](mistakes/ledger-tui-rendering.md) for three independent
instances of this in one stage.

## Offline File Harness for a Visual Review Under a No-Network Sandbox (2026-09-12)

When a stage sandbox denies loopback TCP (see
[mistakes/sandbox-and-settings.md](mistakes/sandbox-and-settings.md#a-stage-sandbox-can-deny-loopback-tcp-even-while-the-server-reports-listening-but-not-always-2026-09-12))
but a plan step needs a real browser render for review, build the harness as static
files instead of a server: `vite build --base ./` produces a bundle loadable via
`file://`; wire real routes with `createMemoryRouter` at the URL under review (e.g.
`/?settings=1`); stub `fetch` with the plan's own fixture JSON and stub `WebSocket` so
the app never tries the network; screenshot with Playwright's `chrome-headless-shell`
launched with `--allow-file-access-from-files` (required for `file://` fetches of
sibling assets). Keep the harness itself out of the reviewed bundle (e.g.
`web/node_modules/.harness/`) and write any screenshots inside the worktree proper, not
under `node_modules/` — the Read tool's worktree guard opens images by path, and a
`node_modules`-nested directory is easy to exclude by accident from later tooling.
