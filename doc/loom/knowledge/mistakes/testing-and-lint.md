# Testing And Lint

> Lint and test discipline: --all-targets, --no-fail-fast, headless CI, ambient git config and inherited descriptors in tests, the stub checker, the maintainability ledger, and reviewer claims.

## Test Code: Struct Init Without Default

**Mistake:** Stage struct tests use explicit constructors without `..Default::default()`. Adding new fields breaks ~10 locations.
**Fix:** Use `..Stage::default()` pattern. Also check `tests/` directory (not just `src/`) when adding fields.

## Debug Output in Production

**Mistake:** `eprintln!` with `Debug:` prefix left in production code.
**Fix:** Use `tracing` crate with proper log levels.

## Clippy --all-targets Required to Catch Test-Module Lints (2026-05-12)

**What happened:** `cargo clippy -- -D warnings` (without `--all-targets`) did not compile test modules, so a style lint in `src/hooks/generator.rs` (items after a test module) went undetected during per-stage acceptance and only surfaced at integration-verify.

**Why:** `cargo clippy` without `--all-targets` compiles only the default target (lib + bin). Test code (`#[cfg(test)] mod tests { ... }`) is in a different target and requires `--all-targets` to be included.

**Prevention:** Stage acceptance criteria that include a clippy check should always use:

```bash
cargo clippy --all-targets -- -D warnings
```

Not `cargo clippy -- -D warnings`. The `--workspace` flag is also useful in monorepos.

## Reviewer False Alarm: Verify Behavior Changes Against the Diff (2026-05-12)

**What happened:** An integration-verify reviewer flagged a "HIGH native regression" in `hooks/generator.rs`, claiming the new backend match arm introduced double-firing of global hooks on native worktrees. The claim was false — the native branch was already unconditionally calling `configure_loom_hooks(obj)` before the change; the new commit only added the container arm.

**Why:** The reviewer analyzed the stage description's framing rather than the actual diff. The description said "branching on config.backend" which sounds like it changes native behavior; the diff showed the native arm was structurally identical to the pre-existing unconditional call.

**Prevention:** When a reviewer asserts a behavior change, verify against the actual diff:

```bash
git show <commit>~1 -- <file>  # before
git show <commit> -- <file>    # after
```

Do not trust verbal descriptions of what a commit does — always compare before/after diffs directly.

## TODO in Rust String Literals Triggers ArtifactStub Checker (2026-06-15)

**What happened:** A Rust format string inside a `push_str()` call contained the word "TODO" as a reference to a future task in the documentation text it was generating (not actual stub code). `loom stage complete` rejected it with an ArtifactStub error, blocking completion.

**Misleading signal:** The word appeared in a prompt or documentation string — semantically it was text content, not a code stub. The ArtifactStub checker scans the raw file content without context.

**Prevention:** Before completing a stage, scan your own format strings and string literals with `rg "TODO|FIXME|unimplemented" loom/src/<your-file>`. If the word appears as content in a string (e.g., as part of documentation text), rephrase to avoid the keyword — "fix later", "outstanding item", "remaining task", or similar.

**Fix:** Rephrased the string literal to avoid the TODO keyword.

## Git-Shelling Tests Must Isolate Ambient Config and Assert Setup Steps (2026-06-15)

**What happened:** `git::merge::tests::merge_stage_refuses_when_merge_head_set` passed locally but failed on CI with `MERGE_HEAD must be set` at the _setup_ assertion. The setup used a `run` closure that called `Command::new("git")…output().unwrap()` — `.unwrap()` only catches spawn failure, not a non-zero exit. So when an ambient git config broke a setup commit, every step silently no-op'd and the failure only surfaced lines later as a confusing MERGE_HEAD assertion. Reproduced exactly by setting `commit.gpgsign=true` (no key) in the global config: the seed commit fails → `checkout main` fails → the "merge" runs on the wrong branch → no conflict → no MERGE_HEAD.

**Misleading signal:** "Passes locally, fails in CI" on a pure-logic test that _must_ be deterministic. The panic points at the symptom (`MERGE_HEAD` absent), not the cause (a swallowed setup-commit failure several lines up). Tests that shell out to `git` inherit the runner's global/system config — `commit.gpgsign`, `core.hooksPath`, templates — none of which exist on a clean dev box.

**Why:** Two compounding defects: (1) the helper discarded git exit status, so setup failures were invisible; (2) the repo was not isolated from ambient git config, so a hostile global setting could break commits/merges. Note `#[serial]` only serializes against other `#[serial]` tests — the merge tests are non-serial and run alongside `repository.rs`'s `GIT_CONFIG_GLOBAL`-mutating tests, another reason to pin config per-Command rather than rely on the process environment.

**Prevention:** Any test that shells out to `git` must (a) assert each setup command's exit status and surface stderr — never `output().unwrap()` and drop the status; and (b) neutralize ambient config by setting `GIT_CONFIG_GLOBAL`/`GIT_CONFIG_SYSTEM` to nonexistent paths and `GIT_CONFIG_NOSYSTEM=1` on the `Command` (so it survives a polluted process env too). Set identity via local config. ~10 test files here use the same `init_repo`/`run_git` shape (`in_progress.rs`, `merge_attribution.rs`, `recovery.rs`, `merge_verify.rs`, …); the asserting ones at least fail loudly, but none isolate ambient config — port the `isolated_git`/`git_ok` helpers from `git/merge/mod.rs` if they ever flake.

**Fix:** Added `isolated_git`/`git_ok` helpers in `git/merge/mod.rs` tests: every setup step asserts success, the conflicting merge dumps stdout/stderr if MERGE_HEAD is absent, and all invocations run with global/system config disabled. Verified green under a forced-`gpgsign` global config that previously reproduced the failure.

## `cargo test` Stops at the First Failing TARGET — a Green Tail Is Not a Green Suite (2026-08-10)

**What happened:** A local `cargo test` run was reported as "2048 passed, suite green". It was not: the run aborted in `tests/e2e` and never executed `tests/maintainability`, `tests/phantom_merge`, or six other targets. The push then failed on `maintainability`, which had been failing the whole time.

**Why it is easy to miss:** the output ends with a plausible `test result: ok` line from the last target that _did_ run, and the `error: test failed, to rerun pass --test e2e` line scrolls past. Nothing announces the nine targets that were skipped. It compounds when the aborting failure is environmental (a sandboxed run cannot create `TMUX_TMPDIR`), because that failure looks ignorable — and ignoring it silently discards the rest of the suite.

**Prevention:** verify with `cargo test --no-fail-fast` so every target runs, and count the `Running tests/...` lines against the target list rather than reading the tail. The pre-push hook used plain `cargo test` and stopped early too — fixed 2026-08-14: it now runs `cargo test --all-targets --no-fail-fast`, so a passing hook proves every target and a failing one reports every failure at once. Never report a suite green off a run that aborted.

