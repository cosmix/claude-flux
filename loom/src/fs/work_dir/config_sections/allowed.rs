//! Shared value-set guard for the project-tier `[models]`/`[pressure]`
//! sections.

/// Keep a workspace-config string only when it names one of `allowed`.
///
/// These values are concatenated into a Claude or Codex command line
/// (`--model`, `--effort`, `-c model_reasoning_effort=`), so an out-of-set
/// value is DROPPED with a warning rather than forwarded: falling through to
/// the next tier is the safe reading of a typo, and both writers that can
/// produce this file (`loom config`, the dashboard settings API) validate on
/// write, so a bad value can only arrive by hand-editing.
pub(super) fn allowed_value<'a>(
    value: Option<&'a String>,
    allowed: &[&str],
    key: &str,
) -> Option<&'a str> {
    let value = value?.as_str();
    if allowed.contains(&value) {
        return Some(value);
    }
    tracing::warn!(
        key,
        value,
        allowed = %allowed.join(", "),
        "workspace config value is not in the allowed set; dropping to fall through to the next tier"
    );
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_value_passes_through_a_set_value() {
        let value = Some("sonnet".to_string());
        assert_eq!(
            allowed_value(value.as_ref(), &["sonnet", "opus"], "models.standard_model"),
            Some("sonnet")
        );
    }

    #[test]
    fn allowed_value_drops_an_out_of_set_value() {
        let value = Some("nonexistent-model".to_string());
        assert_eq!(
            allowed_value(value.as_ref(), &["sonnet", "opus"], "models.standard_model"),
            None
        );
    }

    #[test]
    fn allowed_value_passes_through_none() {
        assert_eq!(
            allowed_value(None, &["sonnet", "opus"], "models.standard_model"),
            None
        );
    }
}
