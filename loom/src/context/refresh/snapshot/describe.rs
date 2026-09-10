//! Human-readable rendering of `SnapshotOutcome`.

use std::time::Duration;

use super::{SnapshotAction, SnapshotOutcome};
use crate::context::local_overlay::LOCAL_PLAN_KEY;
use crate::context::refresh::{clean_generation, short_revision, SourceGraphCounters};

impl SnapshotOutcome {
    /// Render the one advisory line shared by every source-graph entry point.
    pub fn describe(&self) -> String {
        if self.action == SnapshotAction::Unavailable {
            return format!("source graph: unavailable ({})", self.reason);
        }

        let target = self.target_description();
        if self.action == SnapshotAction::Reused {
            let state = if self.overlay.is_some() {
                "generation current"
            } else if self.generation == clean_generation(&self.revision) {
                "tree clean"
            } else {
                "tree dirty"
            };
            return format!("source graph: reused {target} ({state})");
        }

        format!(
            "source graph: {} {target} ({}; {})",
            self.action.as_str(),
            describe_counters(&self.counters),
            describe_elapsed(self.elapsed)
        )
    }

    fn target_description(&self) -> String {
        match &self.overlay {
            Some((plan, stage)) if plan == LOCAL_PLAN_KEY => {
                format!("local overlay {plan}/{stage}")
            }
            Some((plan, stage)) => format!("stage overlay {plan}/{stage}"),
            None => format!("base {}", short_revision(&self.revision)),
        }
    }
}

fn describe_counters(counters: &SourceGraphCounters) -> String {
    let mut parts = Vec::new();
    if counters.files_parsed > 0 {
        parts.push(format!("{} parsed", counters.files_parsed));
    }
    if counters.files_reused > 0 {
        parts.push(format!("{} reused", counters.files_reused));
    }
    if counters.files_deleted > 0 {
        parts.push(format!("{} deleted", counters.files_deleted));
    }
    if parts.is_empty() {
        parts.push("0 parsed".to_string());
    }
    parts.join(", ")
}

fn describe_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs_f64();
    if seconds >= 10.0 {
        format!("{seconds:.1}s")
    } else {
        format!("{seconds:.2}s")
    }
}
