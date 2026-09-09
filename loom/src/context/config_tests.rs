//! Tests for [`crate::context::config`].
//!
//! Split out of `config.rs` so the module stays comfortably inside the 400-line
//! file limit as later waves add tunables.

use super::*;
use tempfile::TempDir;

/// A project root whose `.loom/config.toml` holds `text` verbatim.
fn project_with_config(text: &str) -> TempDir {
    let root = TempDir::new().expect("temp dir");
    let dir = root.path().join(".loom");
    std::fs::create_dir_all(&dir).expect("create .loom");
    std::fs::write(dir.join("config.toml"), text).expect("write config");
    root
}

/// Load from a config file holding `text` verbatim.
fn load_file(text: &str) -> RetrievalConfig {
    let root = project_with_config(text);
    RetrievalConfig::load(root.path())
}

/// Load from a config file whose `[retrieval]` table holds `body`.
fn load(body: &str) -> RetrievalConfig {
    load_file(&format!("[retrieval]\n{body}\n"))
}

#[test]
fn an_absent_file_yields_the_defaults() {
    let root = TempDir::new().expect("temp dir");
    assert_eq!(
        RetrievalConfig::load(root.path()),
        RetrievalConfig::default()
    );
}

#[test]
fn a_partial_file_overrides_only_the_keys_it_names() {
    let config = load("min_knowledge_terms = 4");
    let defaults = RetrievalConfig::default();

    assert_eq!(config.min_knowledge_terms, 4);
    assert_eq!(config.stop_df_ratio, defaults.stop_df_ratio);
    assert_eq!(config.prose_roots, defaults.prose_roots);
    assert_eq!(config.prompt_budget_tokens, defaults.prompt_budget_tokens);
}

#[test]
fn a_file_that_is_not_toml_yields_the_defaults_without_panicking() {
    let config = load_file("this is not toml {{{ ][\n");
    assert_eq!(config, RetrievalConfig::default());
}

#[test]
fn a_file_with_no_retrieval_table_yields_the_defaults() {
    let config = load_file("[something-else]\nkey = 1\n");
    assert_eq!(config, RetrievalConfig::default());
}

#[test]
fn a_retrieval_key_that_is_not_a_table_yields_the_defaults() {
    assert_eq!(load_file("retrieval = 5\n"), RetrievalConfig::default());
}

/// A.19: a zero or negative factor falls back to the default rather than to the
/// clamp bound. Zero would multiply every test-path node's total score to
/// nothing, which is exclusion, not the downweight the factor exists to apply.
#[test]
fn a_non_positive_factor_falls_back_to_its_default() {
    let default = RetrievalConfig::default().test_path_factor;

    assert_eq!(load("test_path_factor = 0").test_path_factor, default);
    assert_eq!(load("test_path_factor = 0.0").test_path_factor, default);
    assert_eq!(load("test_path_factor = -1").test_path_factor, default);
}

#[test]
fn a_ratio_above_one_clamps_to_one() {
    assert_eq!(load("stop_df_ratio = 5.0").stop_df_ratio, 1.0);
}

/// `stop_df_ratio = 1` is what an operator writes when they mean `1.0`, and
/// TOML types it as an integer.
#[test]
fn a_ratio_written_as_an_integer_is_accepted() {
    assert_eq!(load("stop_df_ratio = 1").stop_df_ratio, 1.0);
}

/// A.16's rescue ceiling is read like every other ratio: absent it is the
/// shipped 0.25, above one it clamps, and a non-positive value falls back rather
/// than disabling the rescue floor by arithmetic accident — `0` would put the
/// ceiling under every possible document frequency and quietly restore the empty
/// packs the floor exists to prevent.
#[test]
fn the_rescue_ceiling_defaults_and_clamps_like_a_ratio() {
    let default = RetrievalConfig::default().stop_rescue_max_ratio;

    assert_eq!(default, 0.25);
    assert_eq!(ceiling("min_knowledge_terms = 4"), default);
    assert_eq!(ceiling("stop_rescue_max_ratio = 0.5"), 0.5);
    assert_eq!(ceiling("stop_rescue_max_ratio = 5.0"), 1.0);
    assert_eq!(ceiling("stop_rescue_max_ratio = 0"), default);
    assert_eq!(ceiling("stop_rescue_max_ratio = -0.5"), default);
}

