//! Per-worktree spool for stage-control requests a sandboxed agent cannot send.
//!
//! `loom stage block` and `loom stage dispute-criteria` are the two sanctioned
//! moves for an agent that finds it cannot proceed, and both are daemon-owned:
//! `.loom/work/stages/` is read-only from a worktree, so only the daemon can
//! apply the transition. The CLI therefore sends an RPC over
//! `.loom/work/orchestrator.sock` — which the caller that most needs those commands
//! cannot do. A sandboxed process is denied AF_UNIX outright (see
//! `daemon/server/core.rs`), and the loom binary can never be configured to run
//! outside the host sandbox (`sandbox/settings/policy.rs` rejects
//! `sandbox.excluded_commands` by name). Both routes to the daemon are shut.
//!
//! This module is the third route, modelled directly on [`crate::fs::memory`]'s
//! spool, which exists for the same reason on the same path: the request is
//! appended to `<worktree_root>/.loom/stage-request-spool.jsonl`, inside the
//! worktree's own write boundary and needing no new sandbox grant. The daemon
//! runs outside the sandbox and drains it on its poll loop
//! (`orchestrator/core/spool_drain.rs`).
//!
//! Spooling DEFERS to the daemon's authority rather than bypassing it. A
//! direct `.loom/work/stages/<id>.md` write would decide the transition itself,
//! which is exactly what the read-only mount forbids; a spooled request is
//! still decided by the daemon, still refusable by it, just later.
//!
//! Deliberately absent from the payload: a stage id. Attribution of a drained
//! request comes from *which worktree* the daemon drained it from, not from
//! anything the request claims about itself — a sandboxed agent cannot forge
//! the worktree it is running in, but it could trivially forge a field. That
//! is what stands in for the peer-identity check the socket path performs when
//! there is no connection to identify.
//!
//! [`drain_requests`] applies each request through the daemon's own handlers
//! rather than reimplementing the transitions, so the spooled path and the RPC
//! path cannot drift apart on what a block or a dispute actually does. The
//! resulting `fs` → `daemon::server` dependency is deliberate for that reason.

mod apply;
mod spool;
mod types;

pub use apply::drain_requests;
#[cfg(test)]
pub(crate) use spool::read_pending;
pub use spool::{
    append_to_spool, spool_path, spool_target_from_cwd, DrainOutcome, SPOOL_MAX_BYTES,
    SPOOL_RELPATH,
};
pub use types::StageRequest;
