use super::*;

#[test]
fn disabled_check_yields_no_notice_and_no_refresh() {
    let now = Utc::now();
    let current = Version::parse("0.2.0").unwrap();
    let state = UpdateState {
        last_checked: None,
        latest_version: Some("9.9.9".to_string()),
    };

    let action = decide(Some(&state), false, 24, now, &current);

    assert!(action.notice.is_none());
    assert!(!action.refresh);
}

#[test]
fn newer_latest_version_produces_one_notice_naming_both_versions() {
    let now = Utc::now();
    let current = Version::parse("0.2.0").unwrap();
    let state = UpdateState {
        last_checked: Some(now),
        latest_version: Some("0.3.0".to_string()),
    };

    let action = decide(Some(&state), true, 24, now, &current);

    let notice = action.notice.expect("a newer release should notice");
    assert!(notice.contains("0.2.0"), "{notice}");
    assert!(notice.contains("0.3.0"), "{notice}");
}

#[test]
fn dev_build_is_never_notified_and_never_refreshes() {
    let now = Utc::now();
    let current = Version::parse("0.2.1-dev.5+abc1234").unwrap();

    let newer = UpdateState {
        last_checked: Some(now),
        latest_version: Some("0.2.1".to_string()),
    };
    let action = decide(Some(&newer), true, 24, now, &current);
    assert!(
        action.notice.is_none(),
        "a dev build must never be notified, even of a newer release"
    );
    assert!(!action.refresh);

    let older = UpdateState {
        last_checked: Some(now),
        latest_version: Some("0.2.0".to_string()),
    };
    let action = decide(Some(&older), true, 24, now, &current);
    assert!(
        action.notice.is_none(),
        "a dev build must not be told it is behind an older release"
    );
    assert!(!action.refresh);

    let never_checked = UpdateState {
        last_checked: None,
        latest_version: Some("0.2.1".to_string()),
    };
    let action = decide(Some(&never_checked), true, 24, now, &current);
    assert!(action.notice.is_none());
    assert!(
        !action.refresh,
        "a dev build must not schedule a refresh even when last_checked is None"
    );

    let zero = Version::parse("0.0.0-dev+abc1234").unwrap();
    let ahead = UpdateState {
        last_checked: None,
        latest_version: Some("0.2.1".to_string()),
    };
    let action = decide(Some(&ahead), true, 24, now, &zero);
    assert!(action.notice.is_none());
    assert!(!action.refresh);
}

/// `notice_for` is the formatter `notify_and_maybe_refresh` prints straight
/// to stderr (never stdout — see the module doc). Covers the reachable part
/// of that print without capturing real process stderr or spawning anything.
#[test]
fn notice_for_names_both_versions_and_update_only_when_newer() {
    let current = Version::parse("0.2.0").unwrap();

    let newer = UpdateState {
        last_checked: None,
        latest_version: Some("0.3.0".to_string()),
    };
    let notice = notice_for(Some(&newer), &current).expect("a newer release should notice");
    assert!(notice.contains("0.2.0"), "{notice}");
    assert!(notice.contains("0.3.0"), "{notice}");
    assert!(notice.contains("loom update"), "{notice}");

    let older = UpdateState {
        last_checked: None,
        latest_version: Some("0.1.0".to_string()),
    };
    assert!(notice_for(Some(&older), &current).is_none());

    let equal = UpdateState {
        last_checked: None,
        latest_version: Some("0.2.0".to_string()),
    };
    assert!(notice_for(Some(&equal), &current).is_none());

    let unparseable = UpdateState {
        last_checked: None,
        latest_version: Some("not-a-version".to_string()),
    };
    assert!(notice_for(Some(&unparseable), &current).is_none());

    assert!(notice_for(None, &current).is_none());
}

#[test]
fn absent_or_corrupt_state_both_schedule_a_refresh_silently() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();
    let current = Version::parse("0.2.0").unwrap();

    assert!(read_state(dir.path()).is_none(), "no state file yet");
    let action = decide(None, true, 24, now, &current);
    assert!(action.refresh);
    assert!(action.notice.is_none());

    std::fs::create_dir_all(dir.path()).unwrap();
    std::fs::write(state_path(dir.path()), "not valid json").unwrap();
    assert!(
        read_state(dir.path()).is_none(),
        "a torn state file must read as absent, not error"
    );
}