**Recurrence (2026-08-10, same day):** it happened again, in an interactive session, in the shape this note predicts exactly. The agent ran `cargo test --all-targets`, saw the two sandboxed tmux e2e failures, verified they were pre-existing by stashing, and reported the suite green apart from them — never noticing that `maintainability` and seven other targets had not run at all. The push then failed on `maintainability`, whose four violations the same change had introduced. Knowing the rule did not help, because the environmental failure supplied a ready-made reason to stop looking.

**Detection rule (mechanical, use this instead of judgement):** `error: test failed, to rerun pass --test <name>` in the output means the run is INCOMPLETE, regardless of how many `test result: ok` lines precede it and regardless of whether that failure is yours. Treat it as a hard stop: re-run with `--no-fail-fast` and read every `test result` line before saying anything about the suite. "Pre-existing and environmental" justifies ignoring a _failure_; it never justifies ignoring the _truncation_ that failure caused.

## The Pre-Commit Markdown Lint Silently Skips Itself Under the Bash Sandbox (2026-08-10)

**What happened:** every commit printed `Linting markdown files...` and succeeded, yet the markdown was never linted or auto-fixed. The pre-push hook then lints for real and rejected the push over four `MD049/emphasis-style` errors (asterisk emphasis where this repo enforces underscore).

**Why:** `.githooks/pre-commit` runs `xargs bunx markdownlint-cli2 --fix 2>/dev/null || true`. Under the sandbox `bunx` dies with `bun is unable to write files to tempdir: ReadOnlyFileSystem` (newer bun: `Unexpected accessing temporary directory. Please set $BUN_TMPDIR or $BUN_INSTALL`) — bun wants `/tmp` and `~/.bun/install/cache`, both outside the write allowlist — and both the error and the exit code are discarded by design, so the step is indistinguishable from a clean pass. Exit 0 is not success (CLAUDE.md Rule 13); a hook that swallows stderr can only ever look green.

**Prevention:** lint markdown explicitly before pushing, redirecting bun's cache to a writable dir so it actually runs:

```bash
export TMPDIR="$TMPDIR" BUN_INSTALL_CACHE_DIR="$TMPDIR/bun-cache"
git ls-files '*.md' | rg -v '^doc/plans/' | rg -v '^loom/tests/fixtures/' \
  | xargs bunx markdownlint-cli2
```

Expect `Summary: 0 issues`. `.markdownlint.json` disables MD013/MD033/MD036/MD041/MD060, so long lines are fine — but emphasis style, heading spacing and list style are all enforced. Running `markdownlint-cli2.mjs` straight from the bun cache with `node` does NOT work: that directory has no `node_modules`, so it fails on `Cannot find package 'globby'`.

**A second, independent root cause with the identical symptom (2026-08-30):** even once the bun-cache write path is allowlisted, the same step tries to reach `registry.npmjs.org` and is DENIED by the sandbox's network filter on every commit — again printing nothing to the visible hook output and exiting 0, with the denial visible only in the harness's own `<sandbox_violations>` block, never in the hook's stdout/stderr. Whichever of the two causes is live in a given sandbox configuration, the fix is the same: lint explicitly out-of-band before pushing (command above) rather than trusting the pre-commit step's silence, and check `<sandbox_violations>` after any hook step that touches the network before believing "no output" means "nothing needed doing."

**Push-time behaviour since 2026-08-31:** `.githooks/pre-push` now runs `markdownlint-cli2 --fix` before it reports, so only violations the fixer cannot repair (MD024 duplicate headings, MD025 multiple H1s) block a push; a run that changed files still stops the push and names them, because the commits being pushed still carry the unfixed markdown. It also stopped hiding the linter's output behind `2>/dev/null`, and treats a missing `markdownlint-cli2 v` banner in that output as "the linter never ran" rather than as a lint failure — `bunx` exits 1 for both, so the exit code alone cannot tell them apart.

## Headless CI Has No Terminal Emulator — Pin `LOOM_TERMINAL` in Tests That Build an Orchestrator (2026-08-10)

**What happened:** `merge_handler_attempt_tests::merge_probe_failure_does_not_consume_resolver_attempt_budget` passed on every dev box and failed in CI with `No terminal emulator found. Set TERMINAL environment variable or install one of: kitty, alacritty, ...`. It recurred on 2026-09-03 in five tests across `orchestrator/core/event_handler/verdict_retirement_tests.rs` and `orchestrator/core/stage_executor_tests.rs`.

**Why:** `Orchestrator::new` builds the session backend from the persisted `[terminal]` config (`SessionBackend::from_config`, `orchestrator/terminal/backend.rs:96-112`). With `SessionBackendKind::Native` — which is what an absent config resolves to — it eagerly constructs a `NativeBackend`, and `detect_terminal` probes the host. A GitHub runner has no emulator installed, so construction fails and the `.unwrap()` panics: a pure state-machine assertion killed by the host environment.

**Prevention — the heading above is stale.** `LOOM_TERMINAL` still works, but the mechanism the tree now uses is the config, not the env var: write a `TerminalConfig { backend: SessionBackendKind::Tmux }` into the work dir with `fs::work_dir::write_terminal_config` before calling `Orchestrator::new`. Tmux leaves the native lane unbuilt, so no detection runs (`backend.rs:99-102`, asserted by `backend::tests::from_config_tmux_leaves_the_native_lane_unbuilt`). No env var, no `#[serial]`. Working helpers: `event_handler/tests.rs::handoff_work_dir`, `stage_executor_tests.rs::work_dir`, `event_handler/stalled_judge_tests.rs::work_root`.

**The trap that caused the recurrence: the helper and the test must name the SAME directory.** `write_terminal_config(dir)` and `read_terminal_config(dir)` both key off the directory handed to `OrchestratorConfig::work_dir`. `handoff_work_dir()` returns the `TempDir`, not the work path, so each test recomputes it — and five tests recomputed it as `temp.path().join(".work")` while the helper had written to `temp.path().join(".loom").join("work")`. No config there, so the native lane came back and detection ran. The mismatch is invisible on macOS, where detection succeeds. When adding a test to those files, copy the `.loom/work` join from a neighbouring test rather than inventing the path.

**Detection rule:** to reproduce headless failures locally, build a `PATH` of symlinks that excludes every terminal binary and run the prebuilt test binaries with `DISPLAY`/`WAYLAND_DISPLAY`/`TERMINAL`/`LOOM_TERMINAL` unset. A fix verified only on a machine that has a terminal proves nothing.

## An Inherited Descriptor Keeps an flock Alive After the Owner Releases It (2026-08-10)

**What happened:** `daemon::server::lock::tests::held_and_free_lock_states_are_distinct` failed roughly one run in nine under full parallel load, and passed every time in isolation. After `drop(guard)`, `inspect_lock` occasionally still reported `Held`.

**Why:** flock ownership belongs to the open file description, and `fork` duplicates it. Any _other_ test in the same binary that spawns a command inherits the lock descriptor for the window between fork and exec, and that inherited copy holds the lock alive even after the owner closes its own descriptor — `O_CLOEXEC` drops it at exec, not at fork. Demonstrated directly: a child that sleeps without exec'ing leaves the probe reading `HELD`; the same child with `execl` leaves it `FREE`.

