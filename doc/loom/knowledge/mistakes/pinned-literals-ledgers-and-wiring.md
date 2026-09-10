# Pinned Literals Ledgers And Wiring

> The maintainability ledger exact-match trap and goal-backward wiring checks pinning a pattern to a path.

## Why These Two Belong Together

Loom pins two different things to literal values, and both punish an otherwise-correct
refactor. Three stages of one plan hit the ledger and two hit the wiring pins, so
neither is an edge case: they are the standing tax on editing this repo. The unifying
rule is **check the pins before you fan out, and re-check them after every refactor
round** — not at commit time, when it means reopening every worker diff.

## The Maintainability Ledger Is EXACT-Match, Not a Ceiling

`loom/maintainability-baseline.txt`, enforced by `cargo test --test maintainability`
(`tests/maintainability/baseline.rs::validate_recorded_entries`).

**It errors on SHRINKAGE exactly as loudly as on growth.** "shrank from X to Y" is a
failure. So deleting a line from an over-limit function fails the gate until its entry
is lowered, and growth of a ledgered entry is never recordable — only refactoring
clears it. Stale entries are rejected too.

Consequences that bit real work:

- A file recorded at its exact count cannot take **one** more line. `cli/types.rs` sat
  at exactly 400/400, so adding a single clap variant was a new file violation.
  `merge_handler.rs` carried a 1430-line entry, so routing two cleanup call sites
  through a new primitive was only safe because the file ended up NET SHORTER.
- A ledgered FUNCTION entry (`dispatch.rs dispatch 205`) means the top-level match
  cannot take even one more arm.
- **The two size caps fight each other.** Extracting a helper to get a function under
  the 50-line cap pushed `refresh/source_graph.rs` to 407 against the 400-line file
  cap. Keep the new helper's doc comment tight rather than padding with blank lines.
- In a shared worktree the ledger drifts from many owners at once, so **no single
  subagent can reconcile it — the main agent must, after every subagent has landed.**
  A subagent forbidden to edit the baseline must REPORT its new number instead.
- A pasted mismatch list goes stale within minutes. Re-run the gate yourself as the
  source of truth rather than trusting even a recent list.

**Before fanning out:** `rg <target-path> loom/maintainability-baseline.txt` for every
file the work will touch, and design the extraction into an unledgered sibling
(`signals/helpers.rs`, `format/helpers.rs`, `cli/types_ops.rs`) into the brief up
front.

**Current pressure to know about:** fifteen files sit in the 390-400 band, i.e. one
edit from tripping the gate, and two are exactly AT 400 —
`orchestrator/signals/tests_doctrine.rs` and `commands/clean/sessions.rs`. Others
include `utils.rs` 399, `terminal/native/detection.rs` 399, `verify/before_after.rs`
398, `terminal/tmux/reconcile.rs` 398, `terminal/native/mod.rs` 398,
`context/refresh/source_graph.rs` 397, `git/merge/in_progress.rs` 396,
`context/coverage.rs` 394, `signals/format/brief.rs` 392, `verify/wiring_detection.rs`
390. None is a violation today. `wc -l` your target first; at >=390 plan the split in
the same round, because a split then collides with the wiring pins below.

**The scanner walks fixtures too.** `tests/maintainability/scanner.rs` parses EVERY
`.rs` file under the crate including `tests/fixtures/`, and returns `Err` on unbalanced
braces — so a deliberately-unparseable fixture fails the gate with an error pointing at
the fixture, not at real code. Fix used: the fixture used to be named `syntax_error.rs`;
it was renamed to `syntax_error.rs.broken` and the test passes a virtual `.rs` dispatch
path to the extractor instead. **Any intentionally-invalid fixture must not carry a real
`.rs` extension.**

## Goal-Backward Wiring Checks Pin a PATTERN to a PATH

A stage's `artifacts` and `wiring` lists in `.work/stages/<id>.md` name exact file
paths and exact literal patterns (e.g. pattern `extract::SourceGraphExtractor` in
source `loom/src/context/refresh.rs`). Any later change that moves the pattern out of
that path reports a wiring gap **for a feature that works perfectly**.

Hit twice in one plan, in two different shapes:

