# E1 — Memory entries become events: stable id, full timestamp, session, evidence, receipts type, locked append

Plan: `doc/plans/PLAN-knowledge-source-graph-contracts.md` · Stage: `memory-events` · Wave 0 (runs ALONE; E2 and E3 follow).
Lane: codex `gpt-5.6-sol`, effort `xhigh`. Paths are repo-root relative; the crate is `loom/`.

## Why (F9)

`MemoryEntry` (`loom/src/fs/memory/types.rs`) is `{ timestamp, entry_type, content, context }`: no id, no session, no evidence, no processing state. The journal (`storage.rs`, `parser.rs`) writes the time as `%H:%M:%S` only and re-attaches TODAY's date on read, so a multi-day plan's entries all read back as today and sort wrongly. `append_entry` is an unlocked `OpenOptions::append` while the spool takes `lock_exclusive` for exactly the reason a 2000+2000-char entry can interleave. Nothing can reference an entry, so nothing can record what happened to it. E2 builds receipts and a pending queue on the identity you add here.

## Files you own (write)

- `loom/src/fs/memory/types.rs`, `parser.rs`, `storage.rs`, `spool.rs`, `export.rs`, `query.rs`, `persistence.rs`, `mod.rs` (re-exports + inline tests), `tests/spool.rs`
- `loom/src/commands/memory/handlers/record.rs`, `handlers/read.rs`, `handlers/tests.rs`, `formatters.rs`, `mod.rs`
- `loom/src/cli/types_memory.rs` — the `MemoryCommands` enum only; `loom/src/cli/dispatch.rs` — the `dispatch_memory` arms only
- Any other file that builds a `MemoryEntry` literal or matches `MemoryEntryType` exhaustively (grep first: `rg -n "MemoryEntry \{|MemoryEntryType::" loom/src` — expected: `commands/review/generate.rs`, `orchestrator/monitor/handlers.rs`, `orchestrator/core/spool_drain_tests.rs`, `fs/memory/export.rs`)

Read-only: `commands/handoff/**`, `orchestrator/signals/**`, `hooks/**` (E3), `fs/plan_lifecycle.rs`, `commands/clean/**`, `commands/init/**` (E2).

## Contract

```rust
// types.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryEntryType { Note, Decision, Question, Change, Receipt }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReceiptOutcome { Promoted, Merged, Discarded, Deferred }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    /// The `MemoryEntry::id` this receipt settles.
    pub event_id: String,
    pub outcome: ReceiptOutcome,
    /// Knowledge target the event was promoted or merged into (`architecture/context-retrieval.md#required-items`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// Assigned once at capture: `uuid::Uuid::new_v4().simple()` (32 lowercase hex chars).
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub entry_type: MemoryEntryType,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// `LOOM_SESSION_ID` at capture, when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Paths, `path:line` spans, or symbols the entry rests on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    /// Present exactly when `entry_type == Receipt`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Receipt>,
}
```

Constructors: `MemoryEntry::new(entry_type, content)` and `with_context(...)` keep their signatures and now assign `id`, `timestamp: Utc::now()`, `session` from the environment; add `MemoryEntry::receipt(receipt: Receipt, reason: String) -> Self` (type `Receipt`, `content = reason`), and a builder-style `with_evidence(self, evidence: Vec<String>) -> Self`. `MemoryEntryType::from_str` accepts `receipt`/`receipts`; `display_name` and the emoji table gain the variant (`\u{1F9FE}` receipt).

Journal format (`parser.rs::format_entry` / `parse_journal`), one entry:

```text
### <emoji> <Type> [2026-09-10 14:03:22] id=<32 hex>

<content>

*Context:* <context>            (when present)
*Evidence:* `a/b.rs:12`, `symbol`  (when non-empty; backtick-quoted, comma-separated)
*Session:* <session id>         (when present)
*Receipt:* event=<id> outcome=<promoted|merged|discarded|deferred> target=<target|->   (Receipt entries only)

---
```

`parse_journal` reads exactly this shape back (full timestamp with `%Y-%m-%d %H:%M:%S`; a heading without `id=` is a parse failure of that entry — skipped with a `tracing::debug!`, never a synthetic id). No backward compatibility with the old time-only heading is required (CLAUDE.md: unreleased project).

Locking: `storage.rs::append_entry` takes `crate::fs::locking`'s exclusive file lock on the journal for the duration of the write (mirror `spool.rs::append_to_spool`'s `lock_exclusive` use).

CLI (`types_memory.rs`): `note`, `decision`, `question`, `change` gain `--evidence <REF>` (repeatable, `-e`); `list` and `show` gain `--json` (print the entries as a JSON array of `MemoryEntry`; `list --json` honors `--stage`/`--entry-type`; `show --all --json` prints `{stage_id: [entries]}`). Success line after recording prints the new id: `✓ Recorded note <id> for stage <stage>`. `--entry-type receipt` is accepted by `list`.

`export.rs::format_memory_for_signal` and `format_memory_for_handoff` ignore `Receipt` entries; `commands/review/generate.rs` ignores them too.

## Steps

1. `types.rs`: the types above; `uuid` is already a dependency (`context/delivery.rs` uses `Uuid::new_v4`).
2. `parser.rs`: writer and reader; `constants.rs` untouched.
3. `storage.rs`: locked append. `spool.rs`: no format change (it serialises `MemoryEntry` as JSON — the new fields ride along); update `tests/spool.rs` fixtures to construct entries through the constructors.
4. `record.rs`: `--evidence` threading, session capture (`LOOM_SESSION_ID`), the id in the success line. `read.rs` + `formatters.rs`: `--json`, receipt rendering, the `entry_type` filter accepting `receipt`.
5. Fix every literal/exhaustive-match site the compiler reports outside `fs/memory` and `commands/memory` (list expected above).
6. Tests: in `fs/memory/mod.rs` inline tests — `an_entry_round_trips_its_id_full_timestamp_session_evidence_and_receipt`; `a_journal_read_the_next_day_keeps_yesterdays_date`; `two_concurrent_appends_never_interleave` (spawn two threads appending 3000-char entries 50 times each; parse; assert every entry parses and count == 100); `receipts_are_excluded_from_signal_and_handoff_exports`. In `commands/memory/handlers/tests.rs` (they are `#[serial]`, with `EnvGuard`): `note_records_evidence_and_session_and_prints_the_id`; `list_json_prints_entries_with_ids`.

## Done means

`cargo build --manifest-path loom/Cargo.toml` warning-free; `cargo test --lib fs::memory::` and `--lib commands::memory::` green; `loom/target/debug/loom memory note --help` shows `--evidence`.

## Proof (ONE narrow check, run once)

`cargo test --manifest-path loom/Cargo.toml --lib fs::memory` — skip if unsure.

## Constraints and traps

- The spool payload deliberately carries NO stage id (attribution is by worktree location — `spool.rs` module doc). Do not add one.
- `validate_content` limits (2000 chars) stay; `evidence` entries are validated with the same non-empty rule and a 256-char cap each, at most 16 per entry.
- `reject_stage_forgery` and `validate_stage_id` semantics are unchanged.
- `monitor/handlers.rs` writes a `## Summary` block by appending text (`write_summary`); keep `parse_journal` skipping it.
- Keep each file under 400 lines: `parser.rs` may split into `parser/{write,read}.rs`.
- No git commands.

## Report back

Files changed; the final `MemoryEntry`/`Receipt` definitions verbatim; every site outside your two modules you had to fix; anything unresolved.