**Prevention:** a test asserting "released" against a flock cannot assume the next probe observes it. Poll to a deadline instead of probing once, and report the last observed state (`held` vs `indeterminate`) in the failure message so the next failure is diagnosable. More generally, treat single-probe assertions about process-global OS state as flaky-by-construction in a multithreaded test binary that also spawns processes.

**Note for production:** the same window applies to the daemon singleton lock. A child forked during the microseconds `orchestrator.lock` is open can hold it past daemon exit until that child execs, so an immediate restart could briefly see "another daemon instance holds the singleton lock". Not observed in practice; recorded so the symptom is recognisable.

## Growing a Function That Carries a Maintainability Baseline Entry Breaks the Gate (2026-08-10)

**What happened:** A five-line fix inside `generate_index` (`src/fs/knowledge/index.rs`) pushed it from 53 to 58 lines and failed `cargo test --test maintainability`, blocking the push.

**Why:** `maintainability-baseline.txt` records exact line counts for every function already over the 50-line limit. The ledger is a debt list, not a budget — entries may shrink but never grow, and an entry that no longer corresponds to a violation is rejected as stale.

**Also check the FILE entry, not just the function.** The ledger records both, so a change that keeps every function small can still fail on the file total — and four of them can fail at once, as the codex sandbox work did (`repair.rs`, `fs/permissions/settings.rs`, `sandbox/settings.rs`, plus three functions). The move that satisfies the ledger honestly is extraction: lift the new code into a new module (`fs/permissions/codex_sandbox.rs`, `commands/repair/settings_checks.rs`), which carries no entry at all while it stays under 400 lines, then lower the now-smaller entries to their measured values. Growth is never recordable; shrinkage must be recorded.

**Prevention:** before adding lines to a function, `rg '<fn name>' loom/maintainability-baseline.txt`. If it is listed, refactor rather than extend — and when the refactor drops it under the limit, DELETE the entry rather than lowering it. Prove behaviour is unchanged by regenerating the artifact and diffing (for INDEX.md: `loom knowledge sync` then `git status --porcelain`, expecting no change).

**Two more shapes, from wiring the viewer reconciler (2026-08-11):** (1) a cross-cutting per-tick
call could not live in `orchestrator.rs` (file ledgered at 564) NOR in `event_handler.rs` (468) —
even a one-line addition to a ledgered FILE fails the gate, so a new hook must find an unledgered
host with headroom (`Monitor::poll` in `monitor/core.rs`, 128 lines, same per-tick semantics).
Check the file ledger BEFORE choosing a call site, not after the test fails. (2) MOVING a
grandfathered over-limit function is fine: the ledger accepts deleting the entry at the old path
and re-adding it, same measured size, at the new path (alphabetical within the `function` group).
Relocated debt is not new debt — but only new-code violations must be refactored instead of
ledgered.

## Ledger Growth Reached Push Time Again — Gates Moved Earlier (2026-08-14)

**What happened:** the maintainability failure class recurred a third time: a guidance commit (`deecb23e`) duplicated a 13-line block into two ledgered `signals/` functions and grew four tests, and the breakage was only discovered when the pre-push hook ran the suite. The truncated-`cargo test` trap above is exactly how it went unnoticed at commit time.

**Fix (mechanical, both hooks in `loom/.githooks/`):**

- `pre-commit` now runs `cargo test --quiet --test maintainability` after formatting — ledger growth blocks the commit itself, not the eventual push. Fast when the build is warm; the first commit in a cold worktree pays a compile.
- `pre-push` now mirrors CI: `cargo clippy --all-targets -- -D warnings`, the rustdoc gate, and `cargo test --all-targets --no-fail-fast`. Since 2026-09-05 it also runs `scripts/flake-check.sh` (the CI flake job) last and warns when `rustup check` reports a newer stable.

**Prevention:** when a failure class recurs at push time, the fix is to move its cheapest sufficient check to commit time, not to write another reminder. The ledger fix itself followed the standard shape: extract the duplicated block into an unledgered module (`signals/helpers.rs::append_settled_completion_rules`), never raise a ledger entry.

## A `[`link`]` to a Private Item Fails the Docs Build — Four Recurrences, Now Gated at Commit Time (2026-08-11)

**What happened:** CI's `Documentation` job failed on main while build, test, clippy, fmt, maintainability, audit and deny were all green. Two module doc comments used intra-doc link syntax for private functions — `` [`tmux_endpoint_ready`] `` in `src/commands/attach/mod.rs` and `` [`kind_env`] `` in `src/orchestrator/terminal/native/wrapper.rs`. Both targets are private `fn`s referenced from public docs.

**Why:** the job runs `cargo doc --workspace --all-features --no-deps` under `RUSTDOCFLAGS: -D warnings`, which promotes `rustdoc::private_intra_doc_links` to an error. That lint is a rustdoc lint — `cargo build`, `cargo clippy` and `cargo test` never evaluate it, so the usual pre-push loop cannot see it. Prose that merely _mentions_ a private helper is the common way to trip it.

**Prevention — a syntax rule applied while typing, not a command remembered later:** in a doc comment on a `pub` item, `` [`name`] `` is a promise that `name` resolves as a public path from that module. If the target is a private `fn`, a field, or a local, write a plain code span `` `name` ``. Only `--document-private-items` would make the bracketed form resolve, and neither CI nor the hook passes it. Every `[` typed inside a `///` or `//!` block is therefore a claim to check before the edit is finished; the repair is mechanical, `` s/\[`name`\]/`name`/ ``.

The gate that catches what slips through is the whole list in `loom/.githooks/pre-push` — fmt, markdownlint, clippy, rustdoc, cargo-audit, `cargo test --all-targets --no-fail-fast`, `scripts/flake-check.sh`. Read that file and run its steps before calling a change ready; a hand-assembled subset from memory is not the gate. Its rustdoc step (added 2026-08-14, between clippy and cargo-audit) is `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`, and it is worth running alone right after any doc-comment edit — it takes seconds and is the only local check that evaluates rustdoc lints at all.

**Recurrence (2026-08-26):** it happened again, in an interactive session, in exactly the shape this note predicts. Two doc comments written during a bug-fix series linked `` [`catalog_failure_context`] `` and `` [`splice_section`] `` — both private. The session ran build, clippy, fmt, the full test suite and the hook test suite, called the gate green, and committed five times; the pre-push hook then failed on the docs job. The lesson is not "remember rustdoc" — this note already said that — it is that a hand-picked set of checks is not the gate. The gate is the list in `loom/.githooks/pre-push`: fmt, markdownlint, clippy, rustdoc, cargo-audit, `cargo test --all-targets --no-fail-fast`, `scripts/flake-check.sh`. Read that file and run its steps before claiming a change is ready to push, rather than assembling a plausible-looking subset from memory.

