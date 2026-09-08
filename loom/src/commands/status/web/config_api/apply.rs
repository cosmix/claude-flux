//! Applying one `POST /api/config` update to the tier it names.
//!
//! Both tiers write through their own module's locked read-modify-write
//! (`user_config::write` / `fs::work_dir::update_config`) rather than a bare
//! read/modify/write here: a dashboard write races the daemon, a shell hook and
//! any `loom config` the operator has open, and a lost update in a config file
//! is silent.

use anyhow::{anyhow, Result};

use crate::fs::work_dir::{insert_key, remove_key, update_config};
use crate::user_config;
use crate::user_config::keys::KeySpec;

use super::workspace::Workspace;
use super::Scope;

/// Write or clear `spec` at `scope`, reporting what that scope resolved to
/// before and after.
///
/// `value` is `None` for an unset. `workspace` must be present for
/// [`Scope::Project`]; the caller has already turned its absence into a 409.
pub(super) fn apply(
    scope: Scope,
    spec: &KeySpec,
    value: Option<toml_edit::Value>,
    workspace: Option<&Workspace>,
) -> Result<(String, String)> {
    match scope {
        Scope::User => match value {
            Some(value) => user_config::set(spec, value),
            None => user_config::unset(spec),
        },
        Scope::Project => {
            let workspace =
                workspace.ok_or_else(|| anyhow!("no workspace to write the project scope to"))?;
            project(workspace, spec, value)
        }
    }
}

/// Edit one key of `.loom/work/config.toml`, capturing the project tier's
/// resolved value on either side of the edit inside the same lock hold — the
/// same discipline `user_config::write::locked_edit` keeps, and for the same
/// reason: a pair read outside the lock can describe a state that never
/// existed.
fn project(
    workspace: &Workspace,
    spec: &KeySpec,
    value: Option<toml_edit::Value>,
) -> Result<(String, String)> {
    let root = workspace.root().to_path_buf();
    let mut old_new: Option<(String, String)> = None;
    update_config(&root, |doc| {
        let old = Workspace::from_document(&root, doc)?.value_of(spec)?;
        match value {
            Some(value) => insert_key(doc, spec.section, spec.field, value)?,
            None => remove_key(doc, spec.section, spec.field),
        }
        let new = Workspace::from_document(&root, doc)?.value_of(spec)?;
        old_new = Some((old, new));
        Ok(())
    })?;
    old_new.ok_or_else(|| anyhow!("update_config returned without a captured value"))
}
