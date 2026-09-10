# Merge Cleanup Boundary

> A cleanup-boundary bug: what happened, why it survived undetected, and the fix shape worth reusing.

## What Happened

`auto_merge::attempt_auto_merge` ran worktree-and-branch cleanup inside each of its
three success arms. So "merge" destroyed the evidence of the merge before its caller
could verify it — and the caller derives a missing `completed_commit` from exactly
the branch that cleanup had just deleted.

That is the phantom-merge failure class (`mistakes/phantom-merges.md`) reached from a
new direction: not by writing `merged: true` without checking git, but by removing
the ability to check at all. A caller that wanted to verify ancestry had nothing left
to verify against.

## Why It Survived So Long

A function named `attempt_auto_merge` doing cleanup is invisible from its call sites:
every caller reads as "merge the stage", and nothing in the name or signature
suggests the worktree is gone afterwards. Three separate success arms each did it, so
no single arm looked like the odd one out, and the tests passed because cleanup
genuinely succeeded — the defect was in the ORDER, not in any step.

**Detection rule:** when a function's name describes one action, list the
irreversible side effects it performs. If any of them destroys an input another
caller needs, the order is a latent defect regardless of whether tests pass. Ask
specifically: *after this returns, what can no longer be verified?*

## The Fix, and the Shape Worth Copying

`orchestrator/merge_lifecycle.rs` now owns the ordering structurally, and
`attempt_auto_merge` returns an **UNCLEANED** outcome:

```text
overlay reconcile -> merge attempt -> verify merged ancestry
  -> base reconcile & publish -> mark state and release dependents
  -> cleanup worktree/overlay
```

Two design choices carry the lesson:

- **One door.** `MergeLifecycle::cleanup` is the single path by which any merge
  caller may reach `cleanup_after_merge`. Every caller routes through it — the daemon
  auto-merge path, orphan worktree cleanup, manual `--resolved`, retry, and the
  already-merged short circuit — so **no caller can bury cleanup inside "merge"
  again**. Removing a side effect from a function is only durable if the side effect
  gains a single owner; otherwise the next caller re-adds it locally.
- **Cleanup refuses, rather than trusting.** It will not run unless the stage branch
  is provably contained in the target. A destructive step that cannot verify its own
  precondition must decline, not proceed optimistically.

Note what stayed with the callers and why: the merge attempt, the ancestry
verification and the state marking, because those need the orchestrator handle and
the stage-file lock. The lifecycle module owns only what sits AROUND them. Extracting
a lifecycle does not mean absorbing the lock-holding steps into it.

## Two Subsidiary Rules It Encodes

- **Cleanup still defers when the cwd is inside the worktree it would remove.**
  Deleting a live session's working directory breaks every hook that session fires
  afterwards — a class of failure that presents as unrelated hook errors, never as
  "your directory vanished".
- **A derived-state failure must not fail a good merge.** A reconcile failure marks
  derived state stale and leaves the merge intact, matching the existing rule that a
  cleanup failure must not turn a successful merge into an error. Generalisation: the
  durable result and the derived artifact have different failure budgets; never let
  the cheaper one veto the expensive one.

## Cleanup Refused Over Scaffold It Never Planted (2026-08-26)

**What happened:** Every non-forced worktree removal in this repo failed from
2026-08-09 (`bb15c919`) on. `remove_worktree_scaffold` (`git/cleanup/worktree.rs`)
runs ahead of `git worktree remove` and demanded that the worktree's root `CLAUDE.md`
be a symlink, bailing otherwise. This repo tracks `CLAUDE.md`, so every worktree checks
it out as a regular file; `setup_root_claude_md` (`git/worktree/settings.rs`) already
knew that and skipped planting the symlink — the removal side did not. Git was never
asked, and the `?` chain in `cleanup_after_merge` then skipped the branch and
base-branch deletion too: `.worktrees/<id>`, `loom/<id>` and `loom/_base/<id>` all
survived every merge. The daemon's deferred-cleanup branch
(`orchestrator/core/merge_handler.rs`, the `stage.merged` short circuit) discarded the
`CleanupOutcome`, so the only trace was a `tracing::warn!` on daemon stderr.

**Misleading signal:** the worktree WAS clean — empty `git status`, stage merged and
Completed. Nothing pointed at loom's own pre-check; the natural suspects were untracked
files (a worktree-local runtime dir, say), which cannot be it: the non-forced
`git worktree remove` runs `git status --porcelain` without `--ignored`, so ignored paths
never block it.