**Recurrence (2026-08-31), third time, blocking `git push`:** `` [`queue_dispute_request`] `` in `src/commands/stage/dispute_criteria.rs:35`, `` [`queue_block_request`] `` in `src/commands/stage/state.rs:36`, `` [`drain_spool`] `` in `src/fs/stage_request/apply.rs:25` — three private targets, written across the stages of the block/dispute series, each in a sentence explaining what the private helper does. That context is what invites the mistake: the prose reads as a cross-reference, so the brackets get typed. Three occurrences of one shape means the defect is at authoring time, and a fourth "remember to run the docs job" would not have prevented any of them — the bracket rule above is what has to be applied while the comment is being written. Each doc comment an agent writes about a helper it just wrote is a place to apply it.

**Recurrence (2026-09-05), fourth time, blocking the user's own `git push`:** `` [`POLL_INTERVAL`] `` (twice), `` [`MAX_BACKOFF`] `` and `` [`SLEEP_SLICE`] `` in `loom/src/quota/poller.rs:4-5,31` — private `const`s this time, in the module doc and on `pub fn spawn_quota_poller`; the first three occurrences were private `fn`s. A constant named in prose about cadence reads like public API, which is the same invitation the helper-explaining sentences gave before.

**Fix (mechanical, 2026-09-05):** `loom/.githooks/pre-commit` now runs the rustdoc gate itself, guarded by a staged-diff test — `git diff --cached --diff-filter=ACM -U0 -- '*.rs' | grep -qE '^\+[[:space:]]*(///|//!)'` — so it fires on the commits that can break the docs build and skips every other commit. Warm, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps` costs about 5 seconds. This follows the ledger precedent recorded above: once a class has reached push time repeatedly, move its cheapest sufficient check to commit time. Four occurrences means a fifth written reminder would have failed the same way the first four did; the bracket rule still applies while typing, but the hook is what now enforces it.

**Reading a CI failure without admin rights:** `gh run view <id> --log-failed` returns `HTTP 403: Must have admin rights` on this repo. The annotations endpoint does not require admin and carries both the failing step's error and every runner warning:

```bash
gh api repos/<owner>/<repo>/actions/runs/<run-id>/jobs --jq '.jobs[] | "\(.name)\t\(.conclusion)"'
for id in $(gh api repos/<owner>/<repo>/actions/runs/<run-id>/jobs --jq '.jobs[].id'); do
  gh api repos/<owner>/<repo>/check-runs/$id/annotations --jq '.[] | "\(.annotation_level): \(.message)"'
done | sort -u
```

## The Maintainability Gate Is Repo-Wide, So a Concurrent Session's Violations Block Your Commit (2026-08-19)

**What happened:** two finished, fully verified commits could not land. The pre-commit hook runs
`cargo test --test maintainability` over the whole tree and reported 17 violations. Only 2 were
mine; the other 15 were in `src/sandbox/`, `src/plan/schema/` and `src/models/stage/` — files
belonging to a _different_ Claude session working in the same checkout at the same time. Two commit
attempts were spent before the ownership split was noticed.

**Why:** the ledger gate has no notion of staged scope. It measures the WORKING TREE, so any
uncommitted work anywhere in the repo — including another agent's, including files you have never
opened — participates in your commit. Nothing in the failure output attributes a violation to an
author, so the natural reading is that your own change caused all of them.

**Prevention:** when the gate fails, split the list by owner before touching anything.
`git status --porcelain` tells you which files you actually modified; fix only those. Never "fix"
another session's violation, and never edit `maintainability-baseline.txt` to clear one. If the
remaining failures are entirely someone else's in-flight work, the commit is blocked on THEM, not
on you — surface that and let the operator decide, rather than reaching for `--no-verify` on your
own authority.

**Second-order effect worth knowing:** the same hook runs `cargo fmt` across the whole crate before
the gate. Committing therefore reformats another session's in-progress files in the working tree.
This is harmless in itself — formatting is idempotent and non-semantic — but an agent whose file
changes underneath it will see its next `Edit` fail on a content mismatch. Re-read the file rather
than forcing a `Write`.

**Related:** the entry above covers the ledger's own rule (growth never recordable, shrinkage must
be recorded). That rule still applies to your own violations here; this entry is only about
correctly attributing which violations _are_ yours.

## CI's Clippy Tracks Rustup `stable`, So a New Rust Release Breaks Main With No Code Change (2026-08-26)

**What happened:** three consecutive pushes to main failed CI with only the `Clippy` job red —
build, both test matrices, docs, fmt, maintainability, audit and deny all green, and the same
`cargo clippy --all-targets -- -D warnings` passed locally. The first failure was on 2026-08-21;
the last green run was 2026-08-20. Nothing in those commits touched Rust — they were
`docs(knowledge)` commits. The real trigger was Rust **1.98.0**, released 2026-08-20, whose new
`chunks_exact_to_as_chunks` (style, warn-by-default) fires on `ticks.chunks_exact(2)` in
`src/context/lexical/evidence.rs`. The local toolchain was still 1.97.1, which has no such lint.

**Why:** `.github/workflows/ci.yml` installs `dtolnay/rust-toolchain@stable` and the repo pins no
toolchain file and no `rust-version`. CI therefore silently follows the newest stable, while
a developer machine sits on whatever `rustup update` last fetched. Every six weeks a new stable can
turn previously-clean code into `-D warnings` errors, and the offending commit will be whichever
one happened to push next — usually one that changed nothing relevant.

**Prevention:** when Clippy alone fails and the diff cannot explain it, check the toolchain gap
first — do not read the diff for a cause it does not contain:

```bash
rustc --version                                                   # local
curl -sS https://static.rust-lang.org/dist/channel-rust-stable.toml | rg -m1 '^version = "1\.'
```

If they differ, read the lint list for the intervening release before anything else — the
`## Rust <version>` section of
`https://raw.githubusercontent.com/rust-lang/rust-clippy/master/CHANGELOG.md` names every new lint
and every widened one. Only `style`, `complexity`, `suspicious`, `correctness` and `perf` additions
can break this gate; `pedantic` and `nursery` entries are allow-by-default and irrelevant here.

**Reproducing the newer toolchain without touching `~/.rustup`:** the sandbox denies writes there,
so `rustup update` fails with `Read-only file system`. Redirect all three homes into scratch space
instead — the toolchain download and the crate re-fetch both go over the proxy fine:

```bash
export RUSTUP_HOME=$TMPDIR/rustup CARGO_HOME=$TMPDIR/cargo CARGO_TARGET_DIR=$TMPDIR/target198
~/.cargo/bin/rustup toolchain install 1.98.0 --profile minimal --component clippy --no-self-update
# rustup installs NO cargo/clippy proxies into a redirected CARGO_HOME - call the toolchain's own
# binaries and put its bin dir on PATH so `cargo clippy` finds `cargo-clippy`:
TC=$RUSTUP_HOME/toolchains/1.98.0-x86_64-unknown-linux-gnu
PATH=$TC/bin:$PATH "$TC/bin/cargo" clippy --all-targets -- -D warnings
```

Use a separate `CARGO_TARGET_DIR`: sharing `loom/target/` between two toolchains invalidates every
artifact on each switch.

