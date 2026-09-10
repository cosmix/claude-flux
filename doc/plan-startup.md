# Committed plan inputs

Before `loom run` starts any stages, the active plan and its design briefs must be
committed on the base branch selected by `loom init`. This also applies to
`loom run --foreground` and restarts.

Loom checks the plan recorded in the workspace configuration, its conventional
`doc/plans/briefs/<plan-slug>/` bundle, and `doc/plans/briefs/` references in the
plan and linked briefs. Directory and glob references include their files.
Unrelated untracked plans and scratch files do not block startup.

If inputs are missing, ignored, untracked, staged without a commit, or modified,
startup stops and lists the affected paths. Restore missing inputs and commit
the listed files on the configured base branch, then run `loom run` again.

Once the checks pass, Loom renames the plan to `IN_PROGRESS-...` and commits
that rename before creating worktrees or publishing the source graph. The commit
contains only the rename. A restart with an already committed active filename
does not create another commit. If the rename commit fails, startup stops;
complete the rename commit and retry.

## Existing cleanup failures

Copying uncommitted inputs into a running stage's worktree can produce either
of these outcomes:

- The stage merges, but worktree removal refuses because the copied files remain
  untracked. `loom status --verbose` reports the blocking paths.
- The copied files are committed in the stage, but merging refuses because it
  would overwrite the untracked originals in the main checkout.

The startup check prevents this situation in new worktrees. It does not delete
files from existing worktrees. Preserve and compare the copies before resolving
an existing warning; a successful merge alone does not prove untracked copies
can be discarded.
