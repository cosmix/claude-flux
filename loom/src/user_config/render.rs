//! Introspection and rendering for [`UserConfig`]: resolving a single key's
//! value + origin for `loom config -k`/`--list`, and rendering the whole
//! resolved config as TOML for `loom config --print`.
//!
//! Split out of `mod.rs` to keep that file under Rule 17's 400-line limit —
//! adding the `pressure.*` keys pushed the combined getters/parsing/rendering
//! surface over budget. `mod.rs` keeps the struct, the loaders, and the
//! resolved getters; this file owns everything that turns those getters into
//! operator-facing text, plus [`UserConfig::origin_of`], which `pressure.rs`
//! and `models.rs` reuse for their own sections' keys.

use super::{KeySpec, Origin, UserConfig};

impl UserConfig {
    /// The rendered value and origin for `spec`, for `loom config --list` and
    /// `loom config -k <key>`.
    pub fn value_of(&self, spec: &KeySpec) -> (String, Origin) {
        if let Some(resolved) = self.pressure_value_of(spec) {
            return resolved;
        }
        if let Some(resolved) = self.models_value_of(spec) {
            return resolved;
        }
        match spec.name {
            "update.check" => (
                self.update_check().to_string(),
                self.origin_of(self.update_check),
            ),
            "update.check_interval_hours" => (
                self.update_check_interval_hours().to_string(),
                self.origin_of(self.update_check_interval_hours),
            ),
            "terminal.backend" => (
                self.terminal_backend().to_string(),
                self.origin_of(self.terminal_backend),
            ),
            "context.ceiling_tokens" => (
                self.context_ceiling_tokens().to_string(),
                self.origin_of(self.context_ceiling_tokens),
            ),
            other => unreachable!("value_of: {other} is not in keys::KEYS"),
        }
    }

    pub(super) fn origin_of<T>(&self, set: Option<T>) -> Origin {
        if set.is_some() {
            Origin::Set
        } else {
            Origin::Default
        }
    }

    /// The fully resolved config (every key, its effective value) as TOML,
    /// sections in `[context]`, `[models]`, `[pressure]`, `[terminal]`,
    /// `[update]` order — the shape `loom config --print` renders. Composed
    /// from the private `models_toml` and `pressure_toml` helpers rather
    /// than growing this `format!` further.
    pub fn to_toml_string(&self) -> String {
        format!(
            "[context]\nceiling_tokens = {}\n\n{}\n{}\n[terminal]\nbackend = \"{}\"\n\n[update]\ncheck = {}\ncheck_interval_hours = {}\n",
            self.context_ceiling_tokens(),
            self.models_toml(),
            self.pressure_toml(),
            self.terminal_backend(),
            self.update_check(),
            self.update_check_interval_hours(),
        )
    }
}
