# E2 — Processing receipts, the pending queue, and a durable archive that survives cleanup

Plan: `doc/plans/PLAN-knowledge-source-graph-contracts.md` · Stage: `memory-events` · Wave 1 (parallel with E3; after E1).
Lane: codex `gpt-5.6-sol`, effort `xhigh`. Paths are repo-root relative; the crate is `loom/`.

## Why (F9, F10)

The knowledge-distill signal tells the curator to treat `loom memory show --all` as the source of truth and nothing records what became of each entry: no "promoted", "merged", "discarded", "deferred". A confidently wrong or skipped memory leaves no trace. Memories live only under `.loom/work/memory/`, which `loom clean` (`commands/clean/mod.rs::clean_state_directory`) and `loom init --clean` (`commands/init/cleanup.rs::cleanup_work_directory`) remove wholesale, and `.loom/work/` is git-ignored — nothing outside a run ever sees them. `loom review` keeps the last 10 notes per stage and drops the rest. The distill signal's claim that the ENTIRE `.loom/work/` directory is deleted the moment distillation finishes is false today (`fs/plan_lifecycle.rs::mark_plan_done_if_all_merged` only renames and commits), yet nothing archives either.

E1 (merged before you) gave every entry a stable `id`, a `Receipt` type, `MemoryEntryType::Receipt`, and `--json` listings. Read E1's report and `fs/memory/types.rs` directly — codex's `loom map` will not show them.

## Files you own (write)

- `loom/src/commands/memory/handlers/resolve.rs` — NEW, `handlers/pending.rs` — NEW, `handlers/mod.rs` (declarations), `commands/memory/mod.rs` (re-exports), `handlers/tests_resolve_pending.rs` — NEW (declare with `#[path]` like `handlers/tests.rs`)
- `loom/src/cli/types_memory.rs` (`MemoryCommands` gains `Resolve` and `Pending`), `loom/src/cli/dispatch.rs` (the two arms)
- `loom/src/fs/memory/archive.rs` — NEW; `fs/memory/mod.rs` (declare + re-export)
- `loom/src/fs/plan_lifecycle.rs` (`mark_plan_done_if_all_merged` only), `loom/src/commands/clean/mod.rs` (`clean_state_directory` only), `loom/src/commands/init/cleanup.rs` (`cleanup_work_directory` only)
- `.gitignore` (repo root): add `.loom/memory/`

Read-only: E1's files (`types.rs`, `parser.rs`, `storage.rs`, `record.rs`, `read.rs`); `orchestrator/signals/**`, `hooks/**`, `commands/distill.md` (E3 owns those).

## Contract

### `loom memory resolve`

```text
loom memory resolve <EVENT_ID> --outcome <promoted|merged|discarded|deferred> [--target <file-or-category/slug#heading>] [--reason <text>] [-S/--stage <id>]
```

