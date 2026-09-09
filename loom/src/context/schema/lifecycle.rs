use serde::{Deserialize, Serialize};
use std::fmt;

/// Curation state of a knowledge chunk, overridable via YAML frontmatter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LifecycleState {
    /// Current and trustworthy.
    #[default]
    Active,
    /// Written but not yet reviewed.
    Draft,
    /// Known stale; retrievable but demoted.
    Deprecated,
    /// Replaced by another chunk.
    Superseded,
    /// Historical material retained outside the current retrieval set.
    Historical,
}

impl fmt::Display for LifecycleState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            LifecycleState::Active => "active",
            LifecycleState::Draft => "draft",
            LifecycleState::Deprecated => "deprecated",
            LifecycleState::Superseded => "superseded",
            LifecycleState::Historical => "historical",
        };
        f.write_str(name)
    }
}

/// Which lifecycle states a query may retrieve. `Current` is every caller's default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LifecyclePolicy {
    /// `Active` and `Draft` chunks only.
    #[default]
    Current,
    /// Every state, historical material included.
    Historical,
}

/// How a `--require-id` item is represented in the pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequiredRepresentation {
    /// The whole unit, verbatim, never truncated. Reported as unmet when it cannot fit.
    #[default]
    Full,
    /// The excerpt-bounded representation; `ContextItem::truncated` says whether it was cut.
    Compact,
}