1. A file split for the 400-line limit deleted the pinned path.
2. Extracting two field assignments into a well-named helper removed the literal from
   the pinned file. Behaviour unchanged, every test green.

**The misleading signal is that the refactor is genuinely BETTER code** — a helper that
sets two related fields together reads well and keeps a ledgered file small — so
nothing in the diff looks wrong.

**Rules:**

- Treat each pattern+path pair as a **pinned interface**. `rg` them after every
  refactor round, not only at the start.
- When splitting a file, use the edition-2021 layout `<name>.rs` + `<name>/` subdir —
  never `<name>/mod.rs`, which deletes the pinned path. The context store used to be a
  flat `context/graph_store.rs`; it was renamed to that edition-2021 layout,
  `context/graph_store/mod.rs` plus sibling files, as it grew.
- When the honest fix conflicts with a line-count ceiling on the pinned file, recover
  the line inside that file rather than leaving the pattern hidden. Keeping the pinned
  literal at the pinned call site (inlining the assignment) beats delegating it.
- **For plan authors:** pin wiring patterns to the symbol's DEFINING module, where it
  cannot migrate, not to a call site a future extraction will relocate.

**A third shape: a LATER stage can break an EARLIER, already-merged stage's pin.**
Extracting `take_down_stage_agents` out of `orchestrator/core/event_handler.rs` into
`event_handler/stage_takedown.rs` (done purely to stay under the 400-line file limit) broke
an upstream stage's own wiring check, which pinned the literal pattern `kill_session` to
`loom/src/orchestrator/core/event_handler.rs` — and the stage-completion command refuses on
an AGGREGATED wiring re-verification that re-checks EVERY already-merged stage in the plan,
not just the one currently finishing, LONG AFTER the pinning stage had already merged. The
error names the pinning stage, which reads like that earlier stage regressed, when it is the
CURRENT stage's refactor that moved the code — read the file the pattern names, not the stage
the error names. Two rules follow: before extracting code out of any file, `rg` every stage's
wiring patterns for that file's path, since a refactor that satisfies one gate
(maintainability) can silently break a different, already-closed gate (wiring) belonging to a
sibling stage; and when the honest fix is impossible without moving the code, satisfy the
pattern honestly (e.g. a comment at the new call site naming the moved-to file) rather than
moving code back just to appease a grep.

## And While You Are Reading Plan Pins: Two Wiring-Test Traps

Both blocked a stage after all its acceptance criteria had passed.

- **A `wiring_test` must never reference the plan file that is executing it.** Loom
  renames the active plan to `IN_PROGRESS-<name>` in the MAIN repo at run start, and
  the worktree was cut from the base branch, so the named plan is not on the branch at
  all. Point such a check at a plan committed on the base branch instead. The same
  reason makes the plan unreadable from an integration-verify worktree: a plan authors
  want readable there must be COMMITTED before `loom run`.
- **Every wiring_test path is relative to WORKTREE + working_dir, not the repo root.**
  A `cd loom &&` prefix made `doc/plans/...` resolve to `loom/doc/plans/`, which does
  not exist; plans live at repo-root `doc/plans/`, so the path needed `../`.

## Related

- `mistakes/tests-that-cannot-fail.md` — the sibling class where the pinned literal is
  an assertion rather than a path.
- `mistakes/testing-and-lint.md` — the ledger as part of the wider lint discipline.

## A Bare mktemp -d in a Criterion Writes to the Operator's Real Home