**The annotations workaround does not help for this failure mode.** The entry above recommends
`gh api .../check-runs/<id>/annotations` when `--log-failed` returns 403. For a Clippy failure the
only annotation is `Process completed with exit code 101` — no lint name, no file. Reproducing
locally against the CI toolchain is the only route to the actual diagnosis.

**Fix applied:** `backtick_spans` now uses `as_chunks::<2>().0.iter()` with a `&[open, close]`
pattern. `as_chunks` is stable since 1.88 and discards a trailing odd element exactly as
`chunks_exact` did, so the unpaired-backtick behaviour is unchanged. There is no `rust-toolchain.toml`
in this repo — pinning one would trade these surprise breakages for silently ageing lint coverage,
and that trade was deliberately rejected.

## `install.sh` Aborts With No Controlling TTY, After All Real Work Already Succeeded (2026-08-30)

**What happened:** `install.sh`'s `cleanup_backups()` does `read -r response </dev/tty`
unconditionally. In a sandbox or CI with no controlling TTY this aborts the WHOLE script (`set
-e`) with a raw `No such device or address` error — even though every real installation step
(skills, agents, hooks, `CLAUDE.md`, commands) had already completed successfully by that point.
Pre-existing; not introduced by any specific stage.

**Prevention:** to test `install.sh` non-interactively in a throwaway `HOME`, override
`HOME=$TMPDIR/...` (`CLAUDE_DIR=$HOME/.claude` is computed at runtime from `HOME`, never
hardcoded) and pipe `y` to stdin for `confirm_overwrites` (`install.sh:486`, which reads plain
stdin, not the tty) — but the run will still abort at `cleanup_backups()`'s tty read at the very
end unless a tty is attached, so treat a `No such device or address` failure AFTER the install
steps' own success output as a harness artifact, not evidence the install failed.

## Sandbox-Sensitive Tests Carried a Skip List Into Every Plan Stage (2026-09-02)

**What happened:** 22 tests cannot pass inside a Claude Code session sandbox for environmental reasons — 14 `hooks_*` integration tests whose deny branch needs `is_ancestor` to walk the process tree via `ps`, two `daemon::rpc` tests that bind an AF_UNIX socket, three that read process information, and two `fs::permissions` tests that need `dirs::home_dir()` to resolve. A plan carried `cargo test --all-targets -- --skip <22 names>` on every stage's acceptance criteria. Every stage still disputed the skip list, so each dispute cost a judge round and a full suite run, and the judges kept re-granting the same 22 names.

**Why:** the tests asserted an outcome the sandbox could not produce and had no way to say so. The skip list lived in the plan text instead of in the tests themselves, so nothing in the test run distinguished "environment cannot support this" from "this failed."

**Prevention:** a test that depends on a sandbox-denied capability probes for it first and skips loudly (`SKIP <test>: <why>`), the way `tests/e2e/tmux_backend.rs::skip_unless_tmux_can_bind` already did. A plan should never carry `--skip <name list>` in its acceptance criteria.

**Fix:** `src/process/sandbox_probe.rs` (`process_tree_visible`, `unix_socket_bindable`, `path_writable`, `home_dir_resolvable`, `skip_unless`; `LOOM_TEST_REQUIRE_SANDBOX_FREE=1` turns a skip into a failure) guards 21 of the 22 tests. The remaining one, `commands::attach::wait::tests::diagnose_sessions_names_the_work_dir_and_every_session`, passes in the sandbox and shows no environmental dependency, so it was left unguarded.

## The Same Suite Ran Once Per Stage, Per Check, Per Judge (2026-09-02)

**What happened:** five stages of one plan each carried the unfiltered `cargo test --all-targets` gate as an acceptance criterion. Each copy ran once in the agent's own `loom check`, again in `loom stage complete`, and again for every adjudication of that criterion; integration-verify then ran the whole suite once more.

**Why:** the plan proved the entire repository at every stage instead of proving each stage's own code, and nothing remembered a pass already recorded against an unchanged tree.

**Prevention:** run the full suite once, in integration-verify. A standard stage's acceptance criterion should be `cargo test --lib <module>::`, `cargo test --test <target>`, or an equivalent name filter. `loom plan verify` now warns on a full-suite run outside integration-verify (`plan/schema/validation_suite.rs::is_full_suite_run`), and the plan-writer skill states the rule as item 6 of its acceptance checklist.

**Fix:** `verify/criteria/cache.rs` caches criterion passes under `<work_dir>/acceptance-cache/<sha256>.json`, keyed by the criterion text, the acceptance directory, `git rev-parse HEAD`, the raw `git status --porcelain=v2 --untracked-files=all -z` output, and the content hash of every listed path. Failures are never cached. A command mentioning `$HOME`, `~/`, `mktemp`, or `LOOM_HOME` is never cached. `loom check --no-cache`, `loom stage complete --no-cache`, or `LOOM_ACCEPTANCE_CACHE=0` bypass the cache; a cached pass prints as `✓ passed (cached)`. A command that references any git-ignored path (a built binary under target/, for instance) is never cached, because the digest covers the tracked tree only; cargo test and cargo build stay cacheable since they rebuild from that tree.

## Every Stage Worktree Compiled Its Dependencies From Scratch (2026-09-02)

**What happened:** each stage worktree has its own `target/` directory, so every stage spent minutes recompiling the same dependency crates before its first test ran.

**Why:** a shared `CARGO_TARGET_DIR` across worktrees is unsafe here — parallel stages would overwrite each other's `debug/loom`, which acceptance criteria invoke by relative path — and nothing else shared compiled output between worktrees.

**Prevention:** share rustc output through `sccache`, which caches per input hash and leaves every worktree its own `target/` untouched.

**Fix:** `orchestrator/terminal/native/build_cache.rs` locates `sccache` (`LOOM_SCCACHE=0` disables it, `LOOM_SCCACHE=<path>` pins it, otherwise `which` then `~/.cargo/bin`, `~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin`). The session wrapper exports `RUSTC_WRAPPER=<path>` for every session kind when found, and forwards an operator's own `RUSTC_WRAPPER`, `SCCACHE_DIR`, `SCCACHE_CACHE_SIZE`; the confined acceptance environment allows the same three. `loom run` and `loom doctor` print one line stating whether sccache was found.

**2026-09-04 correction:** sccache IS installed on this machine (0.7.7 at `/usr/bin/sccache`) and IS exported into every session, but it fails closed inside the stage sandbox — see "The Sandbox's AF_UNIX Denial Also Kills sccache" in [sandbox-and-settings.md](sandbox-and-settings.md) for the root cause and the `env -u RUSTC_WRAPPER` / `LOOM_SCCACHE=0` workarounds. The prior "not installed" note was wrong and led two separate stages to misattribute the same failure.

## The Ledger Is Exact in Both Directions and Measured After rustfmt (2026-09-02)

**What happened:** two subagents packed struct fields onto one line to hold a pinned maintainability-ledger count. `cargo fmt` re-expanded the lines on the next commit, and six ledger entries reported growth.

