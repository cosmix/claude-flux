use std::env;
use std::path::PathBuf;

use tempfile::TempDir;

use super::*;

/// Guards against a stray `.git`/`.loom/work` directly under `env::temp_dir()`
/// itself (the OS-shared temp root that every `TempDir` in this suite is
/// nested under, not any one test's own scratch directory). Cleans up
/// whatever it finds there both before the test runs - so a leftover from
/// an earlier run doesn't make this test pass or fail on machine history
/// rather than behaviour - and after, via `Drop`, so a plant this test
/// makes doesn't survive a panicked assertion.
struct AmbientTempRootGuard {
    git: PathBuf,
    work: PathBuf,
}

impl AmbientTempRootGuard {
    fn new() -> Self {
        let root = env::temp_dir();
        let guard = Self {
            git: root.join(".git"),
            work: root.join(".loom").join("work"),
        };
        guard.clean();
        guard
    }

    fn clean(&self) {
        let _ = std::fs::remove_dir_all(&self.git);
        let _ = std::fs::remove_dir_all(&self.work);
    }
}

impl Drop for AmbientTempRootGuard {
    fn drop(&mut self) {
        self.clean();
    }
}

/// Regression guard for the stray-state-directory-at-the-temp-root bug:
/// `find_repo_root_from_cwd`'s upward walk has no ceiling, so from a cwd
/// with no repo in its own ancestry it can climb all the way to the
/// OS-shared temp root and, if something else left an ancestor directory
/// there merely NAMED `.git` (no `HEAD`, no real git internals - exactly
/// what an unrelated process sharing that root leaves behind), the old
/// `.exists()` check accepted it as a repo root and created `.loom/work` next to
/// it. This plants that impostor directly under `env::temp_dir()`, then
/// runs the same "outside any repo" scenario as
/// `note_outside_git_repo_still_fails` from a nested, git-less subdirectory,
/// and asserts `note` still fails and nothing is created at the temp root.
#[test]
#[serial]
fn note_does_not_adopt_an_impostor_git_dir_at_the_temp_root() {
    let _guard = EnvGuard::new();
    env::remove_var("LOOM_STAGE_ID");
    let _temp_root_guard = AmbientTempRootGuard::new();

    // Plant an empty directory literally named `.git` at the OS temp root -
    // an ancestor of every `TempDir` below, but not a real repository.
    std::fs::create_dir_all(env::temp_dir().join(".git")).unwrap();

    // A plain temp dir nested under that same root, with no `.git` of its
    // own, so the only `.git` anywhere in its ancestry is the impostor.
    let plain_dir = TempDir::new().unwrap();
    env::set_current_dir(plain_dir.path()).unwrap();

    let result = note("should not be recorded".to_string(), Vec::new(), None);

    assert!(
        result.is_err(),
        "an impostor .git at the temp root must not be adopted as a repo root"
    );
    assert!(!env::temp_dir().join(".loom").join("work").exists());
}
