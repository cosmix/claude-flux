---
---
# Ledger Tui Rendering

> Wide-glyph padding, fan-out duplication, and latent panics in the ledger TUI.

## Ledger TUI: Wide Glyphs, Fan-Out Duplication, and Latent Panics (2026-09-04)

**East-Asian Wide glyphs break `chars().count()` padding.** `⚡` (U+26A1, the `MergeConflict` icon)
is East-Asian Wide: one `char`, two terminal cells. `commands/status/ui/tui/ledger/cells.rs` originally padded by
`chars().count()`, so a `MergeConflict` row shifted every later column by one cell and clipped the
MERGE cell; `commands/status/ui/tui/ledger/header.rs` already used `Span::width()` (unicode-width aware) as `text_width()`.
**Prevention:** any TUI cell padding in this repo measures with `Span::width()`, never
`chars().count()` — the status icon set contains at least one wide glyph.

**Fanning a UI out by file reproduces the same defect in every worker's slice.** Splitting the ledger
across five parallel workers, each owning a disjoint file set, produced the SAME defect class three
times independently: a `chars().count()` pad in `commands/status/ui/tui/ledger/columns.rs`, a `{:<20}` format pad on a stage id in
`commands/status/ui/tui/ledger/panels.rs`, a `{:<12}` pad on a status label in `commands/status/ui/tui/ledger/legend.rs`, plus three byte-identical copies of a
`cut_line`/`spans_width` truncation loop across `header.rs`, `panels.rs`, and `legend.rs` (all under `commands/status/ui/tui/ledger/`). No worker
could see a sibling's shared helper while writing. **Prevention:** when a plan fans UI work out by
file, give the foundation module (the one worker who runs first, alone) the width/truncation helpers
up front and name them explicitly in every other worker's brief; otherwise budget an orchestrator
pass afterward to converge the duplicates.

**A design table's own example content did not fit its own column width.** The MODELS column is 16
cells, pinned by `FULL_WIDTH=120` (`commands/status/ui/tui/ledger/mod.rs:40`) and the drop-order termination proof at width
64 — it cannot widen. The plan's design table and a worker brief both used `opus›sonnet,terra` (17
cells) as example acceptance content, which the column's own truncation rule cannot render intact
(the real output is `opus›sonnet+1`). **Prevention:** when a design table gives both a column width
and example content, check the example actually fits before writing it into an acceptance criterion.

**Two latent panics/overflows shipped with no test reaching them.** `commands/status/ui/tui/ledger/header.rs` collects
`crate::LOGO.lines()` into a `Vec` and indexes `logos[0..3]`; `LOGO` (`loom/src/lib.rs:36`) happens to have
exactly four lines today, so shortening the banner would panic the dashboard at render time — prefer
`logos.get(n).copied().unwrap_or("")`. And `commands/status/ui/tui/state.rs` did `self.scroll_y + delta as u16` while
the sibling negative-delta branch two lines above correctly saturates; only reachable above ~32768
stages, so hardening rather than a live bug. **Prevention:** when one branch of a numeric pair
saturates and its sibling does not, that asymmetry is the bug even when the overflow is unreachable
today — search for the sibling branch, don't just fix the one that was reported.

## The Live Dashboard Rendered a Frozen Accumulator as a Clock

**What happened:** the TIME column of the ledger behind `loom status --live` showed `0s` for every
executing stage, for the whole of the run, and only jumped to a real number once the stage finished.

**Why:** `Stage::execution_secs` is a bank, not a clock. `begin_attempt`
(`loom/src/models/stage/methods.rs:573`) seeds it to `Some(0)`, and only `accumulate_attempt_time`
(`methods.rs:585`) credits an attempt's seconds — when that attempt ENDS, on completion, crash or
handoff. `time_cell` (`loom/src/commands/status/ui/tui/ledger/cells.rs:301`) rendered
`execution_secs.or(elapsed_secs)`, and because the frozen value is `Some(0)` rather than `None`, the
`elapsed_secs` fallback could never fire. A regression from cad9ee4b: the tree renderer the ledger
replaced computed executing time as `now - started_at`, which ticked.

**Prevention:** before displaying a persisted duration field as a live number, read where it is
WRITTEN, not just where it is declared. A field credited at the end of a unit of work reads frozen
for the whole of that work, and an `Option` fallback chain rescues nothing when the frozen value is
`Some(0)`.

**Fix:** `loom/src/commands/status/data/timing.rs` — `execution_secs_live` adds the in-flight
attempt (`now - attempt_started_at`, falling back to `started_at`) on top of the banked total, for
`Executing` stages only, and `build_stage_summary` calls it, so every status surface fed by
`collect_status_data` ticks.
