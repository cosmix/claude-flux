---
sources:
- loom/src/fs/memory/types.rs
- loom/src/fs/memory/spool.rs
verified: 054528e508d51ede343e254590cdb73ae00f7df6
---
# Memory Spool and Drain

> Read before touching loom memory: spool/drain, ids, receipts, pending, archive

## The Problem It Solves

`loom memory note` writes `.work/memory/<stage>.md` directly
(`commands/memory/handlers/record.rs`). Inside a worktree stage that write is
**impossible**, and the reasons compound:

- `.work` in a worktree is a **symlink** to the main repo's `.work`, so the
  target is outside the worktree's write boundary.
- The generated stage sandbox grants `Read(.work/memory/**)` but **no matching
  `Edit`** (`sandbox/settings.rs`). Only `.work/handoffs/**` gets a write grant
  — the "EROFS exemption".
- The loom binary is **not exempt from the sandbox**. `excluded_commands` is
  rejected outright by `sandbox/settings/policy.rs::validate_emittable`.

Result: `Read-only file system (os error 30)`. The kernel refuses; loom never
gets a say. See the mistakes entry for the full diagnosis.

## Why Not Just Widen the Allowlist

The sandbox filter is **path-based, not binary-based** — there is no knob that
says "let the loom executable write here". The two available levers both
overshoot:

- Adding `.work/memory` to `allowWrite` grants it to **every** process in the
  session, including the agent's own `Write`/`Edit` tools.
- `excludedCommands` does not grant a path; it runs the command **entirely
  outside** the sandbox. And `loom` is not a leaf command — `loom stage complete`
  executes the stage's acceptance criteria as shell (`verify/criteria/`), so
  exempting the binary exempts arbitrary shell reachable through it.

This matters because memory is not a private scratchpad: `orchestrator/signals/
generate.rs` reads `.work/memory/<dep-id>.md` for each dependency and embeds it
into **downstream stages' prompts**. A memory directory writable by any process
in a stage session is a prompt-injection channel between stages.

## The Design

```text
sandboxed agent      loom memory note "..."
                     ├─ try direct write to .work/memory/<stage>.md
                     └─ on PermissionDenied/EROFS only:
                        append to <worktree>/.loom/memory-spool.jsonl

daemon (outside the sandbox)
  every tick         drain every stage's spool -> .work/memory/<stage>.md
  at teardown        final drain in cleanup_after_merge, before removal
```

**Attribution is by filesystem location.** The spool payload carries **no stage
id** — the daemon attributes each entry to the stage that owns the worktree it
drained from. An agent cannot forge which worktree it is running in, but could
trivially forge a field.

Key modules: `fs/memory/spool.rs` (primitives plus the shared
`drain_into_journal`), `orchestrator/core/spool_drain.rs` (per-tick drain),
`git/cleanup/batch.rs` (teardown drain).

## Invariants That Are Easy To Break

**No status filter on the drain.** `loom stage complete` is a session's *last*
act, so entries recorded just before it are still pending when the stage leaves
`Executing`. Filtering by status strands exactly the most valuable notes — the
end-of-stage lessons. The trigger is spool-file existence; enumerating
`.work/stages/*.md` still validates that the id maps to a real stage.

**The drain returns `()`, not `Result`.** Every other step in the tick loop
propagates with `?`, and an `Err` out of the loop body exits `run()`, which in
the daemon sets the shutdown flag and **kills the daemon**
(`daemon/server/orchestrator.rs`). A spool problem must never do that.

**A validation failure must skip, not error.** `drain_spool` truncates only when
every sink call returns `Ok`. An entry that fails `validate_content` therefore
has to be *skipped* with `Ok(())`; returning `Err` would make one poison entry
redeliver forever and block every good entry behind it. Genuine I/O errors still
return `Err`, because retrying those next tick is correct.

**Teardown drains at `cleanup_after_merge`, not at its callers.** Five paths tear
down worktrees and three have no orchestrator in scope (`loom stage merge`,
`loom cleanup`, `loom stage retry`/`skip`). They all funnel through
`cleanup_after_merge`, so one call covers them. It is best-effort: a spool
failure must never wedge a merge.

