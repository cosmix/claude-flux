//! `loom hook reconcile-graph` — the A.12/A.22 debounced background self-heal.
//!
//! The prompt hook's own latency budget (a few seconds, well inside the shell
//! wrapper's 5s ceiling) must never pay for rebuilding a source graph, but a
//! stale or [`ContextPack::degraded`] pack (A.11) still deserves fixing —
//! just not on the request that noticed it. [`spawn_if_needed`] launches this
//! module's own subcommand, [`reconcile_graph`], detached and unawaited, so
//! the *next* retrieval benefits instead. See
//! `doc/PROPOSAL-retrieval-precision.md` §A.12/§A.22 and §P3 recommendation
//! 12 for the design.
//!
//! ## Debounce
//!
//! See the `lock` submodule for the `reconcile.lock` file's encoding and the
//! full Spawn/Skip policy table. In short: `try_reconcile` never unlinks
//! the lock, it REWRITES it — once at the very start, correcting the pid
//! field from whatever [`spawn_if_needed`] claimed it with to this process's
//! own pid, and once at the very end to a finished marker, on BOTH outcomes.
//! See `try_reconcile`'s own comments for why each rewrite is necessary.
//!
//! ## Scope resolution
//!
//! `HookTarget` is shared with the prompt and pre-compact delegates, so the
//! reader and background writer cannot derive different overlay addresses.
//! [`ensure_snapshot`] owns both the stage-overlay and local-current rebuild
//! policies; this module only selects the policy carried by that target.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;

use crate::context::config::RetrievalConfig;
use crate::context::graph_store::GraphStore;
use crate::context::refresh::ensure_snapshot;
use crate::context::schema::ContextPack;
use crate::context::store::ContextStore;
use crate::fs::work_dir::WorkDir;

use super::target::{non_empty_env, HookTarget};

mod lock;
use lock::{claim_lock, decide, reconcile_lock_path, unix_now, LockDecision};

/// The `loom hook reconcile-graph` subcommand body: a best-effort, one-shot
/// reconcile of the source graph for whatever scope
/// `HookTarget::from_environment` resolves.
///
/// Always exits `Ok(())` and prints nothing: this is an internal maintenance
/// entry point [`spawn_if_needed`] launches detached from a hook, so nothing
/// here may ever surface as a session-visible error or stray output. Every
/// failure — no resolvable state directory, a git failure, a graph-store I/O error —
/// is logged at `tracing::debug` and swallowed.
pub fn reconcile_graph() -> Result<()> {
    if let Err(error) = try_reconcile() {
        tracing::debug!(%error, "reconcile-graph: best-effort reconcile did not complete");
    }
    Ok(())
}

/// The fallible half of [`reconcile_graph`]: resolve scope, correct the
/// lock's pid, reconcile, and leave a finished marker on the way out
/// regardless of the reconcile's own outcome.
fn try_reconcile() -> Result<()> {
    let Some(target) = HookTarget::from_environment().filter(HookTarget::exists) else {
        // No state directory resolvable from the environment or cwd at all — nothing
        // to reconcile, and not a failure: a bare checkout with no loom
        // project is a legitimate place for this to be invoked from.
        return Ok(());
    };
    let work_dir = WorkDir::new(&target.work_dir)?;
    let store = ContextStore::open(&work_dir)?;
    let lock_path = reconcile_lock_path(&store);

    // Correct the lock to THIS process's own pid. `spawn_if_needed` claims it
    // with the SPAWNING hook's pid, because it does not yet know the child's
    // real pid before `Command::spawn` returns — but that spawning hook is
    // short-lived and typically exits within its own latency budget, well
    // before a slow reconcile finishes. Leaving its pid on record would make
    // `decide` read a still-running reconcile as "owner dead" moments after
    // spawn and take over it with a duplicate. A plain overwrite (`take_over
    // = true`) is correct here regardless of whether a lock existed at all —
    // this also covers `reconcile_graph` being invoked directly, with no
    // prior claim (as the tests and a manual operator invocation both do).
    let _ = claim_lock(&lock_path, unix_now(), std::process::id(), true);

    let outcome = reconcile(&target, &store);

    // Finished marker, both outcomes — see the module doc's "Debounce"
    // section. Best-effort: a failed write here costs only a debounce
    // interval, never the reconcile's own result.
    let _ = claim_lock(&lock_path, unix_now(), 0, true);

    outcome
}

