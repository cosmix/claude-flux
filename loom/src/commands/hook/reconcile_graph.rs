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
//! `ReconcileTarget::from_environment` mirrors
//! `commands::hook::user_prompt::DeliveryTarget::from_environment` point for
//! point (stage scope when `LOOM_STAGE_ID`/`LOOM_WORK_DIR` name a real stage,
//! else the checkout's own working-tree overlay) but is a second, small
//! implementation rather than a shared one: `DeliveryTarget` is private to
//! that sibling file, which this module does not own.
//!
//! Checkout scope calls
//! `crate::context::refresh::reconcile_semantic_best_effort` — promoted
//! from `pub(super)` to `pub(crate)` in `context::refresh::semantic` for
//! exactly this call site — rather than re-implementing its Base-then-overlay
//! policy (always ensure the immutable committed base, then add the `_local`
//! working-tree overlay when the checkout is dirty). One derivation of
//! that policy, not two: see the promoted function's own doc comment.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};

use crate::context::config::RetrievalConfig;
use crate::context::delivery;
use crate::context::graph_store::GraphStore;
use crate::context::refresh::{
    reconcile_semantic_best_effort, reconcile_source_graph, SourceGraphScope,
};
use crate::context::schema::ContextPack;
use crate::context::store::ContextStore;
use crate::fs::work_dir::WorkDir;
use crate::validation::validate_id;

mod lock;
use lock::{claim_lock, decide, reconcile_lock_path, unix_now, LockDecision};

/// The `loom hook reconcile-graph` subcommand body: a best-effort, one-shot
/// reconcile of the source graph for whatever scope
/// `ReconcileTarget::from_environment` resolves.
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
    let Some(target) = ReconcileTarget::from_environment() else {
        // No state directory resolvable from the environment or cwd at all — nothing
        // to reconcile, and not a failure: a bare checkout with no loom
        // project is a legitimate place for this to be invoked from.
        return Ok(());
    };
    let store = ContextStore::open(&target.work_dir)?;
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

/// Where this session's reconcile is scoped, and the state directory it
/// resolves against.
struct ReconcileTarget {
    work_dir: WorkDir,
    /// `Some((plan, stage))` for a real stage's own overlay (A.22); `None`
    /// for the checkout's working-tree overlay — see [`reconcile`].
    stage: Option<(String, String)>,
}

impl ReconcileTarget {
    /// A stage scope whenever the environment really names one; the
    /// checkout otherwise.
    ///
    /// The single existence check on `work_dir.root()` at the end guards
    /// both `for_stage` and `for_checkout`: `WorkDir::new` never fails
    /// (`try_reconcile`'s own doc comment promises "no state directory
    /// resolvable ... nothing to reconcile, and not a failure"), so without this a
    /// stale `LOOM_STAGE_ID`/`LOOM_WORK_DIR` pin naming a since-deleted
    /// state directory would resolve to a `ReconcileTarget` anyway, and
    /// `ContextStore::open` below would recreate that state directory from
    /// scratch in a checkout that was never `loom init`ed.
    fn from_environment() -> Option<Self> {
        Self::for_stage()
            .or_else(Self::for_checkout)
            .filter(|target| target.work_dir.root().exists())
    }

    /// Resolve from `LOOM_STAGE_ID`/`LOOM_WORK_DIR`. `None` when either is
    /// unset, unusable, or names no stage on disk.
    ///
    /// `LOOM_STAGE_ID` becomes a path component (the stage's overlay
    /// directory under `<state-dir>/context/<plan>/<stage>/`), so it is validated
    /// HERE, at the boundary, exactly as
    /// `commands::hook::user_prompt::DeliveryTarget::for_stage` validates
    /// it — do not weaken this.
    fn for_stage() -> Option<Self> {
        let stage_id = non_empty_env("LOOM_STAGE_ID")?;
        validate_id(&stage_id).ok()?;
        let work_dir = WorkDir::new(non_empty_env("LOOM_WORK_DIR")?).ok()?;
        let stage = crate::verify::load_stage(&stage_id, work_dir.root()).ok()?;
        let plan = delivery::plan_key(&stage).to_string();
        Some(ReconcileTarget {
            work_dir,
            stage: Some((plan, stage_id)),
        })
    }

    /// The checkout this process is running in, for no stage at all —
    /// `LOOM_WORK_DIR` when set, else the current directory, matching
    /// `DeliveryTarget::for_checkout`'s fallback so a hook invoked with no
    /// loom stage in scope still resolves to the same state directory retrieval did.
    fn for_checkout() -> Option<Self> {
        let hint = non_empty_env("LOOM_WORK_DIR").unwrap_or_else(|| ".".to_string());
        let work_dir = WorkDir::new(hint).ok()?;
        Some(ReconcileTarget {
            work_dir,
            stage: None,
        })
    }
}

/// A set environment variable with non-blank content.
fn non_empty_env(name: &str) -> Option<String> {
    let value = std::env::var(name).ok()?;
    (!value.trim().is_empty()).then_some(value)
}

/// Reconcile `target`'s scope: a real stage's own overlay when named, else
/// the checkout's working-tree overlay.
fn reconcile(target: &ReconcileTarget, store: &ContextStore) -> Result<()> {
    let project_root = target
        .work_dir
        .project_root()
        .context("could not resolve a project root for this state directory")?;

    match &target.stage {
        Some((plan, stage)) => {
            // A stage's own worktree is always eligible for its own overlay —
            // unlike the checkout branch below, `Overlay` scope has no
            // dirty-tree gate, because a stage's tree being dirty (mid-edit)
            // is the expected, common case this exists to serve (A.22).
            let graph_store = GraphStore::new(store.root(), target.work_dir.root());
            let scope = SourceGraphScope::Overlay {
                plan: plan.clone(),
                stage: stage.clone(),
            };
            reconcile_source_graph(store, &graph_store, project_root, scope)?;
        }
        None => {
            // The checkout's own working-tree scope: always ensure the
            // committed Base; a dirty tree also gets the `_local` overlay.
            // `reconcile_semantic_best_effort` is the ONE place that policy
            // lives (promoted to `pub(crate)` in `context::refresh::semantic`
            // for exactly this call) — this module must not re-derive it; see
            // its doc comment. It never returns `Err`, so there is nothing
            // further to propagate here; the state write it performs (or
            // does not, on failure) is its own side effect.
            let current = store.load_state()?.semantic;
            reconcile_semantic_best_effort(store, project_root, current);
        }
    }
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
/// working directory (`WorkDir::new`'s state-directory-search fallback — the same
/// path `ReconcileTarget::for_checkout` above and
/// `user_prompt::DeliveryTarget::for_checkout` both take when
/// `LOOM_WORK_DIR` is unset).
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
