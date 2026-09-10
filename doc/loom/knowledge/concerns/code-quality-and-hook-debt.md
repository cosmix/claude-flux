# Code Quality And Hook Debt

> Code-quality/hook debt: oversized units, debug logging, duplicated tables

## Oversized Rust Units Remain Controlled Debt (2026-08-09)

The maintainability gate does not yet prove that every production Rust file is at most 400 lines or
every function at most 50 lines. Existing violations are recorded by exact identity and size in
`loom/maintainability-baseline.txt`; `loom/tests/maintainability.rs` rejects new entries, growth of an
existing entry, or a stale baseline entry. CI runs that gate, so the exception set can only stay flat
or shrink. Treat this as controlled decomposition debt, not as completed decomposition, and remove
an entry whenever its unit is brought under the limit.

## Debug Output in Production

`eprintln!` statements with 'Debug:' prefix in production code (complete.rs, orchestrator.rs). Should use tracing crate with log levels.

## Hook Debug Logging to /tmp/ (2026-03-31)

Several hooks (worktree-isolation.sh, commit-filter.sh, prefer-modern-tools.sh) hardcode debug log paths to `/tmp/<name>-debug.log`. Under `set -euo pipefail`, if `/tmp/` is not writable (e.g., sandboxed environments), the hook script exits immediately with error. `git-add-guard.sh` already uses a gated `debug()` pattern that only writes when `GIT_ADD_GUARD_DEBUG=1` is set. Other hooks should adopt the same pattern.

## Rust/Shell Heredoc Terminator Divergence

The Rust `strip_embedded_content` in `bash.rs:79` uses `line.trim() == marker` (tolerates indented terminators), while the shell version in `_common.sh:44` uses `$0 == marker` (exact line match). Both fail-safe but should be aligned for consistency.

## Remote Releases Do Not Deliver Hooks (PRE-EXISTING, 2026-07-28)

`install.sh::install_hooks_remote` fetches each hook from `${GITHUB_RELEASES}/<name>`, but
`.github/workflows/release.yml` publishes **no hook assets**. A clean remote install now fails
when all downloads miss rather than falsely reporting hook success, but the underlying delivery
contract is still broken: remote installation cannot install token-governance hooks, and
self-update has no hook update path either. This is currently low priority because development
installation (`dev-install.sh`) uses the repository's local hook files.

**Fix shape:** publish and checksum a complete hook bundle (or embed hooks in the binary), verify
its exact `LOOM_HOOKS` inventory before an atomic replacement, and add a clean-install fixture.

## No-Verify Doctrine Block Carries Only a Rust Example (2026-07-28)

The doctrine block must stay byte-identical across the signal, the template, and the hook's
refusal message, so it carries a single scoped-command example — a `cargo` one. A blocked Python
or Go subagent is shown a Rust example. `hooks/subagent-verify-guard.sh` is at the 400-line cap
with no slack, so the fix is to append language-specific examples **after** the pinned block as
explicitly hook-local guidance, the same way the `BLOCKED:` framing line already sits outside it.

## `subagent-verify-guard.sh` Still Regexes Raw Command Strings

It is the last hook that matches patterns against the raw command string, so it still cannot tell
an argument's _value_ from an argument's _mention_: text quoted inside a command is scanned as if
it were shell. The shared `loom_tokens_*` helpers it needs already exist in `hooks/_common.sh`.

It was left for last because its failure direction is the mild one — it blocks project-wide
build/test/lint runs by subagents, so a false positive strands a subagent rather than admitting a
dangerous command. That is a reason to do it last, not a reason it is safe.

**Converting the fifth hook is not a mechanical edit.** The 2026-08-26 conversion of three hooks
opened seven bypasses that the raw regexes had blocked, all found only by adversarial probing
against the OLD pattern — a fully green suite showed nothing. Read
`mistakes/shell-command-matchers.md` § "Converting a Raw-String Matcher to Token Scanning
Silently Narrows It" before starting, and budget for the differential testing it describes.

