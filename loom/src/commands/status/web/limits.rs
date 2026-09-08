//! Admission control for dashboard connection threads.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Connection threads allowed in flight at once.
///
/// Each one costs a 16 KiB request-head buffer plus its stack, and nothing
/// bounds how many clients a local process may open, so the accept loop turns
/// clients away past this point instead of spawning without limit.
pub(super) const MAX_CONNECTIONS: usize = 64;

/// How many of [`MAX_CONNECTIONS`] may be WebSocket subscriptions at once.
///
/// Every HTTP response carries `Connection: close`, so those slots turn over
/// in milliseconds, but a `/ws` subscription holds its one for as long as the
/// tab stays open. Sharing a single pool would let open tabs starve the page
/// they were opened from: with the pool full, the next `GET /` is answered
/// with a 503 whose explanation only ever reaches a client that already has
/// the dashboard loaded. The difference between the two constants is the
/// reserve that non-upgrade requests always draw on.
pub(super) const MAX_WEBSOCKETS: usize = 48;

/// Browser terminals allowed in flight at once.
pub(super) const MAX_TERMINALS: usize = 8;

/// The two lanes a connection can occupy a slot in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Lane {
    /// Held for the whole life of a connection thread, upgraded or not.
    Connection,
    /// Held additionally, for the life of the subscription, by `/ws`.
    WebSocket,
    /// Held additionally by one browser terminal.
    Terminal,
}

impl Lane {
    fn capacity(self) -> usize {
        match self {
            Self::Connection => MAX_CONNECTIONS,
            Self::WebSocket => MAX_WEBSOCKETS,
            Self::Terminal => MAX_TERMINALS,
        }
    }

    fn counter(self, limits: &Limits) -> &AtomicUsize {
        match self {
            Self::Connection => &limits.connections,
            Self::WebSocket => &limits.websockets,
            Self::Terminal => &limits.terminals,
        }
    }
}

/// The server's live occupancy of both lanes, shared by every connection.
#[derive(Debug, Default)]
pub(super) struct Limits {
    connections: AtomicUsize,
    websockets: AtomicUsize,
    terminals: AtomicUsize,
}

impl Limits {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(super) fn connection_count(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(super) fn terminal_count(&self) -> usize {
        self.terminals.load(Ordering::SeqCst)
    }
}

/// Reserve a terminal lane slot for a browser terminal.
pub(super) fn acquire_terminal_slot(limits: &Arc<Limits>) -> Option<Slot> {
    Slot::acquire(limits, Lane::Terminal)
}

/// A reserved slot in one lane, released when the holder is dropped.
///
/// The release runs in `Drop`, so a panicking connection thread returns its
/// slots as well.
pub(super) struct Slot {
    limits: Arc<Limits>,
    lane: Lane,
}

impl Slot {
    /// Reserve a slot in `lane`, or return `None` once it is full.
    ///
    /// The increment and the capacity test are one read-modify-write, so two
    /// threads racing at the boundary cannot both see room: the loser reads a
    /// count at or above the cap and rolls its own increment back.
    pub(super) fn acquire(limits: &Arc<Limits>, lane: Lane) -> Option<Self> {
        let counter = lane.counter(limits);
        let taken = counter.fetch_add(1, Ordering::SeqCst);
        if taken >= lane.capacity() {
            counter.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Self {
            limits: limits.clone(),
            lane,
        })
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.lane
            .counter(&self.limits)
            .fetch_sub(1, Ordering::SeqCst);
    }
}
