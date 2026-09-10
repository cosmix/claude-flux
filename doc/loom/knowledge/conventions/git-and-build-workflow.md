---
---
# Git And Build Workflow

> Git/worktree ops, cargo fmt/test discipline, the shared maintainability ledger

## Git Operations

```bash
git worktree add .worktrees/{stage-id} -b loom/{stage-id}
git worktree remove --force .worktrees/{stage-id}
git merge --no-ff -m "Merge loom/{stage-id}" loom/{stage-id}
git branch -D loom/{stage-id}   # Delete after merge
```

**Active-merge guard rule (2026-04-27):** Helpers that mutate git merge state (`merge_stage`, `get_conflicting_files_from_status`) MUST refuse via `require_no_active_merge` when `MERGE_HEAD` is set on the repo path. Never silently `git merge --abort`. Defense in depth: even if attribution misses an active merge upstream, the guard surfaces an error instead of corrupting in-progress resolution.

**Phantom-merge revert logging (2026-04-27):** All phantom-merge reverts (sync-time merged=true revert, daemon `reconcile_main_repo_active_merge`, CLI `RevertAndSpawnResolver`) MUST log at `tracing::error!` level — not `warn` — so they show up in production logs. Reverts represent invariants violated; the noise is the point.

## Formatting and Test Invocation in a Shared Worktree

- **Never run `cargo fmt` while sibling subagents are live.** `cargo fmt -- <path>`
  **IGNORES its path arguments** and formats the ENTIRE crate, silently reformatting files
  another agent owns — which shows up in `git status` as an ownership violation and can
  collide with an in-flight edit. Use `rustfmt --edition 2021 <file>` for your own files.
  Only the main agent runs repo-wide `cargo fmt`, and only after every subagent has landed.
- **`cargo test` accepts exactly ONE testname filter.** Extra filters are rejected with
  "unexpected argument" BEFORE compiling, so zero tests run. Use one common prefix
  (`cargo test --lib context::`) or separate invocations chained with `&&`.
- **Filter by the real module path.** Tests under a `tests` submodule need it spelled out:
  `context::tests::delivery`, not `context::delivery`, which matches nothing.
- **`rustfmt`'s `fn_call_width = 60` is what forces a call vertical, not `max_width = 100`.**
  Sum the argument names including `,` separators; over 60 and rustfmt goes one-arg-per-line,
  which can explode a match arm and trip the 50-line function gate. Renaming in the pattern
  (`budget_tokens: budget`) is a legitimate way back under the limit.
- **Only the main agent runs `cargo` at all**, and it runs under a RAM watchdog rather than
  a job-count throttle. Measured on this 32-core machine, a full `cargo build --all-targets`
  peaks around 3 GB and the whole test suite barely moves the needle — so throttling `-j` is
  the wrong lever and just wastes the machine. What actually exhausted 125 GB was **leaked
  detached child processes**, not build parallelism (see
  [Never Spawn a Surviving Process From a Test](../mistakes/detached-spawn-in-tests.md)).
  Run cargo wrapped in a watchdog that samples `free`, kills the whole **process group** on a
  low-headroom trip (killing cargo alone orphans the `rustc` children holding the pages), and
  reports any `loom` process still alive after exit.
- **A test may never create a process that outlives the test harness.** `cargo test` gives no
  warning for a leaked detached child; it simply exits green while the child keeps running.
  See [Never Spawn a Surviving Process From a Test](../mistakes/detached-spawn-in-tests.md) —
  this cost a reboot once already.

## `cargo test` Is Not This Repo's Test Gate — `--all-targets --no-fail-fast` Is

Never write plain `cargo test` into a loom plan's acceptance criteria. The gate is:

```bash
cargo test --all-targets --no-fail-fast
```

Both flags earn their place:

- **`--all-targets` is what compiles `loom/tests/**`.** Without it the external
  integration tests are never built, so a changed signature breaks them and NOTHING
  reports it until somebody runs the full command by hand. Signature changes are
  exactly what a refactor stage produces, which is where this bites hardest.
- **`--no-fail-fast` is what makes the report exhaustive.** Stopping at the first
  failing target hides how much else is red; an agent then fixes one failure, re-runs,
  and discovers the next — one round trip at a time.

Know the two non-hermetic tests, so a red run inside a stage session is not
misdiagnosed as your own breakage. The stage-finalisation tests
(`commands/stage/tests/complete.rs`) route through `sandbox_control_session`
(`control_session.rs:70,94`), which reads `LOOM_STAGE_ID` / `LOOM_SESSION_ID` /
`LOOM_WORKTREE_PATH` from the ambient process environment. Running the suite from
INSIDE a loom worktree session leaves those set, silently routing the call down the
sandboxed worktree path instead of the host-side one the test means to exercise, and
it fails with a wrapper-identity mismatch. It is also order-dependent: it failed in
one full `--all-targets` run and passed in the next.

