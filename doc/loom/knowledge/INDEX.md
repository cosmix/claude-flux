<!-- generated automatically on knowledge writes — do not edit by hand -->

# Knowledge Index

> Read this index first, then only what it points to: the section for your area in a tier-1 summary (`rg -n '^## ' <file>` lists them) and the tier-2 topics your task touches. A specific question is cheaper to pull than to read — `loom knowledge context --query "..."` returns the matching sections quoted.

## Tier 1 — Summaries

| File | Description | Lines |
| --- | --- | --- |
| [architecture.md](architecture.md) | High-level component relationships, data flow, module dependencies | 548 |
| [entry-points.md](entry-points.md) | Key files agents should read first | 573 |
| [patterns.md](patterns.md) | Architectural patterns discovered in the codebase | 799 |
| [conventions.md](conventions.md) | Coding conventions discovered in the codebase | 697 |
| [mistakes.md](mistakes.md) | Mistakes made and lessons learned - what to avoid | 1194 |
| [stack.md](stack.md) | Dependencies, frameworks, and tooling used in the project | 116 |
| [concerns.md](concerns.md) | Technical debt, warnings, and issues to address | 875 |

## Tier 2 — Topics

### architecture

| Topic | Blurb | Lines |
| --- | --- | --- |
| [codex-concurrency](architecture/codex-concurrency.md) | Codex fan-out concurrency limits, what is measured, and what degrades under load (the shared… | 102 |
| [codex-plugin](architecture/codex-plugin.md) | Codex plugin install and identity, the codex-rescue subagent, and the loom-codex-forwarder lane. | 400 |
| [context-ceiling](architecture/context-ceiling.md) | The absolute resident-token ceiling: resolution order, and the three independent thresholds (hook… | 71 |
| [context-retrieval](architecture/context-retrieval.md) | The retrieval subsystem: two graphs, two lanes, query-side gating, two-tier fusion, and the… | 525 |
| [core-abstractions](architecture/core-abstractions.md) | ExecutionGraph, Stage, Session, Orchestrator, TerminalBackend — plus data flow and .work/ file… | 93 |
| [directory-structure](architecture/directory-structure.md) | Full loom/src module tree, the .work/ state layout, and the repo-root asset directories. | 49 |
| [execution-containment](architecture/execution-containment.md) | What sandboxed command containment means in loom, its two confinement levels, and what routes… | 193 |
| [hook-system](architecture/hook-system.md) | Hook embedding and install, the SessionStart hookSpecificOutput contract, and the two subagent… | 175 |
| [knowledge-hierarchy](architecture/knowledge-hierarchy.md) | Tier-1/tier-2 knowledge mechanics: layout predicate, target parsing, INDEX.md generation, audit… | 150 |
| [memory-spool](architecture/memory-spool.md) | Topic notes for the architecture knowledge area. | 105 |
| [merge-flow](architecture/merge-flow.md) | How a completed worktree stage reaches the target branch: the daemon writes Completed first, the… | 62 |
| [quota-poller](architecture/quota-poller.md) | How loom learns the operator's Claude and Codex subscription budget, where it caches it, and what… | 29 |
| [remote-control](architecture/remote-control.md) | Capability detection, preflight, resolution, and per-kind session naming for driving external agent… | 64 |
| [signal-generation](architecture/signal-generation.md) | How a stage signal is assembled: stable-prefix cache, shared append_* helpers, per-stage-type… | 175 |
| [skill-catalog](architecture/skill-catalog.md) | The two skill roots, why 53 skills live outside `~/.claude/skills`, and the install/hook-exemption… | 103 |
| [source-graph](architecture/source-graph.md) | What the source graph is and is not, its honesty contract, extractor trait, node/edge and cache… | 244 |
| [status-data-model](architecture/status-data-model.md) | Where each field shown by `loom status` (static, compact, and `--live`) comes from, and what the… | 176 |
| [terminal-backends](architecture/terminal-backends.md) | The native and tmux session backends behind one dispatcher, lane resolution, and session-recorded… | 251 |
| [web-dashboard](architecture/web-dashboard.md) | `loom status --web [PORT]` — a read-only HTTP/WebSocket server (port 7373 default, `127.0.0.1… | 33 |

### entry-points

| Topic | Blurb | Lines |
| --- | --- | --- |
| [hooks](entry-points/hooks.md) | Every hook script and the event it binds to, _common.sh's command-matching and subagent-detection… | 114 |
| [remote-control](entry-points/remote-control.md) | Files and call sites for remote-control capability detection and permission-mode resolution. | 97 |

### patterns

| Topic | Blurb | Lines |
| --- | --- | --- |
| [doctrine-cross-surface](patterns/doctrine-cross-surface.md) | Pinning multi-surface guidance with equality tests, ambiguity-equals-fail-safe privilege lookups… | 106 |
| [hook-content-stripping](patterns/hook-content-stripping.md) | How a hook decides what a Bash command actually invokes: strip embedded content, tokenize into | 151 |
| [remote-control](patterns/remote-control.md) | The detect-capability, preflight, resolve-invocation shape for external agent binaries. | 50 |
| [stage-daemon-channels](patterns/stage-daemon-channels.md) | How a stage agent reaches the daemon to change its own stage's state, and why there are three | 81 |
| [subagent-hierarchy](patterns/subagent-hierarchy.md) | Flat fan-out vs 2-level coordinator hierarchy vs agent teams: when to use each, model mix, file… | 62 |

### mistakes

| Topic | Blurb | Lines |
| --- | --- | --- |
| [adjudication-autonomy-deadlock](mistakes/adjudication-autonomy-deadlock.md) | An accepted verdict deadlocked the run: adoption by stage_id alone, requeue with unanswered… | 188 |
| [ambient-filesystem-trust](mistakes/ambient-filesystem-trust.md) | Why an ancestor directory merely named .git is not evidence of a real repository, and the… | 34 |
| [codex-lane-rogue-wrapper](mistakes/codex-lane-rogue-wrapper.md) | A forwarding wrapper that did the task itself instead of forwarding, and why the codex sandbox… | 117 |
| [codex-navigation](mistakes/codex-navigation.md) | Forbidding reads instead of fixing a slow reader - a misdiagnosis and its correction. | 25 |
| [completion-broker-credential](mistakes/completion-broker-credential.md) | The completion broker unreachable server-side fallback, duplicate file naming, and a sandboxed… | 141 |
| [computed-values-and-hidden-couplings](mistakes/computed-values-and-hidden-couplings.md) | Topic notes for the mistakes knowledge area. Three lessons from the | 156 |
| [detached-spawn-in-tests](mistakes/detached-spawn-in-tests.md) | Never spawn a process from a test that can outlive the test process. | 45 |
| [doctrine-and-acceptance](mistakes/doctrine-and-acceptance.md) | Why a one-phrase grep proves presence but never agreement, and how doctrine drifts across surfaces… | 110 |
| [knowledge-base-drift](mistakes/knowledge-base-drift.md) | How the knowledge base itself goes stale: plan-authoring notes frozen as architecture facts… | 103 |
| [knowledge-cli-invariants](mistakes/knowledge-cli-invariants.md) | Invariants belong in the fs constructor, not the CLI handler; lock ordering for sibling refreshes… | 86 |
| [knowledge-write-channel](mistakes/knowledge-write-channel.md) | Why a distillation stage cannot write knowledge directly, the append-only-is-not-enough gap, and… | 100 |
| [ledger-tui-rendering](mistakes/ledger-tui-rendering.md) | Topic notes for the mistakes knowledge area. | 38 |
| [merge-cleanup-boundary](mistakes/merge-cleanup-boundary.md) | A cleanup-boundary bug: what happened, why it survived undetected, and the fix shape worth reusing. | 170 |
| [parallel-worktree-shared-state](mistakes/parallel-worktree-shared-state.md) | Cross-worktree state races: the one diagnostic question, concrete cases, and a… | 124 |
| [phantom-merges](mistakes/phantom-merges.md) | Eight lessons on loom's merge machinery — writing merged=true without verifying git ancestry (the… | 141 |
| [pinned-literals-ledgers-and-wiring](mistakes/pinned-literals-ledgers-and-wiring.md) | The maintainability ledger exact-match trap and goal-backward wiring checks pinning a pattern to a… | 136 |
| [refactor-stragglers](mistakes/refactor-stragglers.md) | What a large removal or rename leaves behind: straggler initializers, stale comments, stale docs… | 82 |
| [sandbox-and-settings](mistakes/sandbox-and-settings.md) | Sandbox path rules, permission sync, excludedCommands matching, and settings env leaking between… | 516 |
| [schema-reuse-and-silent-skips](mistakes/schema-reuse-and-silent-skips.md) | deny_unknown_fields breaking a type with two deserialization sources, warn-and-continue masking… | 94 |
| [session-identity-env](mistakes/session-identity-env.md) | The wrapper script's `LOOM_*` exports are a contract read by hooks, the CLI and the daemon. Two… | 83 |
| [sessions-and-liveness](mistakes/sessions-and-liveness.md) | Session identity, liveness routing, spawn-site coverage, and the blast radius of adding a session… | 305 |
| [shell-command-matchers](mistakes/shell-command-matchers.md) | Separators that never become tokens, forgeable glob lookups, env leakage in hook tests, and three… | 246 |
| [status-broadcast-hardening](mistakes/status-broadcast-hardening.md) | Topic notes for the mistakes knowledge area. | 72 |
| [store-without-consumer](mistakes/store-without-consumer.md) | A store that was written but never read - what happened, why it stayed invisible, and the concrete… | 94 |
| [subagent-orchestration](mistakes/subagent-orchestration.md) | Liveness signals for subagents, when a missing report is not a missing result, and the… | 284 |
| [testing-and-lint](mistakes/testing-and-lint.md) | Lint and test discipline: --all-targets, --no-fail-fast, headless CI, ambient git config and… | 467 |
| [tests-that-cannot-fail](mistakes/tests-that-cannot-fail.md) | Tests that pass regardless of whether the bug they exist to catch is present, and how to spot the… | 191 |
| [tmux-backend](mistakes/tmux-backend.md) | tmux spawn-failure exit codes, cleanup-on-every-error-path discipline, and PID reuse across a… | 120 |
| [untrusted-value-boundaries](mistakes/untrusted-value-boundaries.md) | Enumerating every producer of a rendered field, not just the field, and why containment at one… | 141 |
| [verification-harness](mistakes/verification-harness.md) | When every check fails at once, suspect the harness; the PATH binary is not your build; silent… | 149 |
| [visibility-and-reachability](mistakes/visibility-and-reachability.md) | pub(crate) is not nameable by itself - visibility is capped by path reachability - plus sibling… | 98 |
| [web-dashboard-server](mistakes/web-dashboard-server.md) | Concurrency, security and testing lessons from building the hand-rolled HTTP/WebSocket | 162 |
| [writer-reader-address](mistakes/writer-reader-address.md) | Topic notes for the mistakes knowledge area. | 71 |

### concerns

| Topic | Blurb | Lines |
| --- | --- | --- |
| [automatic-knowledge-source-graph-followups](concerns/automatic-knowledge-source-graph-followups.md) | Topic notes for the concerns knowledge area. | 47 |
| [codex-heartbeat-starvation](concerns/codex-heartbeat-starvation.md) | Topic notes for the concerns knowledge area. | 57 |
| [daemon-singleton](concerns/daemon-singleton.md) | Historical incident: two daemons once attached to the same `.work/`. Startup now holds an | 98 |
| [iterm2-window-teardown](concerns/iterm2-window-teardown.md) | Topic notes for the concerns knowledge area. | 48 |
| [sandbox-protected-hooks-dir](concerns/sandbox-protected-hooks-dir.md) | Claude Code's sandbox write-protects the project-root `hooks/` directory as part of its… | 43 |
| [sandbox-write-rules-inert](concerns/sandbox-write-rules-inert.md) | Sandbox Write() rules that are inert in loom's generated stage settings and in the | 62 |
| [web-dashboard-latent-issues](concerns/web-dashboard-latent-issues.md) | Issues reviewed in `loom/src/commands/status/web/` during integration-verify and | 28 |
