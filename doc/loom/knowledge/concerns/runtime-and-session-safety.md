# Runtime And Session Safety

> Runtime edge cases: tmux warning, attach lifetime, orphan adoption, guards

## `evaluate_new_session` Fails a Working Spawn on a Benign `~/.tmux.conf` Warning (2026-08-08)

The rule "any stderr with exit 0 is a failure" is a plan mandate, pinned by unit tests and carried by
an explicit stage decision — so it was **deliberately left as-is**. But tmux prints `~/.tmux.conf`
deprecation warnings to stderr _while creating the session fine_, so one benign warning now: fails the
spawn, kills a **working** server via the abort path, and writes the sticky
`.work/terminal-backend-fallback` marker — disabling tmux for the whole repo until someone runs
`loom run --backend tmux`.

The `has-session` probe that immediately follows is the authoritative signal and would distinguish the
two cases. Gating the stderr rule on that probe is a design call for the plan owner, not an
integration-verify defect.

## `loom attach` Overview Panes Are Live, Writable Agent Terminals (2026-08-08)

The overview's panes are full interactive attach clients (no `-r`), so a stray keystroke goes into a
live autonomous agent's input, and `C-b x` kills the **stage's** pane — the inner servers have no
`remain-on-exit`, only the viewer window does.

Left as-is deliberately: the plan mandates the exact pane string. If the overview is meant to be a
_viewer_ rather than N live terminals, adding `-r` to `attach-session` on the **overview path only**
(never on `loom attach <stage-id>`, which is the intentionally interactive path) is a one-word change.

## Nothing Reaps the Overview Viewer Socket (2026-08-08)

`list_loom_sockets` skips `loom-view-<8hex>` by name so `clean`/`init` do not report it as
unattributable, and the skip comment concedes "Nothing currently reaps this socket". After
`loom attach` the viewer server and its nested clients outlive the operator's detach, and
`loom init --clean` now kills every attributed _session_ server while stranding the viewer.

It is the one socket whose name is a pure function of the repo root, so reaping it carries no
cross-checkout risk — roughly two lines in `cleanup_orphaned_sessions` and `clean::sessions` once
`viewer_socket_name` is crate-visible.

## PreToolUse File Guards Cannot Eliminate Path-Swap Races (2026-08-08)

The canonical `worktree-file-guard.sh` now covers Read, Write, Edit, Glob, and Grep; canonicalizes
paths; compares path components; and rejects both leaf and parent symlinks. This closes the concrete
absolute-path, common-prefix, and final-symlink escapes found in the security review.

The guard is still a check before a separate built-in tool performs the real open. A hostile process
can replace a parent path after the hook returns, so the guard cannot bind its decision to the inode
the tool later uses. Treat it as defense in depth; the host OS sandbox remains the authoritative
boundary. A race-free design requires a dedicated file-operation broker that performs traversal and
the actual open relative to an already-open worktree directory using no-follow semantics. Do not
describe the current hook boundary as TOCTOU-free.

## `loom attach <stage-id>` Dies With Its Stage (2026-08-26)

Investigated after a report that the overview never removed dead panes or added new stages. The
report was a ghost — re-tested the same day, the reconciler both adds and kills correctly on tmux
3.6a. The defects the investigation surfaced (debug-only/absent reconciler logging, relative daemon
`work_dir`, `TMUX_TMPDIR` divergence, no retile after kills, no real-tmux test, the never-compiled
`reconcile/steps.rs` copy) were fixed the same day — see `architecture/terminal-backends.md`
§ "Live Overview Reconciliation". What remains is a design property, not a bug:

**Direct attach dies with its stage by design.** `loom attach <stage-id>` `exec`s into the stage's
OWN server (`commands/attach/mod.rs`), whose lifetime is the stage's (default `exit-empty`;
`completion_handler.rs` `kill_session` is only a backstop). The client gets `[exited]` on
completion; no follow/loop exists. Candidate design if this matters: make direct attach a
`select-pane` + `resize-pane -Z` focus on the long-lived overview viewer, so the operator is attached
to the viewer's lifetime, not the stage's (note `split-window` unzooms, and the build should reuse a
healthy viewer instead of `kill-session`-ing it). Daemon log for any attach/reconcile question:
`.work/orchestrator.log`, level fixed by `RUST_LOG` in the shell BEFORE `loom run`.

## Orphan Adoption Only Runs at Daemon Startup (2026-08-29)

