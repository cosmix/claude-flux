# Context Ceiling

> The absolute resident-token ceiling: resolution order, and the three independent thresholds (hook, daemon, native compaction) that enforce it.

## The Context Ceiling: Three Independent Thresholds, One Number

Loom stopped tracking context as a percentage of a fixed window and now tracks an
ABSOLUTE resident-token ceiling per stage/subagent. One resolved number —
`context_ceiling_tokens` — feeds three completely independent enforcement mechanisms,
each firing at a different multiple of it, from softest to hardest:

| Multiple | Mechanism                          | Who enforces it                                                                                             | What happens                                                                                                                          |
| -------- | ----------------------------------- | -------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| 1.0x     | `PostToolUse` hook instruction       | `hooks/post-tool-use.sh`, reading resident tokens off the transcript tail                                       | Blocks each tool call with exit 2 and a stderr instruction to finish the unit of work, run `loom handoff --trigger ceiling`, and stop |
| 1.25x    | Daemon backstop (`BudgetExceeded`)   | `orchestrator/monitor/detection.rs` (`DAEMON_CEILING_MULTIPLIER`, `models/constants.rs`)                        | Writes the outgoing handoff, discovers in-progress records even after daemon restart, KILLS the session, persists `ContextExhausted` only after confirmed death, then re-queues. Discovery/probe/persistence uncertainty leaves the stage in `NeedsHandoff`; stale events naming a predecessor are ignored. |
| 1.5x     | `CLAUDE_CODE_AUTO_COMPACT_WINDOW`    | The installed Claude Code binary itself, via an env var loom's native wrapper sets (`auto_compact_window_tokens`, `orchestrator/terminal/native/wrapper.rs:227-238`) | Claude Code's own native auto-compaction kicks in — "effectively unreachable in practice" per the source comment, because the two lower thresholds should already have ended the session by this point |

The daemon backstop is level-triggered while the exact over-budget assignment remains
`Executing` or `NeedsHandoff`, so a handler failure before the state transition cannot disarm it.
Each poll applies a fresh matching heartbeat to its session snapshot before making that decision:
resident context can fall after compaction, so a stale persisted high-water reading must not kill a
session whose current heartbeat is safely below the backstop. Persisted `Running` predecessors are
not judged once `stage.session` has moved on. Heartbeat change detection compares the complete
payload rather than its whole-second timestamp, and persistence is an exact-session locked
read-modify-write that cannot revive a terminal record or prefix-match another session.
Generated V2 handoffs persist their origin: Red-band and budget artifacts are independently
recoverable across daemon restarts. A cold-start Red observation reuses its durable snapshot, while
a real Green/Yellow-to-Red re-entry writes a fresh one; a cold start reuses Red only when the
resident-token snapshot is identical. Red-band observation and successful artifact readiness are
tracked separately, so a transient write failure retries during the same Red band. Budget lookup
scans all numbered handoffs
for the exact `(stage_id, session_id, budget_exceeded)` tuple and refreshes it when a newer artifact
exists, so continuation can select the newest valid handoff for the exact outgoing session rather
than blindly consuming the highest filename. Number allocation and crash-atomic write share one
directory lock, preventing concurrent daemon/CLI producers from overwriting the same artifact.

Read the resolved value in code with `fs::work_dir::resolve_context_ceiling_tokens(work_dir,
stage_ceiling)` — the one resolver. Resolution order: `stage.context_ceiling_tokens` ->
`.work/config.toml [context] ceiling_tokens` -> `DEFAULT_CONTEXT_CEILING_TOKENS` (150,000,
`models/constants.rs`); subagents default to `DEFAULT_SUBAGENT_CEILING_TOKENS` (120,000). The
1.5x auto-compact env var is separately clamped to `AUTO_COMPACT_WINDOW_MAX_TOKENS` (1,000,000)
before export, since the installed binary re-clamps to `[1, 1_000_000]` and then again to the
model's own context window.

The shell hook never parses TOML or stage YAML. Its internal
`loom hook context-ceilings` call loads both through Rust and prints one validated
`<main>:<subagent>` pair. `hooks/post-tool-use.sh` caches that pair at
`.work/heartbeat/<stage>.<session>.context-ceilings`, then selects the main or subagent half after
classifying the hook payload. The main value includes the stage override; the subagent value is
plan-wide and never consults stage frontmatter. Missing, failed, malformed, or out-of-range helper
output falls back to the two hand-kept shell defaults, so a broken helper cannot disable the
governor. A valid zero is different: Rust emits `0` for the main half when it cannot verify the
requested stage record, explicitly disabling main-stage enforcement rather than borrowing another
stage's ceiling; the independent subagent half remains usable. Keeping the fallback constants is
intentional availability defense, not a second config parser.

**The true last resort is `hooks/pre-compact.sh`'s block-then-allow pattern**, independent of
all three thresholds above (it fires whenever Claude Code's native compaction actually engages,
by whatever trigger): the FIRST `PreCompact` invocation in a session drops a
`.work/compaction-pending/<session-id>` flag file, writes a handoff, and BLOCKS (exit 2) with an
instruction to record a memory note of the working state before continuing; the SECOND
invocation (flag file present) removes the flag, writes an updated handoff, and ALLOWS (exit 0).
This guarantees at least one forced context-preserving checkpoint before Claude Code discards
anything, even if all three ceiling-based mechanisms above somehow never fired.

`orchestrator/monitor/context.rs::context_health(tokens, ceiling)` is a fourth, separate
consumer of the same ceiling — it bands the ratio for DISPLAY (`loom status`, the signal's
recitation-section context line) into Green `<60%`, Yellow `60-90%`, Red `>=90%`. It does not
itself trigger anything; the 1.0x/1.25x/1.5x mechanisms above are independent of these display
bands. See [architecture.md](../architecture.md) "Context Budget Enforcement" for the full
field/resolver contract.

## Handoff System (Full Chain)

Fully functional handoff chain, of which `hooks/pre-compact.sh` (above) is one link:

1. **`loom handoff create`** — CLI command accepting `--stage`, `--session`, `--trigger`, `--message` flags
2. **`pre-compact.sh`** — two-phase block-then-allow pattern (see above); no longer creates a recovery marker file
3. **`session-end.sh`** — uses glob `*-${LOOM_STAGE_ID}.md` for stage file lookup (handles depth prefixes)
4. **Signals** — `cache.rs`'s `append_common_footer()` adds compaction recovery instructions to ALL signal types
5. **`session-start.sh`** — on `SessionStart` with `.source == "compact"` or `"resume"`, emits `hookSpecificOutput` `additionalContext` re-anchor pointer so the agent finds its signal file after compaction
