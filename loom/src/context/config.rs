//! The one tunables surface for retrieval.
//!
//! Every threshold the ranking, packing and hook paths depend on lives here as
//! a field of [`RetrievalConfig`], loaded once per retrieval and threaded down
//! as a `&RetrievalConfig` parameter. No global, no `OnceLock`, no re-read
//! partway through a pipeline: two callers in one process must be able to
//! retrieve against different roots without one silently inheriting the
//! other's tunables.
//!
//! ## The file
//!
//! `<main project root>/.loom/config.toml`, table `[retrieval]`, every key
//! optional:
//!
//! ```toml
//! [retrieval]
//! stop_df_ratio = 0.10
//! stop_rescue_max_ratio = 0.25
//! min_query_token_len = 3
//! df_ident_max = 5
//! test_path_factor = 0.4
//! min_knowledge_terms = 2
//! knowledge_curated_prior = 5.0
//! prompt_budget_tokens = 1500
//! stage_brief_budget_tokens = 3000
//! max_payload_bytes = 16384
//! keep_base_graphs = 3
//! reconcile_debounce_secs = 600
//! reconcile_stale_lock_secs = 1800
//! prose_roots = ["doc"]
//! ```
//!
//! **This is not `.loom/work/config.toml`.** That file is loom's plan and
//! orchestration state (see [`crate::fs::work_dir`]) and has nothing to do with
//! retrieval; the two are different files, in different directories, read by
//! different code. Do not merge them.
//!
//! ## Never an error
//!
//! [`RetrievalConfig::load`] returns [`RetrievalConfig::default`] for an absent
//! file, an unreadable file, a file that is not TOML, and a `[retrieval]` value
//! that is not a table. Retrieval is on the hot path of signal generation and
//! the prompt hook, where a hard failure costs an agent its Knowledge Brief; a
//! typo in an optional tuning file must never be able to do that. Out-of-range
//! values clamp or fall back per field rather than rejecting the file, so one
//! bad key cannot discard twelve good ones.

use crate::context::schema::BRIEF_FRAME_TOKENS;
use std::path::{Component, Path};
use toml::{Table, Value};

/// Path of the config file, relative to the main project root.
const CONFIG_RELPATH: &str = ".loom/config.toml";

/// Smallest accepted token/byte budget.
///
/// Every pack this crate renders is charged [`BRIEF_FRAME_TOKENS`] before a
/// single item is considered (see `ContextPack::recompute_estimate`), so a
/// budget at or below that frame cost cannot even pay for the frame, let
/// alone one item — honouring it would silently produce empty output
/// forever. Defined off `BRIEF_FRAME_TOKENS` rather than as an independent
/// round number so the two constants cannot drift apart again.
pub const MIN_BUDGET_TOKENS: usize = BRIEF_FRAME_TOKENS + 128;

// If `BRIEF_FRAME_TOKENS` ever grows to meet or pass this floor, the floor
// stops leaving any room for an item and the whole point of deriving it from
// the frame cost is lost — fail the build rather than reopen that hole.
const _: () = assert!(MIN_BUDGET_TOKENS > BRIEF_FRAME_TOKENS);

/// Largest accepted token/byte budget.
const MAX_BUDGET: usize = 100_000;

