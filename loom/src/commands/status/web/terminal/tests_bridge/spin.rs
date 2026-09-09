//! Proves the backpressure gate in `bridge::poll_ready` narrows the mask it
//! asks `poll` for, not just the read it skips: a gate that stops reading
//! while still requesting `POLLIN` makes `poll` return immediately forever,
//! burning a core with output identical to a correctly blocked bridge. Every
//! other test in this file asserts on bytes, close codes, or timing budgets
//! that a spin still satisfies - none of them can see this.
use std::os::unix::thread::JoinHandleExt;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use tungstenite::Message;

use super::super::protocol::{Mode, CLOSE_SERVER_STOPPING};
use super::{gating_flood, join_within, skip_bridge_test, start_bridge, wait_for_close};

/// A correctly gated bridge thread spends on the order of a millisecond of
/// CPU time over a full second of wall clock (a handful of blocking `poll`
/// calls and atomic loads); a spinning one spends on the order of a full
/// second, since `poll` never blocks. This sits in the wide gap between the
/// two, so it stays decisive even under `scripts/flake-check.sh`'s eight
/// concurrent CPU spinners: contention only delays when the bridge thread
/// gets scheduled, it does not add to a sleeping thread's own accounted CPU
/// time.
const SPIN_THRESHOLD: Duration = Duration::from_millis(200);

#[cfg(target_os = "linux")]
#[test]
fn bridge_sleeps_while_backpressure_is_engaged() {
    if skip_bridge_test("bridge_sleeps_while_backpressure_is_engaged") {
        return;
    }
    // A child that never reads its stdin: the PTY's own buffer fills almost
    // immediately, so `pending` only ever grows once input starts arriving.
    let mut fixture = start_bridge("sleep 30", Mode::Control);
    // `JoinHandle` does not expose a `pthread_t` needed here on its own, but
    // this stable extension trait does, without disturbing the handle we
    // still join at the end.
    let tid = fixture.server.as_pthread_t();

    // Bytes past both `MAX_PENDING_INPUT` and tungstenite's read buffer are
    // what stay unread in the kernel's socket queue once the gate shuts, and
    // that queue is the condition an unnarrowed `POLLIN` mask spins on. See
    // `gating_flood` for why the shape matters as much as the size.
    for chunk in gating_flood().chunks(32 * 1024) {
        fixture
            .socket
            .send(Message::binary(chunk.to_vec()))
            .expect("send flooded terminal input");
    }
    // Let the bridge thread drain everything it is willing to onto `pending`
    // and settle into the gated, poll-timeout-bound state before timing it.
    thread::sleep(Duration::from_millis(500));

    let before = thread_cpu_time(tid);
    thread::sleep(Duration::from_secs(1));
    let after = thread_cpu_time(tid);
    let spent = after.saturating_sub(before);
    assert!(
        spent < SPIN_THRESHOLD,
        "bridge thread burned {spent:?} of CPU time in one second while backpressure \
         should have kept it asleep between 50 ms poll timeouts"
    );

    fixture.running.store(false, Ordering::SeqCst);
    assert_eq!(
        wait_for_close(&mut fixture.socket, Duration::from_secs(3)),
        Some(CLOSE_SERVER_STOPPING)
    );
    join_within(fixture.server, Duration::from_secs(3));
}

/// The bridge thread's own consumed CPU time, read via its own CPU-time
/// clock rather than wall time or `RUSAGE_SELF`: both would fold in every
/// other thread and process on the box, which is exactly the noise
/// `scripts/flake-check.sh`'s background spinners would add.
#[cfg(target_os = "linux")]
fn thread_cpu_time(tid: libc::pthread_t) -> Duration {
    let mut clockid: libc::clockid_t = 0;
    // SAFETY: `tid` names a thread the caller's `JoinHandle` keeps alive for
    // the duration of this call, and `clockid` is a valid `clockid_t` slot.
    let rc = unsafe { libc::pthread_getcpuclockid(tid, &mut clockid) };
    assert_eq!(rc, 0, "pthread_getcpuclockid failed with errno {rc}");
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid, fully-initialized `timespec` to write into.
    let rc = unsafe { libc::clock_gettime(clockid, &mut ts) };
    assert_eq!(rc, 0, "clock_gettime failed with errno {rc}");
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}