**Why:** the creation side and the removal side of the scaffold encoded different
assumptions. Creation was conditional ("plant a symlink only if the checkout has
none"); removal was unconditional ("it must be our symlink"). Tests covered the
worktree-local CLAUDE.md symlink case, never a regular tracked root `CLAUDE.md`. And
bailing on "unexpected content" in `.claude/` re-implemented git's cleanliness check in
front of git, but stricter and blind — no path in the error, no ignore semantics.

**Prevention:** when a pre-removal step mirrors a conditional creation step, it must
mirror the condition: remove only what you can prove you planted (`remove_if_symlink`),
never assert the shape of what you did not. Do not re-implement git's cleanliness policy
ahead of `git worktree remove` — let git refuse, and name the blocking paths from
`git status --porcelain` in the error. A daemon path that calls cleanup must print a
`Failed`/`Refused` outcome; "logged at warn" is silence in a detached daemon. A
regression test for cleanup must build a real worktree from a repo that tracks the
scaffold path.

**Fix:** `remove_worktree_scaffold` removes the root `CLAUDE.md` only when it is a
symlink; `remove_known_claude_scaffold` skips unknown `.claude/` entries (Claude Code's
`.cc-writes/` runtime dir included) and keeps a non-empty dir; `cleanup_worktree`
appends the blocking paths when git refuses; `try_auto_merge` prints failed/refused
deferred cleanups with the `loom worktree remove <id>` hint. And because the daemon's
console is `.work/orchestrator.log` (fds 1 and 2 are `dup2`'d there in
`daemon/server/lifecycle.rs`, and the `LogLine` broadcaster has no client), printing is
not enough: `MergeLifecycle::cleanup` — the one door — now records a failed or refused
outcome as `Stage.cleanup_warning`, cleared by the next cleanup that succeeds, and
`loom status` (static graph, `--verbose` attention list, live TUI tree) renders it as
`cleanup failed` with the hint. Regression test:
`test_cleanup_worktree_succeeds_when_repo_tracks_root_claude_md` in
`git/cleanup/tests.rs`.

## Cleanup Refused Over Loom's Own Memory Spool (2026-08-27)

**What happened:** every worktree whose stage recorded a memory note survived its merge.
`git worktree remove` refused with "contains modified or untracked files", and the
`Blocking paths` line named `?? .loom/memory-spool.jsonl` — loom's own file.
`fs/memory/spool.rs` writes `<worktree>/.loom/memory-spool.jsonl` as the sandbox fallback
for `loom memory note` (`.work` is a symlink outside the write boundary). Teardown drained
the spool but left the file, and nothing ignored or removed it.

**Why it was invisible here:** this repository's own `.gitignore` lists `.loom/cache/` and
`.loom/memory-spool.jsonl` by hand. No other project has those lines, so loom broke worktree
cleanup everywhere EXCEPT in its own checkout — the one place it gets exercised daily. The
failure was found in a different repository.

**Second defect, found while fixing the first — the per-worktree exclude never worked.**
`add_settings_local_to_worktree_gitignore` wrote to `<repo>/.git/worktrees/<stage-id>/info/exclude`,
with a doc comment asserting that acts as a per-worktree `.gitignore`. **Git resolves `info/`
to the COMMON git dir**, so that file is never read. Demonstrated directly:

```text
rule in .git/worktrees/wt/info/exclude  →  git status: ?? ignored.txt   (NOT ignored)
rule in .git/info/exclude               →  git status: (clean)          (ignored)
```

The function had therefore never done anything since it was written. `.claude/settings.local.json`
escaped notice only because most projects gitignore `.claude/` themselves.

**Prevention:**

- **A cleanup path must know about every file its own product plants.** Creation and removal
  drifted apart again, exactly as in the scaffold incident above. When adding a runtime file
  under a worktree, update `is_worktree_scaffold_path`, `remove_worktree_scaffold`, and the
  exclude list in the same change.
- **Never let the tool's own repo be the only test bed.** A hand-written `.gitignore` line in
  this checkout masked a universal breakage. When a fix depends on ignore rules, ask what a
  fresh project without them would do.
- **Assert git's BEHAVIOUR, not the file content.** A test that only asserted the exclude file
  contained the pattern passed for as long as the bug existed. The regression test now runs
  `git check-ignore` / `git status --porcelain` against a real worktree.

**Fix:** `remove_worktree_scaffold` deletes the drained spool when it is a regular file loom
planted (never a symlink, never a tracked path), then removes `.loom/` only if that leaves it
empty. The exclude writer targets the common `.git/info/exclude` and lists exactly
`.loom/memory-spool.jsonl` and `.loom/cache/` — **not** a blanket `.loom/`, which would hide a
committable `.loom/config.toml`.

## Related

- `mistakes/phantom-merges.md` — the parent failure class, including the original
  "wrote merged=true without verifying ancestry" incidents.
- `architecture/context-retrieval.md` — the overlay/base reconcile steps this
  ordering interleaves, and why base publication requires a clean tree.
