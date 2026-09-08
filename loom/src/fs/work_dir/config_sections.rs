//! Centralized `.loom/work/config.toml` API.
//!
//! All read/write to `.loom/work/config.toml` MUST go through this module so that:
//!   * comments and unknown keys are preserved (toml_edit, not toml::Value),
//!   * structured sub-tables (`[plan_sandbox]`) have one canonical location,
//!   * concurrent access serializes through the file lock used by other
//!     `fs/` writers when needed by callers.
//!
//! Section layout in `.loom/work/config.toml`:
//!
//!   [plan]
//!   source_path / plan_id / plan_name / base_branch
//!
//!   [plan_sandbox]   # persisted snapshot of plan-level sandbox at init time
//!
//! Section keys for the persisted plan-level config (see `read_plan_sandbox`).

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::DocumentMut;

use crate::models::session::TerminalConfig;
use crate::plan::schema::SandboxConfig;
use crate::remote_control::RemoteControlConfig;

use super::ContextConfig;

const PLAN_SANDBOX_SECTION: &str = "plan_sandbox";
const REMOTE_CONTROL_SECTION: &str = "remote_control";
const TERMINAL_SECTION: &str = "terminal";
// pub(crate) rather than private: `work_dir::tests` reaches this section name
// directly (see `write_context_config_preserves_prompt_cache_split`) since it
// isn't a descendant of this module and so can't see a private const here.
pub(crate) const CONTEXT_SECTION: &str = "context";

fn config_path(work_dir: &Path) -> PathBuf {
    work_dir.join("config.toml")
}

/// Read `.loom/work/config.toml` as a `toml_edit::DocumentMut`, preserving
/// comments, formatting, and unknown keys. Returns an empty document if the
/// file does not exist.
pub fn read_config(work_dir: &Path) -> Result<DocumentMut> {
    let path = config_path(work_dir);
    if !path.exists() {
        return Ok(DocumentMut::new());
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    content
        .parse::<DocumentMut>()
        .with_context(|| format!("Failed to parse {}", path.display()))
}

/// Write the document back to `.loom/work/config.toml`, crash-atomically and
/// under the state directory lock.
///
/// The write goes through [`crate::fs::locking::locked_write`] (temp file +
/// `fsync` + `rename`), so a crash mid-write leaves either the old config or the
/// fully-written new config — never a truncated file. The lock serializes
/// against other config writers using the same module.
///
/// NOTE: callers performing a read-modify-write (`read_config` → mutate →
/// `write_config`) should prefer [`update_config`], which holds the lock across
/// the entire sequence so concurrent writers cannot lose each other's sections.
/// A bare `write_config` only makes the final write atomic, not the surrounding
/// read-modify-write.
pub fn write_config(work_dir: &Path, doc: &DocumentMut) -> Result<()> {
    let path = config_path(work_dir);
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
    }
    crate::fs::locking::locked_write(&path, &doc.to_string())
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

/// Read-modify-write `.loom/work/config.toml` while holding the state
/// directory lock for the whole sequence.
///
/// This is the lost-update-safe way to mutate the config: the read, the
/// `modify` closure, and the atomic write all happen under a single exclusive
/// directory lock, so a concurrent daemon plan-rename and a CLI section write
/// can no longer interleave and drop each other's sections.
pub fn update_config<F>(work_dir: &Path, modify: F) -> Result<()>
where
    F: FnOnce(&mut DocumentMut) -> Result<()>,
{
    let path = config_path(work_dir);
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
    }

    crate::fs::locking::locked_update(&path, |existing| {
        let mut doc = if existing.is_empty() {
            DocumentMut::new()
        } else {
            existing
                .parse::<DocumentMut>()
                .with_context(|| format!("Failed to parse {}", path.display()))?
        };
        modify(&mut doc)?;
        Ok(doc.to_string())
    })
    .with_context(|| format!("Failed to update {}", path.display()))
}

