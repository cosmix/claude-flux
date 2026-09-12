//! The pure "what should this invocation do" half of the update check: no
//! IO, no network call, and no clock beyond what the caller passes in.

use super::{UpdateState, MIN_REFRESH_INTERVAL_MINUTES};
use chrono::{DateTime, Duration, Utc};
use semver::Version;

/// What the foreground should do this invocation: print at most one line,
/// and/or hand off to a detached fetcher.
pub(super) struct Action {
    pub(super) notice: Option<String>,
    pub(super) refresh: bool,
}

/// The "is there a newer release" half of [`decide`]. A garbage or
/// unparseable `latest_version`, or one no newer than `current`, reads as
/// "no notice" rather than an error.
pub(super) fn notice_for(state: Option<&UpdateState>, current: &Version) -> Option<String> {
    state
        .and_then(|s| s.latest_version.as_deref())
        .and_then(|v| Version::parse(v.trim_start_matches('v')).ok())
        .filter(|latest| latest > current)
        .map(|latest| {
            format!(
                "loom {current} is out of date (latest {latest}) - run `loom update` to upgrade."
            )
        })
}

/// The pure decision: given the last-known state, the two `[update]` config
/// settings, the current time, and the running version, what should this
/// invocation do? Takes `check_enabled`/`interval_hours` rather than
/// `&UserConfig` so this function — the one real branch to test — needs no
/// seam into `user_config` (which this stage may not edit) to exercise the
/// disabled case.
pub(super) fn decide(
    state: Option<&UpdateState>,
    check_enabled: bool,
    interval_hours: u32,
    now: DateTime<Utc>,
    current: &Version,
) -> Action {
    if !check_enabled {
        return Action {
            notice: None,
            refresh: false,
        };
    }

    // A dev build (any non-empty semver `pre`, e.g. `0.0.0-dev+<sha>` or
    // `X.Y.Z-dev.N+<sha>` — see `version::derive`) is never told about
    // releases, so it also never schedules a fetch: a background GitHub
    // fetch whose result can never be shown is a wasted unauthenticated
    // request against the 60/hour rate limit the interval comment below
    // already guards.
    if !current.pre.is_empty() {
        return Action {
            notice: None,
            refresh: false,
        };
    }

    let notice = notice_for(state, current);
    let refresh = refresh_due(state, interval_hours, now);

    Action { notice, refresh }
}

/// Whether the state is stale enough to schedule a fetch.
fn refresh_due(state: Option<&UpdateState>, interval_hours: u32, now: DateTime<Utc>) -> bool {
    // Floored the same way `schedule_refresh` floors the lock lifetime (see
    // `MIN_REFRESH_INTERVAL_MINUTES`): otherwise `check_interval_hours = 0`
    // (a valid, user-settable config) would make every single invocation
    // decide to refresh, and since the fetcher releases its lock the moment
    // it finishes, the next invocation immediately forks another one — one
    // fetcher plus one unauthenticated GitHub request per loom invocation,
    // against a 60/hour rate limit, with loom running from every hook.
    let interval = Duration::hours(i64::from(interval_hours))
        .max(Duration::minutes(MIN_REFRESH_INTERVAL_MINUTES));
    match state.and_then(|s| s.last_checked) {
        None => true,
        Some(last_checked) => {
            let elapsed = now - last_checked;
            // A `last_checked` in the future (clock skew, e.g. a VM or CI
            // host whose clock jumps backwards) must not disable the check
            // permanently: the writer always stamps its own `now`, so the
            // very next successful refresh sets `last_checked = now` again
            // and the record self-heals after exactly one fetch — the
            // exclusive lock in `schedule_refresh` already bounds that to
            // one fetch per lock lifetime, so there is no "storm" to guard
            // against here.
            elapsed >= interval || elapsed < Duration::zero()
        }
    }
}