#[test]
fn refresh_only_fires_once_the_interval_has_elapsed() {
    let now = Utc::now();
    let current = Version::parse("0.2.0").unwrap();

    let fresh = UpdateState {
        last_checked: Some(now - Duration::hours(1)),
        latest_version: None,
    };
    assert!(!decide(Some(&fresh), true, 24, now, &current).refresh);

    let stale = UpdateState {
        last_checked: Some(now - Duration::hours(25)),
        latest_version: None,
    };
    assert!(decide(Some(&stale), true, 24, now, &current).refresh);

    let future = UpdateState {
        last_checked: Some(now + Duration::hours(10)),
        latest_version: None,
    };
    assert!(
        decide(Some(&future), true, 24, now, &current).refresh,
        "a future last_checked (clock skew) must schedule a refresh rather than \
         wedging the check forever: the writer always stamps its own `now`, so \
         one refresh self-heals the record"
    );
}

#[test]
fn concurrent_stale_calls_schedule_at_most_one_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();
    let interval = Duration::hours(24);

    // 8 threads racing `schedule_refresh` against the same fresh directory:
    // exactly one may win the `O_EXCL` lock.
    let winners: usize = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| scope.spawn(|| schedule_refresh(dir.path(), now, interval)))
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|won| *won)
            .count()
    });
    assert_eq!(winners, 1, "exactly one racer must win the refresh lock");

    assert!(
        !schedule_refresh(dir.path(), now, interval),
        "a later invocation must lose the race, not fetch again"
    );
}

#[test]
fn a_stale_lock_is_taken_over() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();
    let interval = Duration::hours(24);

    // A lock stamped well outside the interval simulates an owner that
    // crashed before releasing it.
    assert!(schedule_refresh(
        dir.path(),
        now - Duration::hours(48),
        interval
    ));
    assert!(schedule_refresh(dir.path(), now, interval));

    // `update.check_interval_hours = 0` means "check every invocation", not
    // "every racer may take over a live lock": the lock keeps its own minimum
    // lifetime, so a second invocation an instant later still backs off.
    let zero = tempfile::tempdir().unwrap();
    assert!(schedule_refresh(zero.path(), now, Duration::zero()));
    assert!(
        !schedule_refresh(zero.path(), now + Duration::seconds(1), Duration::zero()),
        "a zero check interval must not defeat the one-fetcher lock"
    );
}

#[test]
fn perform_refresh_stamps_the_attempt_even_when_the_fetch_fails() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();

    let prior = UpdateState {
        last_checked: Some(now - Duration::hours(48)),
        latest_version: Some("0.1.0".to_string()),
    };
    std::fs::create_dir_all(dir.path()).unwrap();
    std::fs::write(
        state_path(dir.path()),
        serde_json::to_string(&prior).unwrap(),
    )
    .unwrap();
    assert!(schedule_refresh(dir.path(), now, Duration::hours(24)));

    perform_refresh(dir.path(), now, || {
        Err(anyhow::anyhow!("network unreachable"))
    });

    let state = read_state(dir.path()).expect("perform_refresh always writes a state file");
    assert_eq!(state.last_checked, Some(now));
    assert_eq!(
        state.latest_version.as_deref(),
        Some("0.1.0"),
        "a failed fetch must preserve the previous latest_version"
    );
    assert!(
        !lock_path(dir.path()).exists(),
        "the lock must be released on both the failure and success paths"
    );

    assert!(schedule_refresh(dir.path(), now, Duration::hours(24)));
    perform_refresh(dir.path(), now, || {
        Version::parse("0.4.0").map_err(Into::into)
    });

    let state = read_state(dir.path()).expect("perform_refresh always writes a state file");
    assert_eq!(state.latest_version.as_deref(), Some("0.4.0"));
    assert!(!lock_path(dir.path()).exists());
}