**Why:** the ledger records exact line counts, and `cargo fmt` runs before the gate measures them. A count that only holds under un-formatted source is not the count the gate will see.

**Prevention:** `maintainability-baseline.txt` entries are exact in both directions — a shrink must be written back to the ledger, not left as a stale higher number, and growth is never recordable no matter how it was produced. Run `cargo fmt` before measuring a function or file for the ledger; packing arguments or fields onto one line to dodge a count does not survive the formatter.

**Fix:** re-ran `cargo fmt`, remeasured the six affected entries, and wrote back their post-format line counts.

## A Non-Serial Test Read an Env Var a `#[serial]` Sibling Mutates (2026-09-03)

**What happened:** `verify::criteria::tests::runner_tests::test_run_acceptance_caches_pass_and_skips_second_execution` failed the pre-push gate on one machine at `assertion failed: second.results()[0].cached`, and passed on the same tree in another environment.

**Why:** `cache_tests::cache_policy_bypass_from_env` sets `LOOM_ACCEPTANCE_CACHE=0` process-wide for its duration under `#[serial]`. The runner test was not `#[serial]`, so nothing kept the two apart, and it read the ambient value via `CriteriaConfig::default()` and `CachePolicy::from_env()`. Whether the two overlap depends on core count and scheduling, so the failure reproduces on one machine and never shows on another.

**Prevention:** a test whose subject reads the process environment must either pin the value through the config surface (`with_cache_policy`) or be `#[serial]` alongside every test that mutates that variable. `#[serial]` only serialises against other `#[serial]` tests; it does nothing for a non-serial reader.

**Fix:** the runner test pins `CachePolicy::Use` (`verify/criteria/tests/runner_tests.rs`), matching its bypass sibling, so it no longer reads the environment at all.

## ETXTBSY Is a Fork/Exec Race Under Concurrent Tests, Not a Permissions Bug (2026-09-04)

**What happened:** three independent test failures across two stages, all `Os { code: 26,
kind: ExecutableFileBusy, message: "Text file busy" }`, all only under concurrent/
`--all-targets` runs and never when the failing test ran alone:
`orchestrator::terminal::native::wrapper::tests` (two exec sites, ~18% of 17 runs), and a
hand-rolled subprocess test fixture in `quota/codex.rs`'s `poll_once` tests spawning a
freshly-written+chmod'd script.

**Why:** the classic Linux ETXTBSY fork/exec race — the kernel refuses `exec` while ANY
process holds a write fd on that inode. In a multi-thousand-test multi-threaded binary,
another thread's `fork`+`exec` of a just-written script can race a thread still holding the
file open for write; the failure rate scales with concurrency.

**Detection:** a test that passes alone and fails only under `--all-targets`, with error code
26 naming the just-written executable, is this race — never a chmod/permissions problem, and
never specific to one test's script (it hit two independent exec sites in different modules).

**Prevention:** wrap `Command::spawn` in a bounded retry (5 attempts, ~20ms sleep) on
`raw_os_error() == Some(libc::ETXTBSY)` — keep the retry in PRODUCTION code too if a real
external tool self-updating mid-spawn is the same failure mode (`spawn_retrying_text_busy`).
Verify a flake fix by REPETITION, not one green run: 0 failures in 11 full-suite runs after
the fix, against ~18% before, is the only way to know it held — a single green gate run
proves nothing about a flake.

## A Test That "Kills" a Peer by Dropping an fd Is Racy Under Concurrent Process Spawns (2026-09-04)

**What happened:** `daemon::server::broadcast::tests::a_dead_peer_is_evicted_while_a_live_one
_is_kept` failed 2 of 10 full runs: the write to the supposedly-dead peer SUCCEEDED.

**Why:** the test simulated a closed peer with `drop(dead_reader)`, but closing an fd only
releases ONE reference to the socket. A concurrent `std::process::Command` fork in another
test thread can inherit a duplicate of that fd and keep the socket alive until the child
reaches its own exec — so `write_message()` on the "dead" peer doesn't return `EPIPE`. The
production code was never wrong.

**Prevention:** in a test binary that also spawns processes, any test simulating a closed
peer by dropping an fd is racy. Assert on socket state instead: `dead_reader.shutdown
(Shutdown::Both)` before the drop marks the SOCKET itself dead, which no forked fd copy can
undo.

## An Acceptance Criterion That Greps a Colorized Tool Summary Fails Only Inside the Confined Runner (2026-09-04)

**What happened:** `cd web && bunx vitest run ... | rg -q "Tests +[1-9]"`-shaped criteria, and
a criterion grepping cargo's own summary line, pass under an interactive shell and fail when
run through the stage's own confined completion check — twice, on two different tools.

**Why:** the confined acceptance environment (`process/environment.rs`
`STAGE_HOST_ENV_ALLOWLIST`) does not include `NO_COLOR`, and both `vitest` and `cargo`
colorize their summary line whenever `NO_COLOR` is unset — even writing to a file, not a TTY.
The ANSI escapes land BETWEEN the label and the digits (`Tests \e[22m\e[1m\e[32m3 passed`),
so a regex requiring a space immediately before the digit cannot match, even though the
underlying test run is fully green.

**Detection:** a criterion whose command succeeds but whose `rg -q` fails is almost always a
formatting difference — reproduce with `env -i HOME PATH TMPDIR <criterion>` (or the exact
`STAGE_HOST_ENV_ALLOWLIST` set) before assuming a code defect, and pipe through `cat -v` to
see the escapes a terminal hides.

**Prevention for plan authors:** never grep a human-readable summary line for a count. Set
`NO_COLOR=1` in the criterion, or assert against a JSON/basic reporter instead.

## A Backgrounded `cat` Never Drains a Fake Subprocess's stdin (2026-09-05)

**What happened:** two subprocess-test gotchas in `quota/codex.rs`'s `poll_once` tests — and the
first recorded prevention for one of them was itself wrong, which is how the flake reached CI.

1. A fake script that never reads stdin and exits immediately races the parent's writes: if the
   script exits first the parent gets `Broken pipe (os error 32)` rather than a clean write.
   The prevention recorded here on 2026-09-04 — "background a stdin drain (`cat >/dev/null &`)"
   — **does not drain anything**. POSIX assigns `/dev/null` to the standard input of an
   asynchronous list in a shell without job control, before any explicit redirection, and
   `/bin/sh` is `dash` on Ubuntu CI. The backgrounded `cat` reads `/dev/null`, exits at once,
   and the script exits behind it. All it bought was the fork+exec delay, which hid the race
   locally and left it live in CI: `the_child_exiting_without_ever_replying_is_reported_precisely`
   failed 0/40 unloaded runs but 2/30 under 32 busy loops, asserting
   `"failed to write to codex app-server stdin"` against
   `"codex app-server closed without replying"`.
2. Teardown always calls `child.wait_timeout(Duration::from_secs(2))` before killing, on every
   exit path including shutdown; against a script that ignores stdin closing (e.g. `sleep 30`),
   this adds a full ~2s to the test's elapsed time even after the reply-wait loop gave up early.