**What happened:** Five acceptance criteria and two `wiring_tests` entries in one plan acquired a scratch `HOME` with a bare `mktemp -d`. Inside a stage session the sandbox denies the Darwin per-user temp dir, so `mkdtemp` failed, `H` was empty, and `HOME="$H" ./loom/target/debug/loom config -k update.check_interval_hours 6` resolved `dirs::home_dir()` to the operator's real home: `~/.loom/config.toml` was created on the operator's machine three separate times (twice during `loom check`, once during the stage's final `loom stage complete`), and the criteria passed because their follow-up `rg` read the same real file. Two of them were disputed and patched by the adjudicator; the rest were amended by the operator with `loom stage amend`, but the config-foundation amendments were recorded one minute after that stage's completion run had already used the old text.

**Why:** `;` after the `mktemp` assignment let the chain continue with an empty variable, and `HOME=""` is not "no home" — it falls through to `getpwuid`.

**Prevention:** In any criterion or wiring test, acquire scratch directories as `H=$(mktemp -d "${TMPDIR:-/tmp}/<name>.XXXXXX") && [ -n "$H" ] && ...`, join every step with `&&`, and never put a variable that can be empty into `HOME=`; prefer `LOOM_HOME="$H"` where the binary honours it (`user_config::config_path` does). Operator amendments to a running stage must land before the stage's `loom stage complete`; check `loom status` first.

**Fix:** The seven criteria were rewritten that way; `loom stage amend` gained `--field wiring-tests` (`AmendmentField::WiringTests`, `plan/amendment_fields.rs`) so wiring tests can be repaired through the audited path too.

## Maintainability Ratchet Fails on Shrinkage, Not Just Growth

**What happened:** While healing malformed `hooks`/`env`/`worktree` JSON containers in
`fs/permissions/{hooks,settings}.rs`, a refactor moved logic into a new
`fs/permissions/drift.rs` helper. That _shrank_ `settings.rs` and its
`ensure_loom_hooks_local` function below their recorded values in
`maintainability-baseline.txt`. `cargo test --test maintainability` still failed —
not for growing past the ceiling, but with "shrank from N to M lines; lower the entry".

**Why:** `maintainability-baseline.txt` entries are exact counts, not ceilings — the
ratchet (`loom/tests/maintainability.rs` / `tests/maintainability/baseline.rs`) fails
symmetrically on drift in either direction, at both file and per-function granularity.

**Prevention:** After any edit to a file/function with a baseline entry, run
`cargo test --test maintainability` once before finishing — don't just eyeball
`wc -l` against the ceiling. Treat any reported "shrank" line as mandatory, not
optional: lower that exact entry to the reported value.

**Fix:** Update only the entries the test names, to the exact value it reports —
never round, never touch entries it didn't flag, never add a new entry for a file/
function that's still under the 400/50-line limit (those never need one).

## `loom stage complete`'s Unwired-File Check Has a False Positive on `#[path]` Test Modules

**What happened:** `loom/src/commands/status/render/attention_model_tests.rs` is wired at
`commands/status/render/attention_model.rs`'s tail as `#[cfg(test)] #[path = "attention_model_tests.rs"] mod tests;` — the
same pattern `commands/status/render/attention.rs`/`commands/status/render/attention_tests.rs` and `commands/status/data/collector.rs`/`commands/status/data/collector_tests.rs` already use —
but the finalization command's unwired-file checker looks for a `mod attention_model_tests;` declaration
and finds none, so it flags the file as unwired even though `cargo test --lib
commands::status::render::attention_model::tests::` reaches it fine.

**Prevention:** when a stage adds a `#[path]`-included test file, record a note before finalizing the
stage so the false flag isn't mistaken for a real gap; the checker itself is the thing that needs
fixing, not the file.

## Maintainability Baseline: Moving or Growing Past a Ledger Function Needs Care Beyond `wc -l`

Three mechanics not covered by the existing "fails on shrinkage too" entry above:

- **Baseline entries are exact-match, both directions, at once.** `validate_recorded_entries` errors
  on a measured count either ABOVE or BELOW the recorded value against the SAME baseline row — only
  landing exactly on it (or dropping fully under the general 400/50 threshold and removing the row)
  avoids both errors. Comment reflow (never content deletion) is a legitimate way to land exactly on
  a target line count.
- **Relocating an oversized function to a new file needs a NEW baseline entry at the new path** — the
  function's own line count doesn't change when it moves, so a violation that already had a recorded
  entry at the old path is a fresh "unrecorded violation" at the new one; `find_unrecorded_violations`
  flags it regardless of whether the same violation existed elsewhere before.
- **`rustfmt` always splits `#[cfg(test)] mod x;` onto two lines**, even written on one — registering
  a new test submodule costs 2 lines with no way around it; that's real structural cost to budget into
  a baseline update, not padding to trim away.