/// Reconcile exactly the graph scope resolved for the other hook delegates.
fn reconcile(target: &HookTarget, store: &ContextStore) -> Result<()> {
    let graph_store = GraphStore::new(store.root(), &target.work_dir);
    ensure_snapshot(
        store,
        &graph_store,
        &target.project_root,
        target.snapshot_policy(),
    )?;
    Ok(())
}

/// Spawn a detached `loom hook reconcile-graph` when `pack` reports the
/// source graph stale or degraded (A.11), debounced through
/// `reconcile_lock_path` so a burst of hook invocations spawns at most one
/// reconcile at a time.
///
/// Fire-and-forget by contract: never waits on the child, never fails, never
/// prints — the hook's own latency budget must stay unaffected by however
/// long a background reconcile takes.
pub fn spawn_if_needed(pack: &ContextPack, project_root: &Path) {
    if !pack.semantic_freshness.stale && pack.degraded.is_none() {
        return;
    }
    if let Err(error) = try_spawn(project_root) {
        tracing::debug!(%error, "reconcile-graph: could not spawn a background reconcile");
    }
}

/// Whether `store` — `project_root` as resolved by [`try_spawn`]'s caller —
/// is a legitimate target for a detached, full-repo reconcile, independent
/// of the debounce lock decided below. A `loom hook reconcile-graph` run
/// walks every tracked file through tree-sitter and rewrites a
/// multi-megabyte graph; that is too expensive to launch against a checkout
/// the caller reached only by an upward directory search from an unrelated
/// working directory (`WorkDir::new`'s state-directory-search fallback — the
/// same path [`HookTarget::from_environment`] takes when `LOOM_WORK_DIR` is
/// unset).
///
/// Allowed when EITHER:
/// - `LOOM_WORK_DIR` was set in the environment: the caller named its
///   target explicitly, so nothing here was inferred, or
/// - `store.root()` (`<main project root>/.loom/cache/context-v1` —
///   `context::store::CACHE_RELATIVE_DIR`) already exists on disk, meaning
///   loom already maintains a context cache for this checkout. This is
///   exactly what a `loom map`'d repository has (`commands::map`'s
///   `load_graph` calls `ContextStore::ensure` before returning), so an
///   ordinary interactive session in a mapped repository — A.12's own
///   target use case — is unaffected by this gate.
///
/// Reading an index reached by an inferred root is fine (`loom map` itself
/// does exactly that); starting a job that rewrites one is not.
fn allowed_to_spawn(store: &ContextStore) -> bool {
    non_empty_env("LOOM_WORK_DIR").is_some() || store.root().is_dir()
}

/// The fallible half of [`spawn_if_needed`]: check the target is trusted,
/// decide, claim the lock, spawn.
fn try_spawn(project_root: &Path) -> Result<()> {
    let work_dir = WorkDir::new(project_root)?;
    let store = ContextStore::open(&work_dir)?;

    if !allowed_to_spawn(&store) {
        tracing::debug!(
            project_root = %project_root.display(),
            "reconcile-graph: refusing to spawn against an inferred project root with no existing cache"
        );
        return Ok(());
    }

    let lock_path = reconcile_lock_path(&store);

    let main_root = work_dir
        .main_project_root()
        .unwrap_or_else(|| project_root.to_path_buf());
    let config = RetrievalConfig::load(&main_root);

    let now = unix_now();
    let decision = decide(
        &lock_path,
        now,
        config.reconcile_debounce_secs,
        config.reconcile_stale_lock_secs,
        crate::process::is_process_alive,
    );
    if decision == LockDecision::Skip {
        return Ok(());
    }

    let take_over = lock_path.exists();
    if !claim_lock(&lock_path, now, std::process::id(), take_over) {
        // Lost the claim race to another hook, or the cache dir is
        // unwritable — either way, do not spawn a second, uncoordinated
        // reconcile on top of whatever just won.
        return Ok(());
    }

    spawn_detached(project_root)
}

