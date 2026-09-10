use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::Path;

/// Default token budget for a `mode: prompt` case, absent an override.
pub(super) const DEFAULT_PROMPT_BUDGET_TOKENS: usize = 1500;
/// Default token budget for a `mode: stage` case, absent an override.
pub(super) const DEFAULT_STAGE_BUDGET_TOKENS: usize = 3000;
/// Default cases file, relative to the main project root.
pub(super) const CASES_RELATIVE_PATH: &str = "loom/eval/retrieval-cases.yaml";

/// A case's query mode: which default budget applies and whether
/// `stage_fields` gets folded into the query text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum EvalMode {
    #[default]
    Prompt,
    Stage,
}

/// One ground-truth case, deserialized straight off the YAML cases file.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct EvalCase {
    pub(super) name: String,
    #[serde(default)]
    pub(super) query: String,
    #[serde(default)]
    pub(super) mode: EvalMode,
    #[serde(default)]
    pub(super) budget_tokens: Option<usize>,
    #[serde(default)]
    pub(super) stage_fields: Vec<String>,
    #[serde(default)]
    pub(super) require_ids: Vec<String>,
    #[serde(default)]
    pub(super) expect: Vec<String>,
    #[serde(default)]
    pub(super) relevant: Vec<String>,
    #[serde(default)]
    pub(super) forbid: Vec<String>,
    #[serde(default)]
    pub(super) abstain: bool,
    #[serde(default)]
    pub(super) max_rendered_tokens: Option<usize>,
}

fn default_pass_floor() -> f32 {
    0.5
}

/// The whole cases file: aggregate gates plus the cases they gate on.
#[derive(Debug, Deserialize)]
pub(super) struct CasesFile {
    #[serde(default = "default_pass_floor")]
    pub(super) pass_floor: f32,
    #[serde(default)]
    pub(super) precision_floor: f32,
    #[serde(default)]
    pub(super) cases: Vec<EvalCase>,
}

pub(super) fn load_cases_file(path: &Path) -> Result<CasesFile> {
    let raw = std::fs::read_to_string(path).with_context(|| {
        format!(
            "No eval cases file at {} - pass --cases or create it",
            path.display()
        )
    })?;
    let parsed: CasesFile = serde_yaml::from_str(&raw)
        .with_context(|| format!("Failed to parse eval cases file {}", path.display()))?;
    validate_cases(&parsed)?;
    Ok(parsed)
}

/// Reject case definitions that are ambiguous or can never exercise a gate.
pub(super) fn validate_cases(file: &CasesFile) -> Result<()> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for case in &file.cases {
        if !seen.insert(case.name.as_str()) {
            bail!("eval case '{}' is defined more than once", case.name);
        }
        if case.query.trim().is_empty() {
            bail!("eval case '{}' has an empty query", case.name);
        }
        if case.abstain && !case.expect.is_empty() {
            bail!(
                "eval case '{}' cannot set `abstain: true` together with `expect`",
                case.name
            );
        }
        if case.expect.is_empty() && case.forbid.is_empty() && !case.abstain {
            bail!(
                "eval case '{}' has none of `expect`, `forbid`, or `abstain` - it could never fail",
                case.name
            );
        }
    }
    Ok(())
}

/// Fold `stage_fields` onto `query` for stage-mode cases.
pub(super) fn build_query_text(case: &EvalCase) -> String {
    if case.mode != EvalMode::Stage || case.stage_fields.is_empty() {
        return case.query.clone();
    }
    let mut parts = vec![case.query.clone()];
    parts.extend(case.stage_fields.iter().cloned());
    parts.retain(|part| !part.trim().is_empty());
    parts.join("\n")
}

pub(super) fn resolve_budget(case: &EvalCase, cli_override: Option<usize>) -> usize {
    cli_override
        .or(case.budget_tokens)
        .unwrap_or(match case.mode {
            EvalMode::Prompt => DEFAULT_PROMPT_BUDGET_TOKENS,
            EvalMode::Stage => DEFAULT_STAGE_BUDGET_TOKENS,
        })
}