- Validates that `EVENT_ID` names an existing Note/Decision/Question/Change entry in some journal of the plan (or in the current worktree's undrained spool — reuse `read_pending`); unknown id → error naming it (typo protection, like `--require-id`).
- `promoted`/`merged` REQUIRE `--target`; `discarded`/`deferred` REQUIRE `--reason`.
- Records a `MemoryEntry::receipt(Receipt { event_id, outcome, target }, reason_or_default)` through the SAME `record` path E1 owns (`handlers/record.rs::record` — call it; do not copy it) so the spool fallback, `LOOM_STAGE_ID` attribution, and forgery rejection all apply. Default reason text for promoted/merged: `promoted into <target>` / `merged into <target>`.
- Prints `✓ Resolved <event_id> (<outcome>) as receipt <receipt id>`.

### `loom memory pending`

```text
loom memory pending [-S/--stage <id>] [--json] [--strict]
```

- Pending = every `Note`, `Decision`, `Question` entry across the plan's journals (plus the current worktree's undrained spool) whose `id` is not the `event_id` of any `Receipt` entry anywhere. `Change` entries are exempt by design (cheap, regenerable implementation descriptions — the report's point that they compete with real lessons), and are listed under a separate `changes without receipts: N` count.
- Human output: one line per pending entry `<id>  <type>  <stage>  <first 80 chars>`, then `<N> pending across <M> journals`. `--json`: `{"pending": [entries], "changes_without_receipt": N, "receipts": R}`.
- `--strict`: exit 1 when `pending` is non-empty (after printing). Never opens the `ContextStore`; reads `.loom/work/memory/` only, so it is acceptance-safe in a worktree.
- Uses `readonly_work_dir()` (never creates anything); no workspace → prints `no memory journals` and exits 0 (or `{"pending": []}`).

### Archive (`fs/memory/archive.rs`)

```rust
/// Copy `<work_dir>/memory/` and `<work_dir>/telemetry/` (when present) to
/// `<main_root>/.loom/memory/archive/<plan-id-or-default>-<YYYYmmddTHHMMSSZ>/`.
/// Returns the archive path, `None` when there was nothing to archive. Never fails its caller.
pub fn archive_run_state(work_dir: &Path, main_root: &Path, plan_id: Option<&str>) -> Option<PathBuf>
```

Called, best effort with a one-line `eprintln!` on failure, from:

- `plan_lifecycle::mark_plan_done_if_all_merged` — before the rename;
- `clean::clean_state_directory` — before `remove_dir_all`;
- `init::cleanup::cleanup_work_directory` — before `remove_dir_all`.

`plan_id` comes from `.loom/work/config.toml` (`get_plan_id`/`plan_key_from` — grep how `delivery::plan_key_from` and `clean` read it; reuse the existing accessor). `main_root` is `WorkDir::main_project_root()` when resolvable, else the repo root the caller already holds.

## Steps

1. `resolve.rs`, `pending.rs`; wire the enum variants and dispatch arms.
2. `archive.rs` + the three call sites; `.gitignore`.
3. Tests (`tests_resolve_pending.rs`, `#[serial]` with the `init_git_repo` + `EnvGuard` fixtures from `handlers/tests.rs` — copy the two helpers rather than importing private items):
   - `resolve_records_a_receipt_referencing_the_event`
   - `resolve_refuses_an_unknown_event_id`
   - `promoted_requires_a_target_and_discarded_requires_a_reason`
   - `pending_lists_unreceipted_notes_and_omits_changes_and_receipted_entries`
   - `pending_strict_exits_non_zero_only_when_something_is_pending` (test the decision helper, not `process::exit`)
   - `pending_merges_the_current_worktrees_spool`
   - In `fs/memory/mod.rs` or a new `fs/memory/tests/archive.rs`: `archive_copies_journals_and_telemetry_and_survives_state_removal`; `archive_is_a_no_op_without_journals`.
   - `plan_lifecycle` / `clean` / `init::cleanup` tests: one each asserting the archive directory exists after the operation (reuse their existing fixtures; grep `mark_plan_done_if_all_merged` and `clean_state_directory` tests).

## Done means

`cargo build --manifest-path loom/Cargo.toml` warning-free; `cargo test --lib commands::memory::`, `--lib fs::memory::`, `--lib fs::plan_lifecycle`, `--lib commands::clean::`, `--lib commands::init::` green; `loom/target/debug/loom memory --help` lists `resolve` and `pending`.

## Proof (ONE narrow check, run once)

`cargo test --manifest-path loom/Cargo.toml --lib commands::memory::handlers::tests_resolve_pending` — skip if unsure.

## Constraints and traps

- A worktree stage cannot write `<main>/.loom/memory/` (sandbox denies writes escaping the worktree; `.loom/work` is a symlink but `.loom/` itself is not) — that is why receipts travel as ordinary journal entries through the existing spool, and why the archive is written only by host-side commands (`clean`, `init --clean`, plan completion in the daemon). Do not add a direct write to `<main>/.loom/memory/` from any path a stage session runs.
- `pending` must read the spool the way `read.rs::read_journal_with_pending` does (only the current worktree's stage), so one worktree's spool never leaks into another stage's listing.
- Keep `resolve`'s validation cheap: one pass over `list_journals`.
- No git commands (the `.gitignore` edit is a file edit).

## Report back

Files changed and created; the exact archive path format; anything unresolved.