/// Set false to suppress every detached spawn for the remainder of this
/// process. [`spawn_detached`] is the ONLY place this is read — everything
/// above it in the call chain ([`spawn_if_needed`]'s staleness check,
/// [`allowed_to_spawn`]'s inferred-root gate, `lock::decide`'s debounce
/// policy, `lock::claim_lock`'s lock claim) keeps running and stays
/// exercisable by tests exactly as before; only the actual
/// `Command::spawn()` call is suppressed. A process-group-leading child that
/// outlives the test harness is not something a test build may create: the
/// harness exits, the child does not, and a full source-graph reconcile
/// over a real repository is expensive enough that a handful of leaked ones
/// can exhaust a machine — exactly the incident this guard exists to
/// prevent (a real reconcile was launched from `tests_user_prompt_e2e.rs`
/// before this guard existed, against a genuinely stale checkout its
/// `WorkDir` upward search resolved to by accident).
///
/// Defaults to disabled whenever THIS CRATE is itself compiled with
/// `--cfg test`, which is true for every `#[cfg(test)]` unit test in this
/// crate (including the one above) with no extra wiring required. It does
/// NOT cover the integration targets under `loom/tests/*.rs`: those link
/// this crate compiled WITHOUT `--cfg test`, so `cfg!(test)` reads `false`
/// there too, the same as a real binary. An integration test that reaches
/// [`spawn_if_needed`] must call [`disable_spawn_for_tests`] itself before
/// doing so.
static SPAWN_ENABLED: AtomicBool = AtomicBool::new(!cfg!(test));

/// How many times [`spawn_detached`] was called while [`SPAWN_ENABLED`] was
/// false. Exists purely so a test can assert the guard actually fired
/// without needing to observe (or fail to observe) a real child process,
/// which is exactly what the guard exists to prevent creating.
#[cfg(test)]
static SUPPRESSED_SPAWNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Disable `spawn_detached` for the remainder of this process. Idempotent.
///
/// Deliberately `pub`, not `pub(crate)`, and NOT `#[cfg(test)]`-gated, even
/// though it exists only for tests: each file under `loom/tests/*.rs` is its
/// own crate that merely depends on this one (`pub(crate)` would be invisible
/// to it, and `#[cfg(test)]` would not exist in a build without `--cfg
/// test`), so reaching it from there requires both a real, external-visible
/// item and one that exists in every build. Call this before exercising any
/// path that reaches [`spawn_if_needed`] from outside this crate's own unit
/// tests, which get it for free — see `SPAWN_ENABLED`'s doc.
pub fn disable_spawn_for_tests() {
    SPAWN_ENABLED.store(false, Ordering::SeqCst);
}

/// Launch `loom hook reconcile-graph` detached from this process: no stdio,
/// its own process group so it survives the hook's exit, and NEVER
/// awaited — `wait()`/`output()` here would turn a background self-heal into
/// a foreground stall on the hook's own latency budget.
fn spawn_detached(project_root: &Path) -> Result<()> {
    if !SPAWN_ENABLED.load(Ordering::SeqCst) {
        // See `SPAWN_ENABLED`'s doc: a test build must never create a real,
        // process-group-leading child that survives the test harness.
        #[cfg(test)]
        SUPPRESSED_SPAWNS.fetch_add(1, Ordering::SeqCst);
        return Ok(());
    }

    let program = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("loom"));
    let mut command = Command::new(program);
    command
        .args(["hook", "reconcile-graph"])
        .current_dir(project_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // Passed through, not inherited implicitly: `Command` already inherits
    // the parent's environment by default, but naming these two explicitly
    // documents that they are load-bearing for A.22 (a stage's own scope
    // resolution) rather than incidental.
    for key in ["LOOM_STAGE_ID", "LOOM_WORK_DIR"] {
        if let Ok(value) = std::env::var(key) {
            command.env(key, value);
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    command.spawn()?;
    Ok(())
}

#[cfg(test)]
#[path = "tests_reconcile_graph.rs"]
mod tests;