/// Tunables for one retrieval, threaded through the pipeline by reference.
#[derive(Debug, Clone, PartialEq)]
pub struct RetrievalConfig {
    /// A query term is corpus-ubiquitous, and dropped, when its document
    /// frequency exceeds `corpus_size * stop_df_ratio`.
    pub stop_df_ratio: f32,
    /// Ceiling, as a fraction of the corpus, on the document frequency of a
    /// term the rescue floor may put back when stopwording dropped the WHOLE
    /// query. Deliberately looser than `stop_df_ratio` — below it the rescue
    /// is vacuous, since nothing dropped for ubiquity could ever clear it.
    pub stop_rescue_max_ratio: f32,
    /// Shorter query terms are dropped unless backticked in the raw prompt.
    pub min_query_token_len: usize,
    /// Highest document frequency at which a term still counts as corpus-rare,
    /// and so may admit an exact-match rung on its own.
    pub df_ident_max: usize,
    /// Multiplier applied to a source node's total score when its file is a
    /// test file. Ordering pressure, not exclusion.
    pub test_path_factor: f32,
    /// Distinct surviving query terms an item must match to satisfy the
    /// hook's emit floor on its own.
    ///
    /// The name predates the floor covering both channels — it now gates a
    /// source node exactly the same way it gates a knowledge chunk (see
    /// `commands::hook::user_prompt_compose::clears_emit_floor`), because a
    /// knowledge-only floor blacks out retrieval entirely in a checkout with a
    /// mapped source graph but no curated knowledge tree. Left unrenamed
    /// deliberately: every reference below and the `[retrieval]` TOML key
    /// would need to move together with no alias, for a name change that
    /// changes no behavior.
    pub min_knowledge_terms: usize,
    /// Additive score prior for curated knowledge over indexed prose. A score
    /// increment, not a ratio — it is deliberately greater than 1.
    pub knowledge_curated_prior: f32,
    /// Token budget for a pack retrieved by the user-prompt hook.
    pub prompt_budget_tokens: usize,
    /// Token budget for a pack rendered into a stage spawn brief.
    pub stage_brief_budget_tokens: usize,
    /// Largest hook payload emitted, in bytes.
    pub max_payload_bytes: usize,
    /// Base graph files retained besides the ones `state.json` references.
    pub keep_base_graphs: usize,
    /// Minimum seconds between background reconcile attempts. `0` means every
    /// attempt is allowed to proceed.
    pub reconcile_debounce_secs: u64,
    /// Age at which a reconcile lock may be taken over, in seconds.
    pub reconcile_stale_lock_secs: u64,
    /// Project-relative directories whose prose is indexed into the structural
    /// catalog. An explicitly empty list means "index no prose".
    pub prose_roots: Vec<String>,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            stop_df_ratio: 0.10,
            stop_rescue_max_ratio: 0.25,
            min_query_token_len: 3,
            df_ident_max: 5,
            test_path_factor: 0.4,
            min_knowledge_terms: 2,
            knowledge_curated_prior: 5.0,
            prompt_budget_tokens: 1500,
            stage_brief_budget_tokens: 3000,
            max_payload_bytes: 16384,
            keep_base_graphs: 3,
            reconcile_debounce_secs: 600,
            reconcile_stale_lock_secs: 1800,
            prose_roots: vec!["doc".to_string()],
        }
    }
}

impl RetrievalConfig {
    /// Read `<project_root>/.loom/config.toml`, falling back to
    /// [`Self::default`] on every failure. Never returns an error: see the
    /// module docstring for why retrieval must not be killable by this file.
    ///
    /// `project_root` is the **main** project root — the one
    /// [`crate::context::store::ContextStore`] resolves its cache under. In a
    /// linked worktree, `.loom/` is the main repository's, so a stage running
    /// in `.worktrees/<stage-id>/` reads the same tunables the host does
    /// instead of silently falling back to defaults.
    pub fn load(project_root: &Path) -> Self {
        let path = project_root.join(CONFIG_RELPATH);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            // An absent file is the overwhelmingly common case and is not
            // worth a log line; anything else means the file exists and could
            // not be read, which the operator wants to know about.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "could not read the retrieval config; using defaults");
                return Self::default();
            }
        };

        let document: Table = match text.parse() {
            Ok(document) => document,
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "could not parse the retrieval config; using defaults");
                return Self::default();
            }
        };

        match document.get("retrieval").and_then(Value::as_table) {
            Some(table) => Self::from_retrieval_table(table),
            None => Self::default(),
        }
    }

    /// Merge a `[retrieval]` table over the defaults, key by key.
    ///
    /// Driven by what the FILE holds rather than by what the struct holds, so
    /// an unrecognized key is caught by the same `match` that dispatches the
    /// recognized ones. A separate list of known keys would be a second copy
    /// of the arms below, free to drift the moment a tunable is added.
    fn from_retrieval_table(table: &Table) -> Self {
        let mut config = Self::default();
        for (key, value) in table {
            config.apply(key, value);
        }
        config
    }

    /// Apply one `[retrieval]` key, keeping the current value — the default,
    /// since nothing else writes these — when the new one is missing, wrongly
    /// typed or out of range. One bad key costs only its own field.
    fn apply(&mut self, key: &str, value: &Value) {
        match key {
            "stop_df_ratio" => self.stop_df_ratio = ratio(value, key, self.stop_df_ratio),
            "stop_rescue_max_ratio" => {
                self.stop_rescue_max_ratio = ratio(value, key, self.stop_rescue_max_ratio);
            }
            "min_query_token_len" => {
                self.min_query_token_len = count(value, key, self.min_query_token_len);
            }
            "df_ident_max" => self.df_ident_max = count(value, key, self.df_ident_max),
            "test_path_factor" => self.test_path_factor = ratio(value, key, self.test_path_factor),
            "min_knowledge_terms" => {
                self.min_knowledge_terms = count(value, key, self.min_knowledge_terms);
            }
            "knowledge_curated_prior" => {
                self.knowledge_curated_prior = prior(value, key, self.knowledge_curated_prior);
            }
            "prompt_budget_tokens" => {
                self.prompt_budget_tokens = budget(value, key, self.prompt_budget_tokens);
            }
            "stage_brief_budget_tokens" => {
                self.stage_brief_budget_tokens = budget(value, key, self.stage_brief_budget_tokens);
            }
            "max_payload_bytes" => {
                self.max_payload_bytes = budget(value, key, self.max_payload_bytes);
            }
            "keep_base_graphs" => self.keep_base_graphs = count(value, key, self.keep_base_graphs),
            "reconcile_debounce_secs" => {
                self.reconcile_debounce_secs = seconds(value, key, self.reconcile_debounce_secs);
            }
            "reconcile_stale_lock_secs" => {
                self.reconcile_stale_lock_secs =
                    seconds(value, key, self.reconcile_stale_lock_secs);
            }
            "prose_roots" => {
                if let Some(roots) = prose_roots(value, key) {
                    self.prose_roots = roots;
                }
            }
            unknown => tracing::debug!(key = %unknown, "ignoring an unknown [retrieval] key"),
        }
    }
}

