//! TOML value parsing and validation for [`super::RetrievalConfig::apply`].
//!
//! Split out of `config.rs` to keep it under the file line limit — these are
//! implementation details of the config loader, private to it, so nothing
//! here needs more than `pub(super)` visibility.

use super::MAX_BUDGET;
use std::path::{Component, Path};
use toml::Value;

/// Read a ratio or factor into `(0.0, 1.0]`.
///
/// A non-finite, zero or negative value falls back to `default` rather than to
/// the clamp bound: `test_path_factor = 0` reads as "turn this off", but zeroing
/// a multiplier applied to a total score erases the score of every node it
/// touches, which is not a tuning outcome anyone can have wanted. Only the
/// upper end is a true clamp.
pub(super) fn ratio(value: &Value, key: &str, default: f32) -> f32 {
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
pub(super) fn prior(value: &Value, key: &str, default: f32) -> f32 {
    let Some(raw) = number(value, key) else {
        return default;
    };
    if !raw.is_finite() || raw < 0.0 {
        return default;
    }
    raw
}

/// Read a token or byte budget, clamped into `[min, MAX_BUDGET]`.
///
/// `min` is the caller's `MIN_BUDGET_TOKENS` for a token budget and
/// `MIN_PAYLOAD_BYTES` for `max_payload_bytes` — the two floors police
/// different units and must not be conflated. The upper bound is
/// `super::MAX_BUDGET` directly: every budget in this config shares one
/// ceiling, so there is nothing for a caller to pass in.
///
/// Clamped in `i64` before the cast: `-1 as usize` is `usize::MAX`, which would
/// clamp to the *maximum* budget and turn a nonsense value into the most
/// expensive possible setting.
///
/// A clamp is logged like a wrong TYPE is: `prompt_budget_tokens = 150` is a
/// legal integer that silently becomes something else, and an operator who
/// tuned a budget down and saw no effect has no other way to find out why.
pub(super) fn budget(value: &Value, key: &str, default: usize, min: usize) -> usize {
    let Some(raw) = integer(value, key) else {
        return default;
    };
    let clamped = raw.clamp(min as i64, MAX_BUDGET as i64);
    if clamped != raw {
        tracing::warn!(
            key,
            raw,
            clamped,
            "clamping an out-of-range [retrieval] value"
        );
    }
    clamped as usize
}

/// Read a count, clamped to at least 1. A zero count would make its rule
/// vacuous rather than strict — no term long enough, no chunk rare enough.
pub(super) fn count(value: &Value, key: &str, default: usize) -> usize {
    let Some(raw) = integer(value, key) else {
        return default;
    };
    raw.max(1) as usize
}

/// Read a duration in seconds. `0` is legal and means "no delay"; a negative
/// value is not expressible as a duration and falls back to `default`.
pub(super) fn seconds(value: &Value, key: &str, default: u64) -> u64 {
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
pub(super) fn prose_roots(value: &Value, key: &str) -> Option<Vec<String>> {
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
