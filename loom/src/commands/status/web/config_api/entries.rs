//! Projecting the user-config key registry onto the `/api/config` wire shape.
//!
//! Everything here is derived from [`KEYS`]; there is no second table of key
//! names, help text or value kinds to drift out of step with the registry. A
//! key added to `keys.rs` appears on the dashboard with no edit in this file,
//! and `config_api::tests` asserts exactly that.

use anyhow::Result;

use crate::user_config::keys::{KeySpec, KEYS};
use crate::user_config::{Origin, UserConfig};

use super::wire::{ConfigEntry, ConfigKind, EffectiveValue, ScopeValue, Source};
use super::workspace::{self, Workspace};

/// Whether `spec` accepts a project-scope write.
///
/// Asks the resolver rather than keeping a list of key names here: a key is
/// project-scoped exactly when `crate::fs::work_dir` already resolves a
/// workspace section for it, and [`workspace::backs`] is that same fact read
/// off the one table that implements it.
pub(super) fn project_scoped(spec: &KeySpec) -> bool {
    workspace::backs(spec)
}

/// Every registry key, in [`KEYS`] order.
pub(super) fn all(user: &UserConfig, workspace: Option<&Workspace>) -> Result<Vec<ConfigEntry>> {
    KEYS.iter()
        .map(|spec| entry(spec, user, workspace))
        .collect()
}

/// One registry key, resolved across both tiers.
pub(super) fn entry(
    spec: &KeySpec,
    user: &UserConfig,
    workspace: Option<&Workspace>,
) -> Result<ConfigEntry> {
    let (value, origin) = user.value_of(spec);
    let user_scope = ScopeValue {
        value,
        set: origin == Origin::Set,
    };
    let workspace = workspace.filter(|_| project_scoped(spec));
    let project = match workspace {
        Some(workspace) => Some(ScopeValue {
            value: workspace.value_of(spec)?,
            set: workspace.has_key(spec),
        }),
        None => None,
    };
    // Section-level: a present-but-keyless section still wins whole.
    // Key-level: only a present key wins, so an omitted one falls through.
    let project_wins = workspace.is_some_and(|workspace| workspace.shadows(spec));
    Ok(ConfigEntry {
        name: spec.name.to_owned(),
        help: spec.help.to_owned(),
        kind: ConfigKind::from(&spec.kind),
        scopes: scopes(spec),
        default: built_in(spec),
        effective: effective(&user_scope, project.as_ref().filter(|_| project_wins)),
        user: user_scope,
        project,
    })
}

/// The built-in `spec` resolves to with neither file setting it.
///
/// Read through the resolved getters against an all-`None` [`UserConfig`],
/// which is where each default is spelled out — never a second table of
/// default values here, which would be one more thing to drift. `pub(super)`
/// because `workspace`'s key-level resolution needs this exact value for a
/// key a present section omits, rather than writing the expression a second
/// time.
pub(super) fn built_in(spec: &KeySpec) -> String {
    UserConfig::default().value_of(spec).0
}

fn scopes(spec: &KeySpec) -> Vec<String> {
    let mut scopes = vec!["user".to_owned()];
    if project_scoped(spec) {
        scopes.push("project".to_owned());
    }
    scopes
}

/// The tier in force: the workspace section when one is present, else the user
/// file when it sets the key, else loom's built-in.
fn effective(user: &ScopeValue, project: Option<&ScopeValue>) -> EffectiveValue {
    match project {
        Some(project) => EffectiveValue {
            value: project.value.clone(),
            source: Source::Project,
        },
        None if user.set => EffectiveValue {
            value: user.value.clone(),
            source: Source::User,
        },
        None => EffectiveValue {
            value: user.value.clone(),
            source: Source::Default,
        },
    }
}
