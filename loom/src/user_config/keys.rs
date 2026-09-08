//! Typed registry of `~/.loom/config.toml` keys.
//!
//! The single validator for `loom config -k <key> [<value>]`, and the surface
//! a later TUI's commit path and a later update-check lookup share. Deliberately
//! small: a new key is a new [`KeySpec`] in [`KEYS`] plus a matching field on
//! [`crate::user_config::UserConfig`] — the registry names and validates a
//! key, [`crate::user_config::UserConfig`] owns what it resolves to.

use anyhow::{bail, Result};

/// The TOML value shape a [`KeySpec`] accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// `true` / `false`.
    Bool,
    /// A non-negative integer.
    U32,
    /// One of a fixed set of string variants.
    Enum(&'static [&'static str]),
}

/// One entry in the user config's typed key registry: a dotted CLI key, the
/// `[section].field` it maps to in `~/.loom/config.toml`, and the value shape
/// it accepts.
#[derive(Debug)]
pub struct KeySpec {
    /// Dotted key as typed on the CLI, e.g. `"update.check_interval_hours"`.
    pub name: &'static str,
    /// The `[section]` this key lives under.
    pub section: &'static str,
    /// The key within that section.
    pub field: &'static str,
    /// The value shape this key accepts.
    pub kind: ValueKind,
    /// One-line description for `--list`/help output.
    pub help: &'static str,
}

/// Every key `loom config` knows about, in the order `--list`/`--print`
/// render them. The default for each key lives on
/// [`crate::user_config::UserConfig`]'s getters, not here — one source of
/// truth per default.
pub const KEYS: &[KeySpec] = &[
    KeySpec {
        name: "update.check",
        section: "update",
        field: "check",
        kind: ValueKind::Bool,
        help: "Whether loom checks for updates on startup",
    },
    KeySpec {
        name: "update.check_interval_hours",
        section: "update",
        field: "check_interval_hours",
        kind: ValueKind::U32,
        help: "Hours between update checks",
    },
    KeySpec {
        name: "terminal.backend",
        section: "terminal",
        field: "backend",
        kind: ValueKind::Enum(&["native", "tmux"]),
        help: "Terminal backend loom run defaults to",
    },
    KeySpec {
        name: "context.ceiling_tokens",
        section: "context",
        field: "ceiling_tokens",
        kind: ValueKind::U32,
        help: "Default context ceiling, in resident tokens, for a stage's agent session",
    },
    KeySpec {
        name: "pressure.claude_model",
        section: "pressure",
        field: "claude_model",
        kind: ValueKind::Enum(crate::claude::CLAUDE_MODELS),
        help: "Claude model loom pressure uses for the /pressure step",
    },
    KeySpec {
        name: "pressure.codex_model",
        section: "pressure",
        field: "codex_model",
        kind: ValueKind::Enum(crate::codex::CODEX_MODELS),
        help: "Codex model loom pressure uses for the $pressure step",
    },
    KeySpec {
        name: "pressure.address_model",
        section: "pressure",
        field: "address_model",
        kind: ValueKind::Enum(crate::claude::CLAUDE_MODELS),
        help: "Claude model loom pressure uses for the /address reconciliation step",
    },
];

/// The spec for `name`, or an error listing every valid key.
pub fn spec(name: &str) -> Result<&'static KeySpec> {
    KEYS.iter().find(|k| k.name == name).ok_or_else(|| {
        let valid: Vec<&str> = KEYS.iter().map(|k| k.name).collect();
        anyhow::anyhow!(
            "unknown user config key {name:?}; valid keys: {}",
            valid.join(", ")
        )
    })
}

impl KeySpec {
    /// Parse an operator-supplied string into the TOML value this key holds,
    /// erroring with the key name, the offending text and the expected type.
    pub fn parse(&self, raw: &str) -> Result<toml_edit::Value> {
        match self.kind {
            ValueKind::Bool => raw
                .parse::<bool>()
                .map(toml_edit::Value::from)
                .map_err(|_| {
                    anyhow::anyhow!(
                        "{}: {raw:?} is not a bool (expected true or false)",
                        self.name
                    )
                }),
            ValueKind::U32 => raw
                .parse::<u32>()
                .map(|v| toml_edit::Value::from(v as i64))
                .map_err(|_| {
                    anyhow::anyhow!(
                        "{}: {raw:?} is not a u32 (expected a non-negative integer)",
                        self.name
                    )
                }),
            ValueKind::Enum(variants) => {
                if variants.contains(&raw) {
                    Ok(toml_edit::Value::from(raw))
                } else {
                    bail!(
                        "{}: {raw:?} is not one of the expected values: {}",
                        self.name,
                        variants.join(", ")
                    )
                }
            }
        }
    }
}