## Three Hook Files Exceed Rule 17's 400-Line Cap

Over CLAUDE.md Rule 17's 400-line file limit (`wc -l`, 2026-08-26):

| File                             | Lines |
| -------------------------------- | ----- |
| `hooks/_common.sh`               | 1197  |
| `hooks/commit-filter.sh`         | 489   |
| `hooks/subagent-verify-guard.sh` | 416   |

`_common.sh` roughly doubled when the token-scanning helpers landed; it was already over the cap
before that.

Splitting any of them needs a new hook file, and a new hook file needs FOUR registrations: an
`include_str!` const plus a `LOOM_HOOKS` entry in `loom/src/fs/permissions/constants.rs`, the
hooks config builder, BOTH copies of the `all_hooks` array in `install.sh`, and the exact-length
assertion in `loom/src/fs/permissions/tests/hooks_tests.rs`. Miss any one and the hook is
**silently dead** rather than broken — it simply never runs, and nothing reports it. That is why
this is a deliberate, separate change and not a drive-by trim.

## Duplicated Extension-to-Language Table (2026-08-17)

The same real-world fact is encoded twice and nothing pins the copies together:
`language.rs:117-129` maps a `str` to `Option<DetectedLanguage>`, and
`context/extract/lexical.rs:22-36` maps a `Path` to `NodeLanguage`. Both cover `rs`;
`ts,tsx,mts,cts`; `py,pyi`; `go`.

**Failure mode is a silently narrowing capability, not a crash.** Add `.jsx` to one and not
the other and the stage skill recommender classifies a file as TypeScript while the
source-graph tagger labels it `Other` and gives it lexical-only coverage — so
`loom map --outline` shows no symbols for a language loom otherwise claims to support, with no
error anywhere.

**Fix:** have `extract::lexical::language_for_path` delegate to `language::language_for_path`
(make it `pub(crate)`) rather than re-encoding the mapping. The test that matters asserts the
two agree over a shared fixture list — not more tests on either side.

## Low-Severity Cleanups Deferred From the Verification Gate (2026-08-17)

- `refresh/source_graph.rs:111` is a bare `let _ = mark_semantic_stale(..)` while that
  function's own doc (`:341-343`) says callers log and continue. The sibling caller in
  `merge_lifecycle` does log; this one does not.
- `delivery::plan_key` falls back to a `default` namespace when a stage file lacks `plan_id`,
  while `commands/context/record_edit::active_plan` and `MergeLifecycle::plan_id` both read
  `config.plan_id()`. They agree today, but a legacy stage file without `plan_id` would file
  delivery records under a different namespace than the graph overlay and dirty paths. Route
  all three through one derivation.
- **Resolved 2026-09-10:** `print_freshness_line` used to be duplicated across two files.
  There is no `commands/knowledge/status.rs` any more, removed by commit `36268adc`.
  The function now lives once in `commands/knowledge/context.rs:236`, called twice
  (`:163-164`) from that same file. No further action needed.

## Telemetry Under-Reports the Failures It Exists to Measure (2026-08-17)

`orchestrator/core/stage_telemetry.rs:25` calls `.ok()` on
`load_deliveries`, collapsing a genuine I/O error into the same
`ContextUnavailable { reason: "no delivery record for this session" }` as a real
miss. The file exists to measure how often stages spawn without a context brief, so
folding read failures into "no record" under-reports precisely the failures it is
for. **Fix:** give the error branch its own reason string.

**Resolved 2026-09-10:** the related claim that `telemetry::read_events` had no production caller is
no longer true — `commands/knowledge/telemetry.rs:27` now calls it from the `loom knowledge
telemetry` command. The `.ok()` under-reporting above is still open.

## `git/worktree/settings.rs` Is Still Over Cap After Its Tests Moved Out (2026-08-27, size reconfirmed 2026-09-10)