`adopt_orphaned_agents` is called from `recover_orphaned_sessions`, which
`orchestrator.rs` invokes exactly ONCE, at daemon startup — not on the poll loop, despite the
"every tick" phrasing that appeared in the brief that requested it. An agent orphaned mid-run
therefore stays invisible until the next `loom run`. That is enough for the incident it was written
for (a killed daemon is restarted by definition), and the spawn-time guard in `start_stage` closes
the duplicate-spawn hole independently, so this is a narrower reach rather than a hole. Making it
per-tick is a one-line addition to the scheduler loop; the pass is already idempotent and pinned as
such by a test, so the only question is cost — it scans `.work/pids/` per Executing stage.

## `get_work_dir()` Trusts Any Path Containing `.worktrees/` (2026-08-29)

`find_worktree_root_from_cwd` (`git/worktree/paths.rs`) is a pure substring match on `.worktrees/`
in the cwd string — it never checks that the directory is a loom-managed worktree. `get_work_dir`'s
first branch then adopts `<that root>/.work` if one merely exists. A user working in any directory
they happen to name `.worktrees/<anything>` that contains a leftover `.work` would silently read
another project's memory and stage state.

Lower severity than the creation-path bug fixed alongside it (`mistakes/ambient-filesystem-trust.md`):
both of `get_work_dir`'s branches only ever RETURN a `.work` that already exists, so this
misattributes reads rather than manufacturing stray directories. Left alone deliberately, because
changing it would alter the reuse and read-only degrade paths that `loom memory list` depends on
during post-compaction recovery (Rule 3b). Fix shape if it is ever worth doing: confirm the
candidate root is a real worktree — a `.git` FILE containing a `gitdir:` pointer — before trusting
its `.work`.

## `commands::memory` Tests Mutate the Process-Global Working Directory (2026-08-29)

Those tests call `env::set_current_dir` and rely on `#[serial]`, which only serializes a test
against OTHER `#[serial]` tests — anything unmarked in the same binary runs concurrently with a
foreign cwd in effect. In each test the `TempDir` is also declared AFTER its `EnvGuard`, so it is
deleted FIRST and there is a window where the process cwd points at a removed directory.

Neither of these caused the 77-failure incident (an ambient impostor `.git` did, see above), which
is why they were not changed. They remain a live hazard: any future test in that binary that
resolves a relative path while a sibling holds a foreign cwd will fail in a way that looks
unrelated to its own subject. Fix shape: inject the working directory instead of mutating the
process's, as `orchestrator/merge_lifecycle/tests.rs:321` already does deliberately for this exact
reason.

## Guard Hooks: Four Design Questions Deliberately Left Open (2026-08-30)

The adversarial review of `read-guard.sh`/`poll-guard.sh`/`_read_discipline.sh` surfaced four
behaviours kept AS SPECIFIED rather than patched, because each needs a spec decision, not a
one-line fix:

1. **The deny threshold may not fit CLAUDE.md's own workflow.** Five identical `git status`
   invocations in one session trigger a deny, yet CLAUDE.md's own commit workflow runs `git
   status` before staging, again after committing, and again before completion — a real session
   following the documented workflow can plausibly hit the threshold.
2. **Ledger TSV keys can interleave across processes.** `_loom_ledger_append` uses the whole
   normalised command line as the TSV key with no atomicity guarantee beyond `PIPE_BUF`; a key
   longer than `PIPE_BUF` can interleave between two in-process subagents that both sanitise to
   `agent_id=main`.
3. **`loom_deny_enabled` is a line-oriented `grep`-style check**, so a TOML multi-line string
   VALUE that happens to contain the literal lines `[hooks]` and `deny_enabled = true` would
   enable the switch even though no real config intended it.
4. **`poll-guard`'s rule-2 `cat` branch is unreachable for any pre-existing `.work` file**,
   because rule 3 (repeat-read escalation) fires first for files the ledger already has an entry
   for — effectively dead code on the common path.

None of these are fixed; each is a live behaviour a future stage should either ratify explicitly
or change with an accompanying spec decision, not patch as an incidental side effect of unrelated
hook work.

## `is_ancestor("1")` Cannot Distinguish "Not an Ancestor" From "Walked Off the Top of a Container"

`hooks/_common.sh`'s `is_ancestor()` exits its walk-up-the-process-tree loop as soon as the
current pid becomes `"1"` or `"0"`, WITHOUT checking whether that final value equals the target
pid — so `is_ancestor(target="1")` is a guaranteed, deterministic `false` regardless of the real
process tree, even inside a container where PID 1 genuinely is an ancestor of everything. This is
useful as a test fixture (a non-ancestor `LOOM_MAIN_AGENT_PID` of `"1"` can never flake true), but
it is also a real edge-case correctness gap for any deployment where the loom main agent's PID
could legitimately be 1. Not fixed — recorded because the test-fixture use depends on the same
behaviour that makes it a latent bug elsewhere.
