//! Update notification — a stderr line when a newer release exists, never an
//! installer. `loom update` (`crate::commands::self_update`) stays the
//! only thing that writes a new binary; this module only reads a small state
//! file and, at most, spawns a detached child to refresh it.
//!
//! # The hot path takes no network call
//!
//! `loom` is invoked constantly from Claude Code hooks, so a synchronous
//! fetch on every launch would be a latency disaster. [`notify_and_maybe_refresh`]
//! only ever reads `update-state.json` from disk; the actual GitHub request
//! happens in a DETACHED child (`loom __update-refresh`, [`run_refresh`]),
//! spawned at most once per stale interval and never awaited.
//!
//! # Failures are silent everywhere
//!
//! A network error, an unreadable or torn state file, or an unresolvable home
//! directory must never make a loom command fail, panic, or print noise. None
//! of the IO in this module unwraps or expects; every fallible read collapses
//! to "absent" and every fallible write is best-effort.
//!
//! # The notice goes to stderr, never stdout
//!
//! Every loom command's stdout is somebody's input (`loom plan verify --json`,
//! `loom usage`, `loom config -k <key>`), so the notice goes to stderr; the
//! argv exclusion list in `main.rs` (`suppresses_update_check`) is a second
//! line of defence, not the first.
//!
//! # Exactly one fetcher
//!
//! Two invocations of loom racing this module is the common case, since loom
//! runs from every hook. `schedule_refresh` publishes a lock file before
//! spawning so at most one detached fetcher is in flight per stale interval;
//! a fetch that fails still stamps `last_checked` so a network outage backs
//! off instead of respawning on every invocation.

mod decide;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use chrono::{DateTime, Duration, Utc};
use semver::Version;

use decide::decide;
#[cfg(test)]
use decide::notice_for;

/// The persisted record at `<loom dir>/update-state.json`, every field
/// optional so a partial or older record still parses; anything unparseable
/// reads as fully absent instead ([`read_state`]).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct UpdateState {
    #[serde(default)]
    last_checked: Option<DateTime<Utc>>,
    #[serde(default)]
    latest_version: Option<String>,
}

/// The loom user directory: the parent of `crate::user_config::config_path()`
/// (`~/.loom`, or `$LOOM_HOME`). Delegates so the two rules can never drift
/// apart. Has no `#[cfg(test)]` redirect, so tests here pass an explicit
/// `tempfile::tempdir()` instead of calling this. Never creates anything.
fn resolve_dir() -> Option<PathBuf> {
    crate::user_config::config_path()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
}

fn state_path(dir: &Path) -> PathBuf {
    dir.join("update-state.json")
}

fn lock_path(dir: &Path) -> PathBuf {
    dir.join("update-check.lock")
}

/// Read and parse the state file, treating anything short of a fully valid
/// record — missing file, unreadable, torn, or malformed JSON — as absent. A
/// pure `read_to_string`, not `locking::locked_read`, which locks (and
/// creates) the parent directory — a read must never materialize `~/.loom/`.
fn read_state(dir: &Path) -> Option<UpdateState> {
    let text = std::fs::read_to_string(state_path(dir)).ok()?;
    serde_json::from_str(&text).ok()
}

/// The floor for two different things: (1) how often a fetch may actually
/// happen, applied to `decide`'s interval — `update.check_interval_hours`
/// may legitimately be `0` ("check as often as possible"), and without this
/// floor that would mean "on every process start", i.e. one forked fetcher
/// plus one unauthenticated GitHub request per loom invocation; and (2) the
/// minimum lifetime of a held refresh lock, applied to `schedule_refresh`'s
/// takeover check — otherwise that same `interval = 0` would make every lock
/// stale the instant it is written, so two racing invocations would each
/// take over the other's lock and both spawn a fetcher, exactly the
/// concurrency the lock exists to prevent. So `check_interval_hours = 0`
/// means "as often as this floor allows", never "on every process start".
const MIN_REFRESH_INTERVAL_MINUTES: i64 = 15;