**Why:** a fixture cannot paper over a production defect. `poll_once` treated any write failure
as fatal, so a child that died before reading its request was reported as a loom-side write
error instead of by what it printed — the same misreport a real `codex app-server` crashing on
startup would produce. Every attempt to keep a reader alive in the fixture was working around
that, and the cheapest-looking workaround happened not to work at all.

**Prevention:** fix the code, not the fixture. A `BrokenPipe` on a request write to a child is
not an outcome worth reporting: the child's stdout (a reply, a JSON-RPC error, or EOF) is.
`poll_once` now reports only write errors whose `ErrorKind` is not `BrokenPipe` and otherwise
falls through to `await_reply`, so both orderings of the race produce the same verdict.
If a fixture genuinely must hold the read end open, the shell must save the descriptor before
backgrounding — `exec 3<&0; cat <&3 >/dev/null &` — or stay alive itself (`sleep 30`).
`cat <&0 >/dev/null &` fails too: fd 0 is already `/dev/null` by the time the duplication runs.
Check any such claim with `printf 'x\n' | sh -c 'cat > out & wait'`; an empty `out` means the
drain never ran. And any test asserting a tight "returns within Xs" bound on code with an
unconditional teardown grace window must budget that grace on top of the deadline/shutdown
latency.

**Fix:** `loom/src/quota/codex.rs` — `write_requests` returns `std::io::Result<()>` and
`poll_once` matches `Err(e) if e.kind() != ErrorKind::BrokenPipe` for the only fatal case; the
five dead `cat >/dev/null &` drains are gone from `codex_tests.rs`. A race that only fires under
load needs a loaded runner to catch: `scripts/flake-check.sh` re-runs `quota::`, `process::`,
`verdict_apply_tests::` and `stalled_judge_tests::` under CPU contention, pinned to 4 CPUs where
`taskset` exists, wired into CI, the release workflow's publish gate, and the pre-push hook.

## The Pre-Commit Hook Re-Adds Every Staged File, So Partial Staging Is Silently Undone (2026-09-06)

**What happened:** a commit was meant to carry only this session's hunks of `loom/maintainability-baseline.txt` and `loom/src/models/stage/methods.rs`, staged with `git apply --cached`, while another session's uncommitted edits to the same two files stayed in the working tree. The commit that landed contained the other session's ledger entries too (`methods.rs 739`, `types.rs 1166`, a `defaults.rs` entry) with none of the code those entries describe, so the snapshot could not pass its own ledger gate.

**Why:** `loom/.githooks/pre-commit` records `git diff --cached --name-only` before running `cargo fmt` and markdownlint, then runs `git add "$file"` for every one of those paths unconditionally, not only for files the formatters changed. Any path with a partially staged diff is replaced by its full working-tree content. Nothing in the hook output says so; the line reads `Re-staging formatted files...`.

**Prevention:** when a file you need to commit also carries someone else's uncommitted edits, do not rely on partial staging through the hook. Either land the other session's work first, or commit with `--no-verify` after running the hook's own checks by hand (`cargo fmt --check`, `cargo test --test maintainability`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`, markdownlint on the touched files) and say so in the report. After any commit that involved partial staging, diff the committed file against the previous commit before moving on; `git show --stat` alone will not reveal the substitution. The hook itself could re-add only files whose content the formatters changed (`git diff --quiet -- "$file"` before and after), which would remove the trap.

**Fix:** the affected commit was amended with the intended ledger content, and the follow-up commit was made with `--no-verify` after the hook's checks were run by hand.

## A Success-Path Deadline Tighter Than Production Is a Flake (2026-09-06)

**What happened:** `quota::codex::tests::garbage_and_an_over_long_line_are_skipped_before_the_reply` failed the pre-push flake check with `codex app-server timed out`: the fake child had not written its reply within the 5 s the test gave `poll_once`. The `quota::` run took 8.30 s against a normal 3 s. The script itself takes under 0.1 s even under the check's pinned load, and 60 loaded re-runs (the default load, under a pty, and with 32 spinners) never reproduced it, so something on the runner held the child for more than 5 s.

**Why:** the five reply-expecting tests passed a 5 s deadline for no reason. Production gives the exchange 15 s (`CODEX_DEADLINE`), and on the success path the deadline never fires; it only caps a broken exchange, so any value shorter than production's trades nothing for a flake the moment the runner stalls the child. The three timing tests carried the same shape, about 1 s of margin between the expected elapsed time and the assert bound.

**Prevention:** in a subprocess test, a deadline the success path never reaches is not a timing assertion, so give it a generous value (`REPLY_DEADLINE`, 60 s). An elapsed-time bound proves one thing, that the code did not wait out the child's `sleep 30` or the deadline; set it well under that escape and well over the expected time, and say in the comment what it rules out. Build an over-long fixture line with a printf field width (`printf '%70000s\n' ''`) under `#!/bin/sh`; a 70000-element brace expansion needs bash and buys nothing.

**Fix:** `loom/src/quota/codex_tests.rs`: `REPLY_DEADLINE` replaces the five 5 s deadlines, the elapsed bounds go from 4/5/3 s to 15/20/15 s with comments naming what each rules out, and the over-long-line fixture is plain sh.

## One Panic Between set_current_dir and Its Restore Fails Sixty Unrelated Tests (2026-09-06)

**What happened:** a full `cargo test --all-targets` reported 71 failures across memory, stage, stop, map and merge-lifecycle tests, all `Failed to get current dir: NotFound`. Only one test had a real defect: it asserted an `INDEX.md` row shape that had changed, panicked after `std::env::set_current_dir(&test_dir)` and before the restore, and its temp dir was then dropped — leaving the whole test process with a deleted cwd for every later test.

**Why:** the cwd is process-global; a test that changes it and panics before restoring never runs the restore line, and `TempDir`'s drop removes the directory the process is still standing in. `#[serial]` does not help — it only orders the tests, it cannot restore the cwd.

**Prevention:** when many unrelated tests fail with `current_dir` NotFound, run `cargo test --lib -- --test-threads=1` and fix the FIRST failure only; the rest are the cascade. Run a suspect module in isolation (`cargo test --lib <module>`) to separate a real failure from contamination. A new cwd-changing test should restore via a guard type (Drop) rather than a trailing statement.

**Fix:** corrected the one assertion; the other 70 passed untouched.

## Using npx Instead of bunx

**Mistake:** Used npx instead of bunx during implementation.
**Fix:** Always use `bun`/`bunx` per project conventions. Check CLAUDE.md tool preferences before running package managers.

## toml_edit vs toml: Different Use Cases

**Mistake:** Using `toml_edit Item -> serde` for reading nested config sections. `toml_edit` is designed for round-trip writes; its typed access silently drops nested sub-tables.

**Why:** `toml_edit::Item` doesn't implement full `serde::Deserialize` for complex nested structures the same way `toml::Value` does.

**Prevention:** Use `toml_edit` for writes (round-trip safe). Use `toml` (re-parse the full file with `toml::Value`, then `try_into::<T>()` on the section) for typed reads of nested structures.

