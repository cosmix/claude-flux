<!-- generated automatically on knowledge writes — do not edit by hand -->

# Knowledge Index

> Your Knowledge Brief already quotes what retrieval judged relevant. Pull more with `loom knowledge context --query`. Open a file here only when a pull comes back empty; then read the tier-1 summary for the area, and only the tier-2 topics you touch.

## Tier 1 — Summaries

| File | Description | Lines |
| --- | --- | --- |
| [architecture.md](architecture.md) | High-level component relationships, data flow, module dependencies | 548 |
| [entry-points.md](entry-points.md) | Key files agents should read first | 572 |
| [patterns.md](patterns.md) | Architectural patterns discovered in the codebase | 799 |
| [conventions.md](conventions.md) | Coding conventions discovered in the codebase | 697 |
| [mistakes.md](mistakes.md) | Mistakes made and lessons learned - what to avoid | 1187 |
| [stack.md](stack.md) | Dependencies, frameworks, and tooling used in the project | 116 |
| [concerns.md](concerns.md) | Technical debt, warnings, and issues to address | 867 |

## Tier 2 — Topics

### architecture

| Topic | Title | Blurb | Lines |
| --- | --- | --- | --- |
| [architecture/codex-concurrency.md](architecture/codex-concurrency.md) | Codex Concurrency | Codex fan-out concurrency limits, what is measured, and what degrades under load (the shared state.json sidecar). | 102 |
| [architecture/codex-plugin.md](architecture/codex-plugin.md) | Codex Plugin | Codex plugin install and identity, the codex-rescue subagent, and the loom-codex-forwarder lane. | 393 |
| [architecture/context-ceiling.md](architecture/context-ceiling.md) | Context Ceiling | The absolute resident-token ceiling: resolution order, and the three independent thresholds (hook, daemon, native… | 71 |
| [architecture/context-retrieval.md](architecture/context-retrieval.md) | Context Retrieval | The retrieval subsystem: two graphs, two lanes, query-side gating, two-tier fusion, and the persistent BM25 index. | 525 |
| [architecture/core-abstractions.md](architecture/core-abstractions.md) | Core Abstractions | ExecutionGraph, Stage, Session, Orchestrator, TerminalBackend — plus data flow and .work/ file ownership. | 93 |
| [architecture/directory-structure.md](architecture/directory-structure.md) | Directory Structure | Full loom/src module tree, the .work/ state layout, and the repo-root asset directories. | 49 |
| [architecture/execution-containment.md](architecture/execution-containment.md) | Execution Containment | What sandboxed command containment means in loom, its two confinement levels, and what routes through spawn_confined. | 193 |
| [architecture/hook-system.md](architecture/hook-system.md) | Hook System | Hook embedding and install, the SessionStart hookSpecificOutput contract, and the two subagent enforcement hooks. | 156 |
| [architecture/knowledge-hierarchy.md](architecture/knowledge-hierarchy.md) | Knowledge Hierarchy | Tier-1/tier-2 knowledge mechanics: layout predicate, target parsing, INDEX.md generation, audit link rules, coverage… | 144 |
| [architecture/memory-spool.md](architecture/memory-spool.md) | Memory Spool and Drain | Topic notes for the architecture knowledge area. | 105 |
| [architecture/merge-flow.md](architecture/merge-flow.md) | Merge Flow | How a completed worktree stage reaches the target branch: the daemon writes Completed first, the sync loop runs the… | 62 |
| [architecture/quota-poller.md](architecture/quota-poller.md) | Quota Poller | How loom learns the operator's Claude and Codex subscription budget, where it caches it, and what the two dashboards do… | 29 |
| [architecture/remote-control.md](architecture/remote-control.md) | Remote Control | Capability detection, preflight, resolution, and per-kind session naming for driving external agent binaries. | 64 |
| [architecture/signal-generation.md](architecture/signal-generation.md) | Signal Generation | How a stage signal is assembled: stable-prefix cache, shared append_* helpers, per-stage-type prefixes, soft signals. | 175 |
| [architecture/skill-catalog.md](architecture/skill-catalog.md) | Skill Catalog | The two skill roots, why 53 skills live outside `~/.claude/skills`, and the install/hook-exemption hazards that came… | 103 |
| [architecture/source-graph.md](architecture/source-graph.md) | Source Graph | What the source graph is and is not, its honesty contract, extractor trait, node/edge and cache identity, and lifecycle. | 244 |
| [architecture/status-data-model.md](architecture/status-data-model.md) | Status Data Model | Where each field shown by `loom status` (static, compact, and `--live`) comes from, and what the live TUI does not yet… | 176 |
| [architecture/terminal-backends.md](architecture/terminal-backends.md) | Terminal Backends | The native and tmux session backends behind one dispatcher, lane resolution, and session-recorded dispatch. | 251 |
| [architecture/web-dashboard.md](architecture/web-dashboard.md) | Web Dashboard | `loom status --web [PORT]` — a read-only HTTP/WebSocket server (port 7373 default, `127.0.0.1` only) that serves an… | 31 |

### entry-points

| Topic | Title | Blurb | Lines |
| --- | --- | --- | --- |
| [entry-points/hooks.md](entry-points/hooks.md) | Hooks | Every hook script and the event it binds to, _common.sh's command-matching and subagent-detection helpers, and the… | 111 |
| [entry-points/remote-control.md](entry-points/remote-control.md) | Remote Control | Files and call sites for remote-control capability detection and permission-mode resolution. | 97 |

### patterns

| Topic | Title | Blurb | Lines |
| --- | --- | --- | --- |
| [patterns/doctrine-cross-surface.md](patterns/doctrine-cross-surface.md) | Doctrine Cross Surface | Pinning multi-surface guidance with equality tests, ambiguity-equals-fail-safe privilege lookups, and token-based shell… | 106 |
| [patterns/hook-content-stripping.md](patterns/hook-content-stripping.md) | Hook Command Matching | How a hook decides what a Bash command actually invokes: strip embedded content, tokenize into | 151 |
| [patterns/remote-control.md](patterns/remote-control.md) | Remote Control | The detect-capability, preflight, resolve-invocation shape for external agent binaries. | 50 |
| [patterns/stage-daemon-channels.md](patterns/stage-daemon-channels.md) | Stage-to-Daemon Channels | How a stage agent reaches the daemon to change its own stage's state, and why there are three | 81 |
| [patterns/subagent-hierarchy.md](patterns/subagent-hierarchy.md) | Subagent Hierarchy | Flat fan-out vs 2-level coordinator hierarchy vs agent teams: when to use each, model mix, file exclusivity. | 62 |

### mistakes

| Topic | Title | Blurb | Lines |
| --- | --- | --- | --- |
| [mistakes/adjudication-autonomy-deadlock.md](mistakes/adjudication-autonomy-deadlock.md) | Adjudication Autonomy Deadlock | An accepted verdict deadlocked the run: adoption by stage_id alone, requeue with unanswered disputes, a live disputing… | 153 |
| [mistakes/ambient-filesystem-trust.md](mistakes/ambient-filesystem-trust.md) | Ambient Filesystem Trust | Why an ancestor directory merely named .git is not evidence of a real repository, and the validation this requires. | 34 |
| [mistakes/codex-lane-rogue-wrapper.md](mistakes/codex-lane-rogue-wrapper.md) | Codex Lane Rogue Wrapper | A forwarding wrapper that did the task itself instead of forwarding, and why the codex sandbox state-dir escape hatch… | 117 |
| [mistakes/codex-navigation.md](mistakes/codex-navigation.md) | Codex Navigation | Forbidding reads instead of fixing a slow reader - a misdiagnosis and its correction. | 25 |
| [mistakes/completion-broker-credential.md](mistakes/completion-broker-credential.md) | Completion Broker Credential | The completion broker unreachable server-side fallback, duplicate file naming, and a sandboxed completion that exits 0… | 141 |
| [mistakes/computed-values-and-hidden-couplings.md](mistakes/computed-values-and-hidden-couplings.md) | Computed Values and Hidden Couplings | Topic notes for the mistakes knowledge area. Three lessons from the | 156 |
| [mistakes/detached-spawn-in-tests.md](mistakes/detached-spawn-in-tests.md) | Detached Spawn In Tests | Never spawn a process from a test that can outlive the test process. | 45 |
| [mistakes/doctrine-and-acceptance.md](mistakes/doctrine-and-acceptance.md) | Doctrine And Acceptance | Why a one-phrase grep proves presence but never agreement, and how doctrine drifts across surfaces unnoticed. | 110 |
| [mistakes/knowledge-base-drift.md](mistakes/knowledge-base-drift.md) | Knowledge Base Drift | How the knowledge base itself goes stale: plan-authoring notes frozen as architecture facts, `[UPDATED]` duplicates… | 103 |
| [mistakes/knowledge-cli-invariants.md](mistakes/knowledge-cli-invariants.md) | Knowledge Cli Invariants | Invariants belong in the fs constructor, not the CLI handler; lock ordering for sibling refreshes; update appends. | 76 |
| [mistakes/knowledge-write-channel.md](mistakes/knowledge-write-channel.md) | Knowledge Write Channel | Why a distillation stage cannot write knowledge directly, the append-only-is-not-enough gap, and how doctrine baked… | 100 |
| [mistakes/ledger-tui-rendering.md](mistakes/ledger-tui-rendering.md) | Ledger Tui Rendering | Topic notes for the mistakes knowledge area. | 38 |
| [mistakes/merge-cleanup-boundary.md](mistakes/merge-cleanup-boundary.md) | Merge Cleanup Boundary | A cleanup-boundary bug: what happened, why it survived undetected, and the fix shape worth reusing. | 170 |
| [mistakes/parallel-worktree-shared-state.md](mistakes/parallel-worktree-shared-state.md) | Parallel Worktree Shared State | Cross-worktree state races: the one diagnostic question, concrete cases, and a blind-review-subagent instance. | 124 |
| [mistakes/phantom-merges.md](mistakes/phantom-merges.md) | Phantom Merges | Eight lessons on loom's merge machinery — writing merged=true without verifying git ancestry (the costliest recurring… | 141 |
| [mistakes/pinned-literals-ledgers-and-wiring.md](mistakes/pinned-literals-ledgers-and-wiring.md) | Pinned Literals Ledgers And Wiring | The maintainability ledger exact-match trap and goal-backward wiring checks pinning a pattern to a path. | 136 |
| [mistakes/refactor-stragglers.md](mistakes/refactor-stragglers.md) | Refactor Stragglers | What a large removal or rename leaves behind: straggler initializers, stale comments, stale docs, duplicate modules. | 82 |
| [mistakes/sandbox-and-settings.md](mistakes/sandbox-and-settings.md) | Sandbox And Settings | Sandbox path rules, permission sync, excludedCommands matching, and settings env leaking between main repo and… | 516 |
| [mistakes/schema-reuse-and-silent-skips.md](mistakes/schema-reuse-and-silent-skips.md) | Schema Reuse And Silent Skips | deny_unknown_fields breaking a type with two deserialization sources, warn-and-continue masking total failure, and an… | 94 |
| [mistakes/session-identity-env.md](mistakes/session-identity-env.md) | Session Identity Env | The wrapper script's `LOOM_*` exports are a contract read by hooks, the CLI and the daemon. Two long-standing defects… | 83 |
| [mistakes/sessions-and-liveness.md](mistakes/sessions-and-liveness.md) | Sessions And Liveness | Session identity, liveness routing, spawn-site coverage, and the blast radius of adding a session field. | 305 |
| [mistakes/shell-command-matchers.md](mistakes/shell-command-matchers.md) | Shell Command Matchers | Separators that never become tokens, forgeable glob lookups, env leakage in hook tests, and three Bash traps. | 246 |
| [mistakes/status-broadcast-hardening.md](mistakes/status-broadcast-hardening.md) | Status Broadcast Hardening | Topic notes for the mistakes knowledge area. | 72 |
| [mistakes/store-without-consumer.md](mistakes/store-without-consumer.md) | Store Without Consumer | A store that was written but never read - what happened, why it stayed invisible, and the concrete trail. | 94 |
| [mistakes/subagent-orchestration.md](mistakes/subagent-orchestration.md) | Subagent Orchestration | Liveness signals for subagents, when a missing report is not a missing result, and the one-background-watch doctrine. | 284 |
| [mistakes/testing-and-lint.md](mistakes/testing-and-lint.md) | Testing And Lint | Lint and test discipline: --all-targets, --no-fail-fast, headless CI, ambient git config and inherited descriptors in… | 457 |
| [mistakes/tests-that-cannot-fail.md](mistakes/tests-that-cannot-fail.md) | Tests That Cannot Fail | Tests that pass regardless of whether the bug they exist to catch is present, and how to spot the shape. | 191 |
| [mistakes/tmux-backend.md](mistakes/tmux-backend.md) | Tmux Backend | tmux spawn-failure exit codes, cleanup-on-every-error-path discipline, and PID reuse across a retried session id. | 120 |
| [mistakes/untrusted-value-boundaries.md](mistakes/untrusted-value-boundaries.md) | Untrusted Value Boundaries | Enumerating every producer of a rendered field, not just the field, and why containment at one render site alone is not… | 141 |
| [mistakes/verification-harness.md](mistakes/verification-harness.md) | Verification Harness | When every check fails at once, suspect the harness; the PATH binary is not your build; silent subagents are failed… | 149 |
| [mistakes/visibility-and-reachability.md](mistakes/visibility-and-reachability.md) | Visibility And Reachability | pub(crate) is not nameable by itself - visibility is capped by path reachability - plus sibling traps around wrapper… | 98 |
| [mistakes/web-dashboard-server.md](mistakes/web-dashboard-server.md) | Web Dashboard Server (loom status --web) | Concurrency, security and testing lessons from building the hand-rolled HTTP/WebSocket | 162 |
| [mistakes/writer-reader-address.md](mistakes/writer-reader-address.md) | Writer/Reader Address | Topic notes for the mistakes knowledge area. | 71 |

### concerns

| Topic | Title | Blurb | Lines |
| --- | --- | --- | --- |
| [concerns/automatic-knowledge-source-graph-followups.md](concerns/automatic-knowledge-source-graph-followups.md) | Automatic Knowledge Source Graph Followups | Topic notes for the concerns knowledge area. | 47 |
| [concerns/codex-heartbeat-starvation.md](concerns/codex-heartbeat-starvation.md) | Codex Heartbeat Starvation | Topic notes for the concerns knowledge area. | 57 |
| [concerns/daemon-singleton.md](concerns/daemon-singleton.md) | Daemon Singleton (Resolved 2026-08-08) | Historical incident: two daemons once attached to the same `.work/`. Startup now holds an | 98 |
| [concerns/iterm2-window-teardown.md](concerns/iterm2-window-teardown.md) | Iterm2 Window Teardown | Topic notes for the concerns knowledge area. | 48 |
| [concerns/sandbox-protected-hooks-dir.md](concerns/sandbox-protected-hooks-dir.md) | Sandbox Protected hooks/ Directory | Claude Code's sandbox write-protects the project-root `hooks/` directory as part of its bare-git-repo rule; shell… | 43 |
| [concerns/sandbox-write-rules-inert.md](concerns/sandbox-write-rules-inert.md) | Sandbox Write Rules Inert | Sandbox Write() rules that are inert in loom's generated stage settings and in the | 62 |
| [concerns/web-dashboard-latent-issues.md](concerns/web-dashboard-latent-issues.md) | Web Dashboard Latent Issues | Issues reviewed in `loom/src/commands/status/web/` during integration-verify and | 28 |
