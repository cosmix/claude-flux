# Entry Points

> Key files agents should read first to understand the codebase.
>
> **Related files:** [architecture.md](architecture.md) for system overview, [patterns.md](patterns.md) for design patterns.

## CLI Entry Point

`main.rs`/`lib.rs` entry, command dispatch, plan parsing/validation/graph, the
verification pipeline, and `.work/` config file paths.

→ [CLI, Commands and Plan Pipeline](entry-points/cli-and-plan-pipeline.md)

## Command Dispatch (cli/types.rs)

→ [CLI, Commands and Plan Pipeline](entry-points/cli-and-plan-pipeline.md)

## Orchestrator Core

Orchestrator main loop, stage/session data models, the daemon, monitor subsystem,
signal generation, merge/completion routing, terminal backends, recovery functions,
daemon credentials, dispute-criteria RPC, `TruthCheck`/before-after gates, the
subagent verification guard, and the status command.

→ [Orchestrator, Daemon and Sessions](entry-points/orchestrator-daemon-and-sessions.md)

## Data Models

→ [Orchestrator, Daemon and Sessions](entry-points/orchestrator-daemon-and-sessions.md)

## Plan Parsing Pipeline

→ [CLI, Commands and Plan Pipeline](entry-points/cli-and-plan-pipeline.md)

## Git Operations

Git worktree/merge/branch operations, `.work/` filesystem state, the handoff
system, sandbox config generation, the remote-control gate, WorkDir directory
helpers, ANTHROPIC_API_KEY env hygiene, the self-update HTTP client, and the
tiered knowledge-base module layout.

→ [Filesystem and Integration Modules](entry-points/filesystem-and-integration-modules.md)

## File System State

→ [Filesystem and Integration Modules](entry-points/filesystem-and-integration-modules.md)

## Daemon

→ [Orchestrator, Daemon and Sessions](entry-points/orchestrator-daemon-and-sessions.md)

## Monitor Subsystem

→ [Orchestrator, Daemon and Sessions](entry-points/orchestrator-daemon-and-sessions.md)

## Signal System

→ [Orchestrator, Daemon and Sessions](entry-points/orchestrator-daemon-and-sessions.md)

## Stage Completion (CLI)

→ [Orchestrator, Daemon and Sessions](entry-points/orchestrator-daemon-and-sessions.md)

## Terminal Backend

→ [Orchestrator, Daemon and Sessions](entry-points/orchestrator-daemon-and-sessions.md)

## Handoff System

→ [Filesystem and Integration Modules](entry-points/filesystem-and-integration-modules.md)

## Sandbox

→ [Filesystem and Integration Modules](entry-points/filesystem-and-integration-modules.md)

## Hooks

The full hook roster — every script in `hooks/`, the event it binds to, what it blocks — plus
`hooks/_common.sh`'s shared helpers and the registration sites a new hook must be added to.

→ [Hook Entry Points](entry-points/hooks.md)

## Schema-to-Runtime Conversion

→ [CLI, Commands and Plan Pipeline](entry-points/cli-and-plan-pipeline.md)

## CLI Subcommand Registration Pattern

→ [CLI, Commands and Plan Pipeline](entry-points/cli-and-plan-pipeline.md)

## Remote Control Module

→ [Filesystem and Integration Modules](entry-points/filesystem-and-integration-modules.md)

## Other Modules

→ [Filesystem and Integration Modules](entry-points/filesystem-and-integration-modules.md)

## Key Config Files

→ [CLI, Commands and Plan Pipeline](entry-points/cli-and-plan-pipeline.md)

## Verification System

→ [CLI, Commands and Plan Pipeline](entry-points/cli-and-plan-pipeline.md)

## Plan Validation Functions (plan/schema/validation.rs)

→ [CLI, Commands and Plan Pipeline](entry-points/cli-and-plan-pipeline.md)

## Plan Parser Module (plan/parser/mod.rs)

→ [CLI, Commands and Plan Pipeline](entry-points/cli-and-plan-pipeline.md)

## Execution Graph Build (plan/graph/mod.rs)

→ [CLI, Commands and Plan Pipeline](entry-points/cli-and-plan-pipeline.md)

## Status Command (commands/status/)

→ [Orchestrator, Daemon and Sessions](entry-points/orchestrator-daemon-and-sessions.md)