## CI Clippy Failures That Don't Reproduce Locally = Toolchain Drift (2026-07-22)

**What happened:** CI's Clippy job failed on main while `cargo clippy --all-targets -- -D warnings` passed locally with zero warnings. Local toolchain was 1.95.0; CI installs latest stable via `dtolnay/rust-toolchain@stable`, which had moved to 1.97.1 and shipped new lints (`useless_borrows_in_formatting`, broader `question_mark`) that fired on 21 existing sites.
**Why:** The workflow floats on `@stable` while local toolchains only move on explicit `rustup update`. Every ~6-week Rust release can introduce lints that break CI with `-D warnings` even though no code changed.
**Prevention:** When a CI clippy failure doesn't reproduce locally, check `rustup check` FIRST — if stable has moved, `rustup update stable` and re-run before hunting for any other cause. Most new-lint fallout is machine-applicable: `cargo clippy --fix --all-targets --allow-dirty`, then review the diff (non-trivial rewrites like `question_mark` can leave awkward leftover blocks worth hand-cleaning).
**Fix:** Updated local stable to 1.97.1, applied `cargo clippy --fix`, hand-simplified the `?`-operator rewrite in `fs/work_dir.rs`, verified clippy + fmt + full test suite green.

## Bash Tool CWD Persists — Never Bare-`cd` Into a Subdirectory Crate (2026-08-08)

**Two stages of one plan hit this.** The Bash tool's working directory persists across calls, and this
repo's crate root is the `loom/` subdirectory — so a single `cd loom` silently retargets every later
relative path, **including calls issued in the same parallel message block**.

**Why it keeps recurring:** the failure presents as `No such file or directory`, i.e. as a _missing
file_, not as a wrong-directory error. It reads as "that file doesn't exist" and sends you looking for
the wrong bug. The other tell is `git status` printing `loom/src/...` prefixes instead of `src/...`.

**Prevention:** never bare-`cd`. Prefix every command with its own `cd <dir> &&` so each call is
self-contained and order-independent.

## `libc::mode_t` Width Differs by Platform — `.into()` Is a Clippy Error on Linux (2026-08-10)

`libc::mode_t` is `u32` on Linux and `u16` on macOS, while the std APIs we call
(`DirBuilderExt::mode`, `PermissionsExt::from_mode`, our own `safe_fs::open_safely`) all take `u32`
unconditionally. So `const MODE: libc::mode_t = 0o700; builder.mode(MODE.into())` compiles on macOS
and fails on Linux with `useless_conversion`, which `-D warnings` promotes to an error — it blocked
`git push` (`loom/src/daemon/server/storage.rs:14`). The same trap applies to any
platform-width alias: `c_int`, `off_t`, `nlink_t`, `time_t`.

**Prevention:** declare permission constants as `u32` (the type every Rust-side API wants) and cast
at the raw-libc boundary only: `libc::fchmod(fd, MODE as libc::mode_t)`. A cast to an alias is exempt
from `unnecessary_cast`, so it is lint-clean on both platforms, whereas `.into()`/`u32::from()` is
lint-clean on exactly one.

**Also:** the pre-push hook runs Clippy, the pre-commit hook does not. A lint-broken commit lands
locally and only surfaces at push time. Run `cargo clippy --all-targets -- -D warnings` before
committing, not after.

## A Test Substring That Crosses a `colored` Segment Boundary Passes Under a Pipe and Fails in the Pre-Push Hook

**What happened:** `cleanup_warning_renders_as_a_single_line_with_a_stage_file_hint` in
`loom/src/commands/status/render/attention_tests.rs` passed under plain `cargo test` and in CI but
failed inside the pre-push hook, blocking `git push`. Its assertion looked for `"Cleanup warning:
failed: worktree busy\nretrying next cycle"`, a substring that starts in the plain label and
continues into the `warning.yellow()` argument.

**Why:** the `colored` crate emits ANSI escapes whenever stdout is a terminal and
`CLICOLOR`/`NO_COLOR` are unset. Git runs hook stdout on the terminal, so `cargo test` inside
`loom/.githooks/pre-push` renders with colors and an escape sequence lands between the label and the
text. Under a pipe, a sandbox, or CI the same test sees no escapes and passes, so the failure
surfaces only at push time.

**Prevention:** in a test that renders through `colored`, assert on substrings that lie wholly inside
one colored segment or wholly in plain text, never across the boundary between a label and a
`.yellow()`/`.dimmed()`/etc. argument. To reproduce the hook's environment locally, run
`CLICOLOR_FORCE=1 cargo test --lib <test-name>`. `loom/src/commands/graph/tests.rs` has a private
`strip_ansi` helper if a test genuinely must match across segments.

**Fix:** split the assertion into `contains("Cleanup warning: ")` and `contains("failed: worktree
busy\nretrying next cycle")`.

## A cargo test Filter Written Against the Test File's Own Name Selects Zero Tests, Silently (2026-09-10)

The repo convention `#[path = "tests_<name>.rs"] mod tests;` (`commands/knowledge/{sync,check,eval,context,telemetry}.rs`,
`commands/hook/target.rs`, `commands/map.rs`) keeps every test compiled under the OWNING
module's path (e.g. `commands::knowledge::sync::tests::...`), not under a module named after
the file. A narrow `cargo test` filter written against the filename
(`commands::knowledge::tests_eval`, `tests_context`, `tests_sync`) matches nothing, selects
zero tests, and prints `ok` with `0 passed` — indistinguishable at a glance from a real,
narrow, all-green run. A plan brief that quotes such a filter as a "Proof" check proves
nothing.

Prevention: before quoting a narrow test filter in a brief or acceptance criterion, run
`cargo test --lib -- --list | rg <filter>` and confirm a non-zero match count.

## `cargo audit` Git-Fetches Its Database First — It Cannot Pass in a No-Network Stage Sandbox (2026-09-12)

**What happened:** a plan copied `cargo audit -f loom/Cargo.lock -d loom/target/advisory-db`
verbatim from a sibling plan's pre-push gate into a stage whose sandbox declares "No
network access." `cargo-audit` git-fetches the RustSec advisory database into that path
before scanning anything; `loom/target/advisory-db` does not exist in a fresh worktree,
so the fetch fails ("couldn't fetch advisory database: git operation failed") before the
audit itself ever runs — the criterion fails regardless of whether the dependency tree
has any advisories.

**Why:** the criterion was copied from a plan whose stage DOES have network access,
without checking the destination stage's own network policy.

**Prevention:** a plan step that runs `cargo audit` inside a no-network stage must use
`cargo audit --no-fetch -d "$HOME/.cargo/advisory-db"` against a database path already
present on the machine (populated by an earlier `cargo audit` run outside a stage
sandbox), never a fresh in-worktree path the offline run cannot populate itself. When
copying a gate list between plans, re-check each criterion against the destination
stage's own sandbox network policy — a criterion that passed in the source plan is not
evidence it will pass in the copy.