fn read_section<T: serde::de::DeserializeOwned>(
    work_dir: &Path,
    section: &str,
) -> Result<Option<T>> {
    let path = config_path(work_dir);
    if !path.exists() {
        return Ok(None);
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    let value: toml::Value = toml::from_str(&content)
        .with_context(|| format!("Failed to parse {} as TOML", path.display()))?;
    let Some(section_value) = value.get(section).cloned() else {
        return Ok(None);
    };
    let typed: T = section_value
        .try_into()
        .with_context(|| format!("Failed to deserialize [{section}] section"))?;
    Ok(Some(typed))
}

/// Render `value` as a document holding nothing but `[section]`.
fn rendered_section_doc<T: serde::Serialize>(section: &str, value: &T) -> Result<DocumentMut> {
    // Serialize the typed value to a toml::Value, then convert to a
    // toml_edit Item by parsing its string representation.
    let toml_value = toml::Value::try_from(value)
        .with_context(|| format!("Failed to serialize [{section}] section"))?;
    let rendered = toml::to_string_pretty(&toml::Value::Table({
        let mut t = toml::map::Map::new();
        t.insert(section.to_string(), toml_value);
        t
    }))
    .with_context(|| format!("Failed to render [{section}] section"))?;

    rendered
        .parse()
        .with_context(|| format!("Failed to re-parse rendered [{section}] section"))
}

fn write_section<T: serde::Serialize>(work_dir: &Path, section: &str, value: &T) -> Result<()> {
    let new_doc = rendered_section_doc(section, value)?;

    let section = section.to_string();
    // RMW under the directory lock so a concurrent writer (e.g. the daemon
    // plan-rename touching `[plan]`) cannot drop the section we are inserting,
    // nor we theirs.
    update_config(work_dir, |doc| {
        if let Some(item) = new_doc.get(&section) {
            doc.insert(&section, item.clone());
        } else {
            // Section serialized to nothing (empty table) — remove from doc.
            doc.remove(&section);
        }
        Ok(())
    })
}

/// Like [`write_section`], but MERGES `value`'s keys into the section instead
/// of replacing it, leaving keys no Rust struct owns exactly as they were.
///
/// `[context]` has more than one owner: [`ContextConfig`] writes the two
/// ceilings, while `prompt_cache_split` is read straight from the document by
/// `native::launch::prompt_cache_split_enabled` and belongs to no struct at
/// all. Replacing that table on a re-init would silently switch prompt cache
/// splitting back off. Single-owner sections keep using [`write_section`],
/// whose replace semantics are what they want.
fn merge_section<T: serde::Serialize>(work_dir: &Path, section: &str, value: &T) -> Result<()> {
    let new_doc = rendered_section_doc(section, value)?;

    let section = section.to_string();
    // Same RMW-under-the-directory-lock discipline as `write_section`.
    update_config(work_dir, |doc| {
        // Nothing to merge: an empty rendered section sets no keys, so it must
        // leave the existing table (and its other owners' keys) untouched.
        let Some(new_table) = new_doc.get(&section).and_then(|item| item.as_table()) else {
            return Ok(());
        };
        let existing = doc
            .entry(&section)
            .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
        // `as_table_like_mut` covers the inline form (`context = { .. }`) too,
        // so a hand-written section keeps its other keys either way.
        match existing.as_table_like_mut() {
            Some(table) => {
                for (key, item) in new_table.iter() {
                    table.insert(key, item.clone());
                }
            }
            // Not a table at all (an operator wrote a scalar): there is nothing
            // to preserve, so replace it with the section this module owns.
            None => *existing = toml_edit::Item::Table(new_table.clone()),
        }
        Ok(())
    })
}

/// Insert `field = value` into `[section]` of an in-flight document, creating
/// the section when absent.
///
/// Writes the one key rather than a serialized struct, which is what
/// `write_section` and `merge_section` do. A caller setting a single
/// operator-chosen key must not freeze its section's other values into the
/// file as a side effect: `[context]`'s `subagent_ceiling_tokens` is DERIVED
/// unless the file states it, and writing the derived number would silently
/// pin it, while `prompt_cache_split` belongs to no struct at all and would be
/// dropped by a replacing write.
///
/// Takes the document rather than the work directory so the caller can pair it
/// with [`remove_key`] and its own before/after reads inside ONE
/// [`update_config`] closure — that is, inside one hold of the directory lock.
pub fn insert_key(
    doc: &mut DocumentMut,
    section: &str,
    field: &str,
    value: toml_edit::Value,
) -> Result<()> {
    let table = doc
        .entry(section)
        .or_insert(toml_edit::table())
        .as_table_like_mut()
        .ok_or_else(|| anyhow::anyhow!("[{section}] in the workspace config is not a table"))?;
    table.insert(field, toml_edit::Item::Value(value));
    Ok(())
}

/// Remove `field` from `[section]`, dropping the section itself when that
/// leaves it empty.
///
/// Dropping the emptied section is the point, not tidiness. The workspace
/// tier's fallback to `~/.loom/config.toml` is SECTION-level (see
/// [`read_context_config`]), so a section left present but keyless still wins
/// WHOLE and resolves to the derived built-in — an operator clearing the one
/// key they had set would find the user tier still shadowed by a section that
/// no longer says anything.
///
/// A section still holding another key stays, because it still means
/// something — and for `[context]` that is the COMMON case, not an exotic one.
/// `loom init` writes `ceiling_tokens` and `subagent_ceiling_tokens` together
/// whenever a plan sets either (`commands::init::plan_setup`), so removing
/// `ceiling_tokens` from such a workspace leaves the section alive and the
/// ceiling resolving to the built-in rather than to the user tier. That is the
/// truth about the file, and a caller reporting the result must report it as
/// still project-sourced rather than as a reset. `prompt_cache_split`, owned
/// by no struct at all, behaves the same way.
pub fn remove_key(doc: &mut DocumentMut, section: &str, field: &str) {
    let Some(table) = doc
        .get_mut(section)
        .and_then(|item| item.as_table_like_mut())
    else {
        return;
    };
    table.remove(field);
    if table.is_empty() {
        doc.remove(section);
    }
}

/// Read the persisted plan-level sandbox config (`[plan_sandbox]`).
///
/// Returns `Ok(None)` if the section is missing — callers should fall back
/// to plan-file parsing or defaults.
pub fn read_plan_sandbox(work_dir: &Path) -> Result<Option<SandboxConfig>> {
    read_section(work_dir, PLAN_SANDBOX_SECTION)
}

/// Persist the plan-level sandbox config (`[plan_sandbox]`).
pub fn write_plan_sandbox(work_dir: &Path, sandbox: &SandboxConfig) -> Result<()> {
    write_section(work_dir, PLAN_SANDBOX_SECTION, sandbox)
}

/// Read the persisted Remote Control config (`[remote_control]`).
///
/// A missing section yields `RemoteControlConfig::default()` (mode = auto),
/// so callers always get a usable value.
pub fn read_remote_control_config(work_dir: &Path) -> Result<RemoteControlConfig> {
    Ok(read_section(work_dir, REMOTE_CONTROL_SECTION)?.unwrap_or_default())
}

/// Persist the Remote Control config (`[remote_control]`).
pub fn write_remote_control_config(work_dir: &Path, config: &RemoteControlConfig) -> Result<()> {
    write_section(work_dir, REMOTE_CONTROL_SECTION, config)
}

/// Read the persisted terminal backend config (`[terminal]`).
///
/// Resolution order: a present workspace `[terminal]` section wins WHOLE; an
/// absent one falls through to `~/.loom/config.toml`'s `terminal.backend`
/// (see [`crate::user_config::UserConfig`]), then to
/// `TerminalConfig::default()` (native) when neither is set. So a missing
/// section no longer means "the default" outright — it means "the user
/// config, then the default".
pub fn read_terminal_config(work_dir: &Path) -> Result<TerminalConfig> {
    if let Some(section) = read_section::<TerminalConfig>(work_dir, TERMINAL_SECTION)? {
        return Ok(section);
    }
    Ok(TerminalConfig {
        backend: crate::user_config::UserConfig::load().terminal_backend(),
    })
}

/// Persist the terminal backend config (`[terminal]`).
pub fn write_terminal_config(work_dir: &Path, config: &TerminalConfig) -> Result<()> {
    write_section(work_dir, TERMINAL_SECTION, config)
}

/// Read the persisted context ceilings (`[context]`).
///
/// Resolution order: a present workspace `[context]` section wins WHOLE; an
/// absent one falls through to `~/.loom/config.toml`'s `context.ceiling_tokens`
/// (see [`crate::user_config::UserConfig`]) when set, then to
/// `ContextConfig::default()`. So a missing section no longer means "the
/// default" outright — it means "the user config, then the default".
///
/// This fallback is SECTION-level, not key-level, and deliberately so:
/// `[context]` deserializes through the private `ContextConfigRaw`
/// (`context_config.rs`), whose entire purpose is to tell "the TOML set this
/// key" apart from "the TOML left this to derive" *before* the built-in
/// defaults are baked in by its `From` impl — by the time
/// `read_section::<ContextConfig>` has returned, that distinction is gone, and
/// a key-level merge would silently treat a derived default as an explicit
/// setting. Only `ceiling_tokens` gets a user-level override here;
/// `subagent_ceiling_tokens` and `model_window_tokens` keep deriving from the
/// built-ins regardless of the user config, and `ContextConfig::ceiling_for`'s
/// stage-override rule is untouched.
pub fn read_context_config(work_dir: &Path) -> Result<ContextConfig> {
    if let Some(section) = read_section::<ContextConfig>(work_dir, CONTEXT_SECTION)? {
        return Ok(section);
    }
    Ok(ContextConfig::with_user_ceiling(
        crate::user_config::UserConfig::load().context_ceiling_tokens_set(),
    ))
}

/// Persist the context ceilings (`[context]`).
///
/// Merges rather than replaces: `[context]` also carries `prompt_cache_split`,
/// which no Rust struct owns (see `merge_section`).
pub fn write_context_config(work_dir: &Path, config: &ContextConfig) -> Result<()> {
    merge_section(work_dir, CONTEXT_SECTION, config)
}

/// The ceiling governing a stage's agent session, in absolute resident tokens.
///
/// ONE resolution order, and every reader of a stage ceiling must use it:
/// the stage's own `context_ceiling_tokens` -> `[context] ceiling_tokens` ->
/// `~/.loom/config.toml`'s `context.ceiling_tokens` ->
/// [`crate::models::constants::DEFAULT_CONTEXT_CEILING_TOKENS`]. Skipping a
/// middle tier makes the signal, the governor and the daemon quote different
/// numbers for one session.
///
/// Takes the stage's value rather than the `Stage` itself so `fs/` keeps no
/// dependency on the stage model — pass `stage.context_ceiling_tokens`. A
/// caller that already holds a [`ContextConfig`] uses
/// [`ContextConfig::ceiling_for`] instead, which is the same order without the
/// read.
pub fn resolve_context_ceiling_tokens(work_dir: &Path, stage_ceiling: Option<u32>) -> u32 {
    // An unreadable or unparseable workspace [context] section still falls
    // through to the user config tier (then the built-in default) rather than
    // skipping straight to the built-in — a malformed workspace section must
    // not silently discard an operator's ~/.loom/config.toml ceiling.
    let config = read_context_config(work_dir).unwrap_or_else(|_| {
        ContextConfig::with_user_ceiling(
            crate::user_config::UserConfig::load().context_ceiling_tokens_set(),
        )
    });
    config.ceiling_for(stage_ceiling)
}