## Post-Tool Heartbeat

→ [Orchestrator, Daemon and Sessions](entry-points/orchestrator-daemon-and-sessions.md)

## Orchestrator Core Recovery Functions (Exact Locations)

→ [Orchestrator, Daemon and Sessions](entry-points/orchestrator-daemon-and-sessions.md)

## Plan Graph Loader — Stage File Preference (Critical)

→ [CLI, Commands and Plan Pipeline](entry-points/cli-and-plan-pipeline.md)

## Plan Schema — StageDefinition Amendable Fields

→ [CLI, Commands and Plan Pipeline](entry-points/cli-and-plan-pipeline.md)

## WorkDir Directory Helpers (Existing vs. Missing)

→ [Filesystem and Integration Modules](entry-points/filesystem-and-integration-modules.md)

## Sandbox Settings — ANTHROPIC_API_KEY

→ [Filesystem and Integration Modules](entry-points/filesystem-and-integration-modules.md)

## HTTP Client Pattern — self_update/client.rs

→ [Filesystem and Integration Modules](entry-points/filesystem-and-integration-modules.md)

## Daemon Credentials and Operator Proofs

→ [Orchestrator, Daemon and Sessions](entry-points/orchestrator-daemon-and-sessions.md)

## Dispute Criteria — Current Implementation

→ [Orchestrator, Daemon and Sessions](entry-points/orchestrator-daemon-and-sessions.md)

## Fix Attempts Counter — Current Usage

→ [Orchestrator, Daemon and Sessions](entry-points/orchestrator-daemon-and-sessions.md)

## Remote Control & Permission Mode Integration Points

Every file and call site involved in remote-control capability detection and permission-mode
resolution, with line references.

→ [Remote Control & Permission Mode](entry-points/remote-control.md)

## Signal Generation — Key Files and Line References

→ [Orchestrator, Daemon and Sessions](entry-points/orchestrator-daemon-and-sessions.md)

## TruthCheck / before_stage / after_stage / code_review

→ [Orchestrator, Daemon and Sessions](entry-points/orchestrator-daemon-and-sessions.md)

## `loom pressure` — Plan Pressure-Testing Files

→ [CLI, Commands and Plan Pipeline](entry-points/cli-and-plan-pipeline.md)

## Tiered Knowledge Base (2026-07-28)

→ [Filesystem and Integration Modules](entry-points/filesystem-and-integration-modules.md)

## Subagent Verification Guard (2026-07-28)

→ [Orchestrator, Daemon and Sessions](entry-points/orchestrator-daemon-and-sessions.md)

## Terminal Backends and `loom attach` (2026-08-08)

The dispatcher/lanes, `[terminal]` config, `--backend` flag, and `loom attach` entry
points all live in one topic file now.

Full detail: [terminal-backends.md](architecture/terminal-backends.md).

## Context Retrieval Subsystem (2026-08-17)

The context-retrieval pipeline (`context/mod.rs` is the one entry point) and the
source-graph retrieval channel: extraction, ranking, fusion, packing, delivery
dedupe, the overlay lifecycle, and `loom map`'s three read-only view flags.

→ [Context Retrieval and Source Graph](entry-points/context-and-source-graph.md)

## Execution Containment (2026-08-17)

| Path                                          | What it owns                                                                                                            |
| --------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `verify/criteria/confine.rs`                  | `spawn_confined` (the single leaf primitive for every plan-authored command), `resolve_confinement`, `plan_confinement` |
| `process/environment.rs`                      | `STAGE_HOST_ENV_ALLOWLIST`, `apply_stage_environment`                                                                   |
| `models/stage/types.rs:255`                   | `CommandConfinement`; `:340` `NetworkConfig`                                                                            |
| `orchestrator/terminal/native/wrapper.rs:181` | a SECOND, diverging copy of the env allowlist                                                                           |

Start from `architecture/execution-containment.md` — it states precisely what these do and
do not guarantee, which is narrower than the word "containment" suggests.

## New CLI Surface (2026-08-17)

→ [CLI, Commands and Plan Pipeline](entry-points/cli-and-plan-pipeline.md)

## Source Graph as a Retrieval Channel, and Its Lifecycle (2026-08-18)

→ [Context Retrieval and Source Graph](entry-points/context-and-source-graph.md)
