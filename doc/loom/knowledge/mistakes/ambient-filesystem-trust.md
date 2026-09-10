# Ambient Filesystem Trust

> Why an ancestor directory merely named .git is not evidence of a real repository, and the validation this requires.

## An Ancestor Named `.git` Is Not a Repository (2026-08-29)

**What happened:** `loom memory note` run from a directory with no repository in its own ancestry
wrote its journal into an unrelated directory far above it. On the machine where this surfaced,
that was the OS temp root, shared by every process and every concurrent agent on the box.

**Why:** `get_or_create_work_dir` accepted a candidate root on `root.join(".git").exists()`.
`find_repo_root_from_cwd` (`git/worktree/paths.rs`) walks up with NO ceiling, so from a repo-less
cwd it climbs to `/` and returns the first ancestor holding anything named `.git`. An empty
directory with that name — left by some unrelated tool — satisfied the check. The existence test
was guarding the wrong branch: it was written to catch the helper's "found nothing, returning cwd"
fallback, and did nothing about the "found something" branch it was actually reached through.

**Prevention:** an unbounded upward walk will eventually leave the territory you meant it to
search, so whatever it returns must be validated by STRUCTURE, not by name. Ask what the check
does when the named thing exists but is empty or foreign. Be especially suspicious of any walk
that can reach a shared root — `/tmp`, `$HOME`, a mount point — where unrelated processes leave
debris.

**Fix:** `is_real_git_dir` requires a `.git` DIRECTORY to also carry a `HEAD`, which every real
repository has from the moment it is created; a `.git` FILE is still accepted on existence alone,
since that is the worktree pointer form and git always writes real content there. Only the
creation path is affected; the reuse and read-only degrade paths were already incapable of
creating anything.

**How it was found, which is the more general lesson:** as 77 test failures across seven unrelated
modules. `WorkDir::new` also searches upward, so the one stray `.work` this bug created at the temp
root was adopted by every test that built a `TempDir` beneath it. The failures looked environmental
and were not — they were a real production defect reported through an unrelated symptom. A test
failure whose cause looks like "the machine" deserves a root cause before it earns that label.

## `find_repo_root_from_cwd` Returns `Some(cwd)` Outside Any Repo (2026-08-11)

**What happened:** `get_or_create_work_dir` in `commands/memory/handlers/work_dir.rs` needed "am I inside a
git repo?" before it would create a `.work` directory. `find_repo_root_from_cwd` returns
`Option<PathBuf>`, so `None` reads as "not in a repo" — but it is not. After walking to the
filesystem root without finding a `.git`, it ends at
`git/worktree/paths.rs:84-85` with an explicit _"Fallback: return the original cwd if nothing else
works"_ → `cwd.canonicalize().ok()`. Outside any repo it therefore returns `Some(cwd)`.

**Why it matters:** the name says _find repo root_ and the `Option` implies a search that can
fail, so `if let Some(root)` looks like a repo check and compiles clean. Here it would have
scattered a `.work` directory into any directory the command was ever run from.

**Prevention:** treat `find_repo_root_from_cwd` as _"the best base path to use"_, never as a repo
predicate. When you need the predicate, confirm it yourself:
`find_repo_root_from_cwd(&cwd).filter(|root| root.join(".git").exists())`.

**Detection:** the giveaway is an `Option` whose `None` arm you cannot trigger in a test. If you
cannot write the failing case, the function probably never returns `None` — read its tail before
relying on it. Existing callers are unaffected only because they all pair it with
`.unwrap_or_else(|| cwd)`, which wants exactly the fallback; that idiom hides the trap from anyone
reading call sites to infer semantics.
