//! The write half of `~/.loom/config.toml`: [`set`] and [`unset`].
//!
//! Split out of `mod.rs` to keep that file under Rule 17's 400-line limit once
//! the unset path joined the set path. `mod.rs` keeps the struct, the loaders
//! and the resolved getters; this file owns everything that mutates the file,
//! and re-exports through `mod.rs` so callers still say
//! `crate::user_config::set`.
//!
//! Both paths run through [`locked_edit`], which is the ONLY thing here allowed
//! to create `~/.loom/` — see the module docs on `mod.rs` for why the read
//! paths must not.

use std::path::Path;

use anyhow::{Context, Result};
use toml_edit::DocumentMut;

use super::{parse_document, write_config_path, KeySpec};

/// Read-modify-write `spec`'s key in `~/.loom/config.toml`, creating the
/// section if absent. Comments and unrelated keys are preserved verbatim
/// (`toml_edit::DocumentMut`).
///
/// Returns the rendered value `spec` resolved to just before and just after the
/// write, both captured inside the same locked read-modify-write. A caller that
/// instead re-read the config before and after this call
/// (`commands::config::set_key` used to) could report a value written by a
/// concurrent `set` that raced it — `loom` is invoked concurrently from shell
/// hooks, so that race is real.
pub fn set(spec: &KeySpec, value: toml_edit::Value) -> Result<(String, String)> {
    set_in(&write_config_path()?, spec, value)
}

/// [`set`] factored over an explicit path so tests can exercise the
/// read-modify-write behavior against a temp file instead of the real
/// `~/.loom/config.toml`.
pub(crate) fn set_in(
    path: &Path,
    spec: &KeySpec,
    value: toml_edit::Value,
) -> Result<(String, String)> {
    locked_edit(path, spec, |doc| {
        let table = doc
            .entry(spec.section)
            .or_insert(toml_edit::table())
            .as_table_like_mut()
            .ok_or_else(|| {
                anyhow::anyhow!("[{}] in {} is not a table", spec.section, path.display())
            })?;
        table.insert(spec.field, toml_edit::Item::Value(value));
        Ok(())
    })
}

/// Remove `spec`'s key from `~/.loom/config.toml`, reverting it to loom's
/// built-in default. Absent already, this is a no-op that still reports the
/// resolved value either side.
///
/// Returns the same before/after pair as [`set`], captured under the same lock
/// and for the same reason.
pub fn unset(spec: &KeySpec) -> Result<(String, String)> {
    unset_in(&write_config_path()?, spec)
}

/// [`unset`] against an explicit path, the test seam [`set_in`] is.
///
/// An emptied section is LEFT in place, unlike the workspace config's
/// [`crate::fs::work_dir::remove_key`]. The two differ because their fallbacks
/// do: this file resolves key by key, so a keyless `[pressure]` changes
/// nothing an operator can observe, whereas a keyless workspace section still
/// wins whole and would keep shadowing this tier. Removing it here would buy
/// nothing and would discard the comments attached to the section header, which
/// [`set_in`] promises to preserve.
pub(crate) fn unset_in(path: &Path, spec: &KeySpec) -> Result<(String, String)> {
    locked_edit(path, spec, |doc| {
        if let Some(table) = doc
            .get_mut(spec.section)
            .and_then(|item| item.as_table_like_mut())
        {
            table.remove(spec.field);
        }
        Ok(())
    })
}

/// Apply `edit` to `~/.loom/config.toml` under the exclusive
/// parent-directory lock, reporting what `spec` resolved to on either side of
/// it.
///
/// The whole sequence — the before-read, the edit, the after-read and the
/// crash-atomic write — happens inside one [`crate::fs::locking::locked_update`]
/// call, so the pair returned always describes a state this process actually
/// wrote, never one a concurrent writer interleaved.
fn locked_edit<F>(path: &Path, spec: &KeySpec, edit: F) -> Result<(String, String)>
where
    F: FnOnce(&mut DocumentMut) -> Result<()>,
{
    let mut old_new: Option<(String, String)> = None;
    crate::fs::locking::locked_update(path, |existing| {
        let old = resolved(&existing, spec);
        let mut doc: DocumentMut = existing.parse().with_context(|| {
            format!(
                "refusing to rewrite unparseable user config at {}",
                path.display()
            )
        })?;
        edit(&mut doc)?;
        let updated = doc.to_string();
        old_new = Some((old, resolved(&updated, spec)));
        Ok(updated)
    })?;
    old_new.ok_or_else(|| anyhow::anyhow!("locked_edit: locked_update returned without a value"))
}

/// What `spec` resolves to in `text`, built-in default included. An
/// unparseable file renders as all-defaults here rather than failing: the
/// caller is about to refuse the write for that same reason, with a message
/// naming the file.
fn resolved(text: &str, spec: &KeySpec) -> String {
    parse_document(text).unwrap_or_default().value_of(spec).0
}