/// Take the refresh lock for `dir`, or report that another invocation
/// already holds a live one. `true` means the caller should spawn a fetcher
/// and, eventually, [`release_lock`]. A lock whose owner is dead, or whose
/// stamp is missing/unreadable/unparseable, is presumed abandoned and taken
/// over once; a live owner's lock is taken over only past `interval` (floored
/// at [`MIN_REFRESH_INTERVAL_MINUTES`]); a second race loss after either
/// backs off.
///
/// Residual race: two invocations can both read the same stale, dead-owner
/// stamp and both take over — an atomic lock publish alone cannot close that
/// TOCTOU window, since `lock_is_stale` must read-then-decide before either
/// side publishes. Bounded at one redundant HTTPS GET plus one extra atomic
/// state write (see [`perform_refresh`]); never corruption.
fn schedule_refresh(dir: &Path, now: DateTime<Utc>, interval: Duration) -> bool {
    let _ = std::fs::create_dir_all(dir);
    let path = lock_path(dir);

    if try_create_lock(&path, now) {
        return true;
    }

    let lock_lifetime = interval.max(Duration::minutes(MIN_REFRESH_INTERVAL_MINUTES));
    if lock_is_stale(&path, now, lock_lifetime) {
        let _ = std::fs::remove_file(&path);
        return try_create_lock(&path, now);
    }

    false
}

/// Publish the refresh lock at `path`, stamped `"<rfc3339 now> <pid>"` so
/// staleness can be judged by content and owner liveness, not mtime.
///
/// Invariant: **the lock path existing must imply the lock is fully
/// stamped.** `create_new` then a separate `write_all` publishes the path
/// before the stamp lands, so a racer that loses `create_new` can read an
/// empty file in that window, have `parse_lock_stamp` fail, and have
/// `lock_is_stale` call it abandoned and take over — two winners. Writing the
/// stamp to a temp file beside `path` and publishing with
/// [`std::fs::hard_link`] (atomic, fails `AlreadyExists` if held) closes that
/// window. The temp name adds a per-process counter to the pid so this
/// module's own multi-threaded tests never collide, and is removed on every
/// path, success or failure.
fn try_create_lock(path: &Path, now: DateTime<Utc>) -> bool {
    let Some(dir) = path.parent() else {
        return false;
    };
    static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let unique = TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let pid = std::process::id();
    let temp_path = dir.join(format!(".update-check.lock.{unique}.{pid}.tmp"));
    let stamp = format!("{} {pid}", now.to_rfc3339());
    let published =
        std::fs::write(&temp_path, stamp).is_ok() && std::fs::hard_link(&temp_path, path).is_ok();
    let _ = std::fs::remove_file(&temp_path);
    published
}

/// Parse `"<rfc3339> <pid>"` out of a lock file's contents. `None` for
/// anything short of both fields parsing cleanly — [`lock_is_stale`] treats
/// that identically to a corrupt lock (stale), never as "someone is on it,
/// forever".
fn parse_lock_stamp(text: &str) -> Option<(DateTime<Utc>, u32)> {
    let mut parts = text.trim().splitn(2, ' ');
    let stamp = DateTime::parse_from_rfc3339(parts.next()?).ok()?;
    let pid: u32 = parts.next()?.trim().parse().ok()?;
    Some((stamp.with_timezone(&Utc), pid))
}

/// Whether the lock at `path` should be treated as abandoned, per the policy
/// on [`schedule_refresh`]'s doc: missing means the owner completed cleanly
/// (not stale); an unreadable or unparseable stamp is stale (corrupt must not
/// block forever); a dead owner is stale at any age; a live owner is stale
/// only past `interval`, the ceiling for a hung fetcher.
fn lock_is_stale(path: &Path, now: DateTime<Utc>, interval: Duration) -> bool {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        // The previous owner released the lock; there is nothing to take
        // over, and reporting "stale" here is exactly the bug that let a
        // second invocation spawn a fetcher milliseconds after a completed
        // one.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
        Err(_) => return true,
    };
    let Some((stamp, pid)) = parse_lock_stamp(&text) else {
        return true;
    };
    if !crate::process::is_process_alive(pid) {
        return true;
    }
    now - stamp >= interval
}

/// Release the refresh lock. Errors ignored — a missing lock is not this
/// caller's problem.
fn release_lock(dir: &Path) {
    let _ = std::fs::remove_file(lock_path(dir));
}

/// Hidden argv token for the detached refresh child. Not a clap subcommand —
/// `main.rs` intercepts it before `Cli::parse()` — so `--help` never shows it.
pub const REFRESH_ARG: &str = "__update-refresh";