/// The rescue ceiling a `[retrieval]` table holding `body` resolves to.
fn ceiling(body: &str) -> f32 {
    load(body).stop_rescue_max_ratio
}

#[test]
fn a_budget_clamps_into_its_legal_range() {
    let low = load("prompt_budget_tokens = 1");
    let high = load("stage_brief_budget_tokens = 999999");
    let negative = load("max_payload_bytes = -8");

    assert_eq!(low.prompt_budget_tokens, MIN_BUDGET_TOKENS);
    assert_eq!(high.stage_brief_budget_tokens, MAX_BUDGET);
    assert_eq!(negative.max_payload_bytes, MIN_BUDGET_TOKENS);
}

#[test]
fn a_count_clamps_to_at_least_one() {
    assert_eq!(load("df_ident_max = 0").df_ident_max, 1);
    assert_eq!(load("keep_base_graphs = -3").keep_base_graphs, 1);
}

/// Seconds are durations, not counts: `0` is a meaningful "no delay" and must
/// survive, while a negative is not expressible and falls back.
#[test]
fn seconds_accept_zero_and_reject_a_negative() {
    let defaults = RetrievalConfig::default();
    let zero = load("reconcile_debounce_secs = 0");
    let negative = load("reconcile_stale_lock_secs = -1");

    assert_eq!(zero.reconcile_debounce_secs, 0);
    assert_eq!(
        negative.reconcile_stale_lock_secs,
        defaults.reconcile_stale_lock_secs
    );
}

/// An additive prior is not a ratio: its default is 5.0, so clamping it into
/// `(0.0, 1.0]` would silently rewrite the shipped value.
#[test]
fn the_curated_prior_is_not_clamped_to_one() {
    let defaults = RetrievalConfig::default();
    let raised = load("knowledge_curated_prior = 12.5");
    let negative = load("knowledge_curated_prior = -1.0");

    assert_eq!(raised.knowledge_curated_prior, 12.5);
    assert_eq!(
        negative.knowledge_curated_prior,
        defaults.knowledge_curated_prior
    );
}

/// A prose root that escapes the project would aim the indexer at any directory
/// the process can read and pull its contents into a Knowledge Brief.
#[test]
fn prose_roots_outside_the_project_are_dropped() {
    let config = load(r#"prose_roots = ["../etc", "", "/abs", "doc"]"#);
    assert_eq!(config.prose_roots, vec!["doc".to_string()]);
}

#[test]
fn a_nested_escaping_prose_root_is_dropped_too() {
    let config = load(r#"prose_roots = ["doc/../../etc", "doc/prose"]"#);
    assert_eq!(config.prose_roots, vec!["doc/prose".to_string()]);
}

/// An operator who writes an empty list means "index no prose", and gets it.
#[test]
fn an_explicitly_empty_prose_root_list_is_honoured() {
    assert!(load("prose_roots = []").prose_roots.is_empty());
}

#[test]
fn an_unknown_key_is_ignored_and_does_not_fail_the_load() {
    let defaults = RetrievalConfig::default();
    let config = load("stop_df_ration = 0.5\nmin_knowledge_terms = 7");

    assert_eq!(config.min_knowledge_terms, 7);
    assert_eq!(config.stop_df_ratio, defaults.stop_df_ratio);
}

#[test]
fn a_value_of_the_wrong_type_costs_only_its_own_field() {
    let defaults = RetrievalConfig::default();
    let config = load("stop_df_ratio = \"half\"\nmin_knowledge_terms = 7");

    assert_eq!(config.stop_df_ratio, defaults.stop_df_ratio);
    assert_eq!(config.min_knowledge_terms, 7);
}
