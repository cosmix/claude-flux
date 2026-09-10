mod adjudication;
mod cache;
mod crud;
mod format;
mod generate;
mod helpers;
mod knowledge;
mod merge;
mod merge_conflict;
mod parse;
mod recovery;
mod recovery_format;
mod recovery_parsing;
mod recovery_types;
mod retrieval;
mod section_formatters;
mod types;

#[cfg(test)]
mod tests;

// Declared here rather than nested under `tests` so `cargo test signals::tests_size`
// filters to exactly this module's tests (nesting under `tests` would make the
// path `signals::tests::tests_size`, which that filter would not match).
#[cfg(test)]
#[path = "tests_size.rs"]
mod tests_size;

// Declared here for the same reason as `tests_size` above: the stage's wiring
// check filters on `signals::tests_doctrine`, and a module nested under
// `tests` would make the path `signals::tests::tests_doctrine`, which that
// filter silently misses - exit 0, zero tests run.
#[cfg(test)]
#[path = "tests_doctrine.rs"]
mod tests_doctrine;
#[cfg(test)]
#[path = "tests_doctrine_prefixes.rs"]
mod tests_doctrine_prefixes;
#[cfg(test)]
#[path = "tests_doctrine_waiting.rs"]
mod tests_doctrine_waiting;

// Re-export public types
pub use cache::SignalMetrics;
pub use recovery::generate_recovery_signal;
pub use recovery_parsing::read_recovery_signal;
pub use recovery_types::{LastHeartbeatInfo, RecoveryReason, RecoverySignalContent};
pub use types::{
    DependencyStatus, EmbeddedContext, MergeConflictSignalContent, MergeSignalContent,
    SignalContent, SignalUpdates,
};

// Re-export public functions
pub use cache::compute_hash;
pub use crud::{list_signals, read_signal, remove_signal, update_signal};
pub use format::{
    format_dependency_table, format_signal_with_metrics, format_skill_recommendations,
    FormattedSignal,
};
// Crate-internal re-exports. `mod cache` and `mod format` are private, and
// visibility is capped by path reachability, so a `pub(crate)` item inside
// them is unreachable from outside `signals/` unless it is re-exported here.
// Both of these have consumers in other modules: the session launcher writes
// the stable prefix to its own file for the prompt-cache split
// (`orchestrator::terminal::native::launch`), and the UserPromptSubmit hook
// renders its brief with the SAME untrusted-excerpt fencing rules the signal
// path uses (`commands::hook::user_prompt`) rather than a second copy.
pub use adjudication::generate_adjudication_signal;
pub(crate) use cache::stable_prefix_for;
pub(crate) use format::format_knowledge_brief;
pub use generate::{
    build_embedded_context_with_stage, generate_signal, generate_signal_with_metrics,
    generate_signal_with_skills, DEFAULT_MAX_SKILL_RECOMMENDATIONS,
};
pub use knowledge::generate_knowledge_signal;
pub use merge::{find_live_merge_session_for_stage, generate_merge_signal, read_merge_signal};
pub use merge_conflict::{generate_merge_conflict_signal, read_merge_conflict_signal};