/// Launch `loom __update-refresh` detached from this process: no stdio, its
/// own process group, and NEVER awaited. `process_group(0)` moves the child
/// out of this process's group but not out from under it as a parent — it is
/// reparented to init only once THIS process exits, immediate for the
/// short-lived commands that dominate. Nothing ever waits on the child, so on
/// a long-lived foreground command (a status/attach TUI) a fetcher that
/// finishes first sits as `<defunct>` until the parent exits — harmless.
fn spawn_refresh(dir: &Path) -> std::io::Result<()> {
    // No unit test in this crate may fork a detached child into a worktree
    // that may be removed once its stage merges. `loom/tests/*.rs`
    // integration targets link this crate without `--cfg test` and are
    // unaffected — they exercise the real binary via a scratch `LOOM_HOME`
    // with the check switched off (`tests/integration/helpers.rs`).
    if cfg!(test) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "spawn_refresh is disabled under #[cfg(test)]",
        ));
    }

    let program = std::env::current_exe()?;
    let mut command = Command::new(program);
    command
        .arg(REFRESH_ARG)
        // The user's home, never the worktree this process may be running
        // in: a worktree is removed once its stage merges, and a fetcher
        // that outlives the parent must not depend on it still existing.
        .current_dir(dirs::home_dir().unwrap_or_else(|| dir.to_path_buf()))
        // All three: loom runs as a child of piped callers
        // (`verify/criteria/confine.rs`), and an inherited stdout/stderr fd
        // would keep that pipe open after loom exits, making the collector
        // block for the full output-collection timeout on every invocation.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(unix)]
    {
        // Safe-Rust process-group reassignment, not a `pre_exec` `setsid`:
        // this crate denies undocumented unsafe blocks, and the existing
        // detached-spawn precedent (`reconcile_graph::spawn_detached`) shows
        // `process_group(0)` is enough to survive the parent's exit.
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    command.spawn()?;
    Ok(())
}

/// Fetch the latest release's version, reusing
/// `commands::self_update::get_latest_release` (widened to `pub(crate)`).
fn fetch_latest_version() -> anyhow::Result<Version> {
    let release = crate::commands::self_update::get_latest_release()?;
    Ok(Version::parse(release.tag_name.trim_start_matches('v'))?)
}

/// The detached child's worker, factored over an injected clock and fetch so
/// it is testable without network access or a real spawn. A failed fetch
/// still stamps `last_checked`: the exclusive lock in [`schedule_refresh`]
/// stops two fetchers running at once, but if a failed attempt left
/// `last_checked` untouched, every subsequent loom invocation — loom runs
/// from every hook — would spawn another fetcher for as long as the network
/// stayed down. Stamping the attempt is what turns the interval into backoff.
fn perform_refresh<F>(dir: &Path, now: DateTime<Utc>, fetch: F)
where
    F: FnOnce() -> anyhow::Result<Version>,
{
    let mut state = read_state(dir).unwrap_or_default();

    if let Ok(version) = fetch() {
        state.latest_version = Some(version.to_string());
    }
    state.last_checked = Some(now);

    let _ = std::fs::create_dir_all(dir);
    if let Ok(text) = serde_json::to_string(&state) {
        // Outside a held directory lock, which `atomic_write_locked`'s own
        // doc flags as reintroducing lost-update/torn-read races — safe here
        // because writers are serialized by the exclusive refresh lock
        // (modulo `schedule_refresh`'s residual race), and readers use a
        // plain `read_to_string` against a path this helper only ever
        // `rename`s into place, so the worst case is a redundant whole-file
        // write, never a torn read.
        let _ = crate::fs::locking::atomic_write_locked(&state_path(dir), &text);
    }

    release_lock(dir);
}

/// Entry point for the detached child. Fetches, rewrites the state file,
/// releases the lock, and exits. Never prints — its stdio is `/dev/null`.
pub fn run_refresh() {
    let Some(dir) = resolve_dir() else { return };
    perform_refresh(&dir, Utc::now(), fetch_latest_version);
}

/// Entry point for every ordinary foreground invocation: reads the state
/// file, prints at most one line to stderr, and schedules a detached refresh
/// when stale. A dev build (see `decide`) prints nothing and schedules no
/// refresh. Takes no network call and never fails.
pub fn notify_and_maybe_refresh() {
    let Some(dir) = resolve_dir() else { return };
    let Ok(current) = Version::parse(env!("LOOM_VERSION")) else {
        return;
    };

    let config = crate::user_config::UserConfig::load();
    let state = read_state(&dir);
    let now = Utc::now();

    let action = decide(
        state.as_ref(),
        config.update_check(),
        config.update_check_interval_hours(),
        now,
        &current,
    );

    if let Some(notice) = &action.notice {
        eprintln!("{notice}");
    }

    if action.refresh {
        let interval = Duration::hours(i64::from(config.update_check_interval_hours()));
        if schedule_refresh(&dir, now, interval) && spawn_refresh(&dir).is_err() {
            release_lock(&dir);
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