/// Read a ratio or factor into `(0.0, 1.0]`.
///
/// A non-finite, zero or negative value falls back to `default` rather than to
/// the clamp bound: `test_path_factor = 0` reads as "turn this off", but zeroing
/// a multiplier applied to a total score erases the score of every node it
/// touches, which is not a tuning outcome anyone can have wanted. Only the
/// upper end is a true clamp.
fn ratio(value: &Value, key: &str, default: f32) -> f32 {
    let Some(raw) = number(value, key) else {
        return default;
    };
    if !raw.is_finite() || raw <= 0.0 {
        return default;
    }
    raw.min(1.0)
}

/// Read an additive score prior: any finite, non-negative value is legal.
///
/// Unlike [`ratio`] this is not bounded above by 1.0 — it is added to a BM25
/// score, whose scale is unbounded, and its default is 5.0.
fn prior(value: &Value, key: &str, default: f32) -> f32 {
    let Some(raw) = number(value, key) else {
        return default;
    };
    if !raw.is_finite() || raw < 0.0 {
        return default;
    }
    raw
}

/// Read a token or byte budget, clamped into `[MIN_BUDGET_TOKENS, MAX_BUDGET]`.
///
/// Clamped in `i64` before the cast: `-1 as usize` is `usize::MAX`, which would
/// clamp to the *maximum* budget and turn a nonsense value into the most
/// expensive possible setting.
fn budget(value: &Value, key: &str, default: usize) -> usize {
    let Some(raw) = integer(value, key) else {
        return default;
    };
    raw.clamp(MIN_BUDGET_TOKENS as i64, MAX_BUDGET as i64) as usize
}

/// Read a count, clamped to at least 1. A zero count would make its rule
/// vacuous rather than strict — no term long enough, no chunk rare enough.
fn count(value: &Value, key: &str, default: usize) -> usize {
    let Some(raw) = integer(value, key) else {
        return default;
    };
    raw.max(1) as usize
}

/// Read a duration in seconds. `0` is legal and means "no delay"; a negative
/// value is not expressible as a duration and falls back to `default`.
fn seconds(value: &Value, key: &str, default: u64) -> u64 {
    let Some(raw) = integer(value, key) else {
        return default;
    };
    if raw < 0 {
        return default;
    }
    raw as u64
}

/// Read `prose_roots`, dropping every entry that could point the indexer
/// outside the project. `None` leaves the current value alone.
///
/// An absolute path, or one with a `..` component, would let a config file aim
/// the prose indexer at any directory the process can read and pull its
/// contents into a Knowledge Brief. Empty strings are dropped because
/// `Path::new("")` joins to the project root itself. A list that survives
/// filtering empty is honoured as written: it means "index no prose".
fn prose_roots(value: &Value, key: &str) -> Option<Vec<String>> {
    let Some(array) = value.as_array() else {
        log_wrong_type(key, "an array of strings");
        return None;
    };
    Some(
        array
            .iter()
            .filter_map(Value::as_str)
            .filter(|entry| is_contained_root(entry))
            .map(str::to_string)
            .collect(),
    )
}

/// True when `entry` names a directory inside the project.
fn is_contained_root(entry: &str) -> bool {
    let path = Path::new(entry);
    !entry.is_empty()
        && path.is_relative()
        && !path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
}

/// Read a value as a float, accepting a TOML integer for it — `stop_df_ratio = 1`
/// is what an operator writes when they mean `1.0`.
fn number(value: &Value, key: &str) -> Option<f32> {
    match value
        .as_float()
        .or_else(|| value.as_integer().map(|raw| raw as f64))
    {
        Some(raw) => Some(raw as f32),
        None => {
            log_wrong_type(key, "a number");
            None
        }
    }
}

/// Read a value as a TOML integer.
fn integer(value: &Value, key: &str) -> Option<i64> {
    match value.as_integer() {
        Some(raw) => Some(raw),
        None => {
            log_wrong_type(key, "an integer");
            None
        }
    }
}

/// Log a value whose TOML type is not the one its key needs.
fn log_wrong_type(key: &str, expected: &str) {
    tracing::warn!(key, expected, "ignoring a wrong-typed [retrieval] value");
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
