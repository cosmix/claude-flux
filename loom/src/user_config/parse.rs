//! Typed field extraction for `~/.loom/config.toml`'s parsed `DocumentMut`.
//!
//! Split out of `mod.rs` to keep that file under Rule 17's 400-line limit —
//! [`super::parse_document`] is the only caller, walking one `[section]
//! field` pair per registered key and converting a type mismatch into an
//! error naming the key, the offending TOML type, and — for `get_u32` — the
//! offending value.

use anyhow::Result;
use toml_edit::DocumentMut;

use crate::models::session::SessionBackendKind;

pub(super) fn section_item<'a>(
    doc: &'a DocumentMut,
    section: &str,
    field: &str,
) -> Option<&'a toml_edit::Item> {
    doc.get(section)?.get(field)
}

pub(super) fn get_bool(doc: &DocumentMut, section: &str, field: &str) -> Result<Option<bool>> {
    match section_item(doc, section, field) {
        None => Ok(None),
        Some(item) => item.as_bool().map(Some).ok_or_else(|| {
            anyhow::anyhow!(
                "{section}.{field}: expected a bool, found {}",
                item.type_name()
            )
        }),
    }
}

pub(super) fn get_u32(doc: &DocumentMut, section: &str, field: &str) -> Result<Option<u32>> {
    match section_item(doc, section, field) {
        None => Ok(None),
        Some(item) => {
            let int = item.as_integer().ok_or_else(|| {
                anyhow::anyhow!(
                    "{section}.{field}: expected an integer, found {}",
                    item.type_name()
                )
            })?;
            u32::try_from(int)
                .map(Some)
                .map_err(|_| anyhow::anyhow!("{section}.{field}: {int} is out of range for a u32"))
        }
    }
}

/// Extract and validate an enum-typed field against `allowed`, the same
/// shape as [`get_backend`] but for a registry key whose variants come from a
/// `&'static [&'static str]` (see [`super::keys::ValueKind::Enum`]) rather
/// than a fixed Rust enum.
pub(super) fn get_enum(
    doc: &DocumentMut,
    section: &str,
    field: &str,
    allowed: &[&str],
) -> Result<Option<String>> {
    match section_item(doc, section, field) {
        None => Ok(None),
        Some(item) => {
            let raw = item.as_str().ok_or_else(|| {
                anyhow::anyhow!(
                    "{section}.{field}: expected a string, found {}",
                    item.type_name()
                )
            })?;
            if allowed.contains(&raw) {
                Ok(Some(raw.to_string()))
            } else {
                Err(anyhow::anyhow!(
                    "{section}.{field}: {raw:?} is not one of the expected values: {}",
                    allowed.join(", ")
                ))
            }
        }
    }
}

pub(super) fn get_backend(doc: &DocumentMut) -> Result<Option<SessionBackendKind>> {
    match section_item(doc, "terminal", "backend") {
        None => Ok(None),
        Some(item) => {
            let raw = item.as_str().ok_or_else(|| {
                anyhow::anyhow!(
                    "terminal.backend: expected a string, found {}",
                    item.type_name()
                )
            })?;
            match raw {
                "native" => Ok(Some(SessionBackendKind::Native)),
                "tmux" => Ok(Some(SessionBackendKind::Tmux)),
                other => Err(anyhow::anyhow!(
                    "terminal.backend: {other:?} is not one of native, tmux"
                )),
            }
        }
    }
}