**Locking is exclusive, not `O_APPEND`.** Content and context are each capped at
2000 chars, so one entry can exceed the platform's atomic-append window (~4096
bytes) and concurrent subagents would interleave bytes on a line.

**Reads merge the spool.** `loom memory list` is step one of post-compaction
recovery (CLAUDE.md Rule 3b), so `query`/`list`/`show` merge undrained entries,
and the aggregate `list`/`show` additionally surface a stage whose only entries
are still pending — both enumerate journal *files*, which such a stage does not
yet have. Daemon-side readers stay on the pure `read_journal`; they run after
the drain.

## Related

- Stage may only record to its own journal: `record()` rejects a `--stage` that
  disagrees with `LOOM_STAGE_ID` on **both** write paths. `validate_stage_id`
  blocks path separators but not a sibling stage's id.
- The completion broker solves the same "sandboxed agent needs privileged state
  written" problem a different way — hook outside the sandbox plus an
  authenticated RPC. See [mistakes/completion-broker-credential.md](../mistakes/completion-broker-credential.md).

## Entry Identity, Evidence and Receipts

Every `MemoryEntry` (`fs/memory/types.rs`) is given an `id` once, at capture —
`Uuid::new_v4().simple()`, 32 lowercase hex characters — alongside `timestamp`,
`entry_type`, `content`, an optional `context`, `session` (`LOOM_SESSION_ID` at
capture, when set), `evidence` and `receipt`. The spool line is the whole serialized
entry and the journal heading records the id (`format_entry` writes `id=<id>`), so
the id minted inside the sandbox is the id the journal keeps; the drain never
re-mints it.

- **Entry types** (`MemoryEntryType`): `Note`, `Decision`, `Question`, `Change`,
  `Receipt`. `loom memory note`, `decision`, `question` and `change` all accept
  evidence.
- **Evidence** is a list of paths, `path:line` spans or symbols the entry rests on.
  `validate_evidence` (`fs/memory/persistence.rs`) allows at most 16 references,
  each non-empty, at most 256 characters, with no backtick and no newline. `record`
  validates on the way in; `drain_into_journal` re-validates content, context and
  evidence on the way out, so a poison entry is skipped rather than written or
  redelivered.
- **Receipts** settle an earlier event. `loom memory resolve <event-id> --outcome
  <promoted|merged|discarded|deferred>` builds a `Receipt` entry and writes it
  through the same `record` path, so the stage-forgery check and the spool fallback
  apply to it too. `promoted` and `merged` require `--target` (the knowledge target
  the event went into); `discarded` and `deferred` require `--reason`. The id must
  name an existing non-receipt entry in some journal or in the current worktree's
  undrained spool, or the command fails with "Unknown memory event id". A journal
  entry is dropped on parse when it is typed `Receipt` without a receipt payload, or
  carries a payload without being a `Receipt` (`EntryBuilder::build`).

## Pending Events and the Run Archive

`loom memory pending [--stage <id>] [--json] [--strict]`
(`commands/memory/handlers/pending.rs`) lists every `Note`, `Decision` and
`Question` whose id no receipt settles, across every journal plus the current
worktree's undrained spool, deduplicated by id. It also counts `Change` entries
without a receipt and the receipts themselves. `--strict` exits 1 when anything is
pending.

The journals live in the state directory, which is removed when a plan finishes.
`archive_run_state` (`fs/memory/archive.rs`) first copies its `memory/` and
`telemetry/` subdirectories, whichever exist, to
`.loom/memory/archive/<plan-id-or-default>-<YYYYmmddTHHMMSSZ>/` under the main
repository root. It runs at plan completion
(`fs/plan_lifecycle.rs::archive_run_state_before_done`), in `loom clean`
(`clean_state_directory`), and in the init-time state cleanup
(`commands/init/cleanup.rs::cleanup_work_directory`). A failure prints a warning and
returns `None`, so cleanup and completion continue.