Splitting the inline test module into `tests_settings.rs` / `tests_settings_env.rs` took the file
from 1263 to 637 lines and its ledger entry from 1119 to 637 — a large win, but the production
half is still well over the 400-line guidance and remains a recorded violation in
`maintainability-baseline.txt`. It has since grown further, to 706 lines
(`maintainability-baseline.txt:26`).

It is genuinely multi-purpose: worktree `.work`/`.claude`/`CLAUDE.md` scaffold planting, settings
generation and permission merging, env scrubbing, and the git-exclude writer. Those are separable
— the exclude writer in particular (`add_to_gitignore_exclude`,
`add_worktree_exclude_patterns`, and the two `add_settings_local_to_*_gitignore` entry points)
is self-contained and has its own tests.

Not done as part of the sandbox bug fixes because a structural split is not a surgical change and
would have collided with four agents working the same tree. Worth a dedicated stage; note that
any file at or near its ledger cap must be refactored in the same change that grows it (see
`mistakes/sandbox-and-settings.md`).

## Ledger TUI: Tech Debt From the Live-Ledger-Dashboard Plan (2026-09-04)

- **`StageSummary.session_backend` has no reader.** Populated by the collector
  (`commands/status/data/collector.rs:225`, from `session.backend`) and delivered to spec by the payload-parity stage, but
  no ledger column, static renderer line, or attention entry ever reads it — every other occurrence in
  the tree is a test-fixture struct literal. Either give the ledger a BACKEND cell or drop the field
  from `StageSummary`.
- **`render_context_bar` (`commands/status/render/progress.rs:56`, re-exported at `commands/status/render/mod.rs:18`) is dead**, with
  no caller anywhere in the tree — `commands/status/ui/tui/ledger/cells.rs:100-116` independently reimplements the
  same five-cell bar with the same characters. Also apparently unused: `pub use theme::{StatusColors,
  Theme}` (`commands/status/ui/mod.rs:4`, every consumer imports `ui::theme::Theme` directly) and `pub use
  app::TuiApp` (`commands/status/ui/tui/mod.rs:25`, used only by `run_tui` in the same file).
- **The `.`/`..` stage-id path-component check exists in four places at three different strengths**
  (`commands/status/data/execution_models.rs:38`, `commands/memory/handlers/work_dir.rs:99`, `hooks/codex-forward.sh:43`,
  `hooks/spawn-guard.sh:309`) — see mistakes.md for the resulting divergence. Candidate for one shared
  helper.
- **Model-name display normalization (strip `claude-` prefix, strip trailing `-YYYYMMDD`) is
  implemented twice**: `commands/subagents/table.rs::display_model`/`strip_date_suffix` and
  `commands/status/data/execution_models.rs::normalize_model`. The status/ copy was written
  independently because `commands/subagents/table.rs` sits outside the payload-parity stage's declared files. Worth
  collapsing into one shared helper in a stage that owns both paths.
- **`commands/status/ui/tui/app.rs` is pinned at exactly the 400-line file cap** after this plan's fixes (doc comment,
  reconnect retry loop + constants). Any future addition must trim an equal number of lines elsewhere
  or move content into `commands/status/ui/tui/app_tests.rs`. Relatedly, `TuiApp::reconnect_after_read_error` cannot get a
  narrow unit test without mocking a live Unix socket plus a real
  `Terminal<CrosstermBackend<Stdout>>` — `TuiApp` owns both directly with no trait seam — so it is
  covered only by the full build.
- **The ledger footer's error branch has zero test coverage.** `TuiApp.last_error` is wired end to end
  from daemon exit / `Response::Error` through to `panels::render_footer`, but no ledger test ever
  sets it non-`None` (`commands/status/ui/tui/ledger/tests.rs:189`, `commands/status/ui/tui/ledger/layout.rs:295` both pass `None`).