Re-run with `env -u LOOM_STAGE_ID -u LOOM_SESSION_ID` BEFORE concluding your change
broke it. The durable fix is an RAII env guard at test start — mirroring `EnvGuard`
in `commands/memory/handlers/tests.rs` — that restores on `Drop`, so a panic mid-test
cannot leak state into later tests. Do not apply that fix from an unrelated stage:
touching a file outside your territory is cross-stage merge-conflict bait.

## The Maintainability Ledger Is Shared State, and Only One Concurrent Stage May Own It

`loom/maintainability-baseline.txt` is an EXACT-match ledger: it fails when the code
SHRINKS as well as when it grows, and a plain `cargo test` runs it. It is also one
file at one path, shared by every worktree in a plan.

Three consequences a plan author has to design around:

1. **Exactly one CONCURRENT stage may own the ledger.** Two parallel stages that both
   grow or delete ledgered code will conflict on merge, and each will have reconciled
   against a baseline the other invalidated.
2. **A plan that grows or deletes ledgered code without owning the ledger cannot pass
   its own acceptance.** Deleting a ledgered function fails exactly like adding an
   over-long one, so a stage that removes ~4000 lines of orphaned surface MUST also
   hold the ledger.
3. **When a refactor drops an entry under the limit, DELETE the entry rather than
   lowering it.** Lowering keeps a permanent claim on a function that no longer needs
   one.

Before adding lines to any function: `rg '<fn name>' loom/maintainability-baseline.txt`.
If it is listed, refactor rather than extend.

## Working Directory

The Bash tool keeps its working directory ACROSS calls in this harness, so one `cd` silently
redirects every later relative command. Prefix verification commands with an explicit absolute
`cd`, or pass absolute paths. **If EVERY independent check fails at once — fmt and clippy and
build and an unrelated gate — suspect the working directory before the code**; a real
regression almost never breaks all of them in the same instant.

## Git Push Requires Explicit User Request

Never `git push` unless the user explicitly asks — commit locally and stop. "Fix the CI failure" does NOT imply pushing to make CI green; the user decides when commits leave the machine. (Learned 2026-07-22: pushed after fixing a red CI run on the theory that CI-green was the deliverable — user rejected: "i didn't ask you to push.")

## Never Read the Daemon Credential Files

`.work/admin.token` and `.work/user.token` (and their `.loom/work/` equivalents) are operator secrets. Never read them, never run a command whose output could include them, and never widen a shell command into a directory sweep. On 2026-09-03 the operator rejected two batched commands as attempts to read the admin token: one combined `rg`/`fd` sweeps over `hooks/tests` and `loom/tests` with a whole-file `rg -n "" hooks/<script>`; the other combined `loom plan verify` with `rg` sweeps over `doc/plans/briefs/<plan>/`. Run `loom plan verify` on its own, or ask the operator to run it with the `!` prefix; search with `rg -n <pattern> <explicit file>` on named files, never on directories that may hold fixtures or state; read hook scripts through narrow `rg -n <pattern> -A <n> hooks/<file>` queries, never a whole-file dump.

`rg` and `fd` skip the token files through the state root's `.ignore`; sessions the daemon spawns additionally get `RIPGREP_CONFIG_PATH`, which backstops `rg` alone, so `rg -uu` sweeps skip them too. The daemon publishes both `.ignore` and `ripgreprc` before either token exists (`daemon/server/tokens.rs::publish_fresh_tokens`), and the wrapper only exports `RIPGREP_CONFIG_PATH` when `ripgreprc` is already on disk, so a `loom run --foreground` session — which has no daemon, no tokens, and no exclusion files — never exports it either.

## A Function Called From `$(...)` Cannot `exit` to Block Its Caller

A shell function invoked inside a command substitution (`X=$(normalize_lexical ...)`) runs in a
subshell — calling `exit` inside it only terminates that subshell, not the calling script, and
relying on `set -e`'s command-substitution-assignment behavior to propagate the failure is fragile
and non-obvious. Convention in this repo's hook scripts: have the function return a plain nonzero
status and let the CALL SITE do `if ! X=$(normalize_lexical ...); then block_target; exit 2; fi` — an
`if`-tested command is explicitly exempt from `set -e`, so this is both correct and matches the rest
of the file's style.
