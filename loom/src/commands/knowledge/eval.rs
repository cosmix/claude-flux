//! `loom knowledge eval` — score retrieval against a checked-in ground-truth
//! set using hit rate, ranking quality, delivery eligibility, and rendered cost.
//!
//! See `doc/PROPOSAL-retrieval-precision.md` §5 ("Measure it") and Appendix
//! A.20 for the design this module implements. The cases file
//! (`loom/eval/retrieval-cases.yaml` by default) pairs a query with relevant,
//! required, forbidden, or deliberately abstaining outcomes.
//!
//! `mode: stage` differs from `mode: prompt` only in its default budget (3000
//! vs 1500 tokens) and in that `stage_fields` lines are newline-joined onto
//! `query` — the same shape `build_stage_query_text` assembles a real stage's
//! metadata into, just supplied by the case instead of a `Stage` record.
//!
//! **This is a CLI gate, not a `cargo test`.** It reads the live on-disk
//! index (whatever `loom knowledge sync` last built), which is not
//! reproducible in CI. The unit tests exercise parsing, metrics, and gates
//! with synthetic packs; run this command by hand against a real index.

use crate::context::config::RetrievalConfig;
use crate::context::retrieve::{retrieve_for_stage, StageQuery};
use crate::fs::work_dir::WorkDir;
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

mod cases;
mod metrics;
mod report;

use cases::{build_query_text, load_cases_file, resolve_budget, EvalCase, CASES_RELATIVE_PATH};
use metrics::{score_case, CaseResult};
use report::{aggregate, exit_reason, print_human, print_json};

/// Run every case in `cases` (or the default cases file) and report the gate.
pub fn eval(cases: Option<PathBuf>, budget_tokens: Option<usize>, json: bool) -> Result<()> {
    let work_dir_hint = Path::new(".");
    let main_root = main_project_root(work_dir_hint)?;
    let cases_path = cases.unwrap_or_else(|| main_root.join(CASES_RELATIVE_PATH));
    let cases_file = load_cases_file(&cases_path)?;
    let config = RetrievalConfig::load(&main_root);

    let results = cases_file
        .cases
        .iter()
        .map(|case| run_case(case, budget_tokens, work_dir_hint, &config))
        .collect::<Result<Vec<_>>>()?;
    let aggregates = aggregate(&results);
    let reason = exit_reason(
        &aggregates,
        cases_file.pass_floor,
        cases_file.precision_floor,
    );

    if json {
        print_json(&results, &cases_file, &aggregates, reason.as_deref())?;
    } else {
        print_human(&results, &cases_file, &aggregates, reason.as_deref());
    }

    if let Some(reason) = &reason {
        eprintln!("loom knowledge eval: FAIL - {reason}");
        std::process::exit(1);
    }
    Ok(())
}

fn main_project_root(work_dir_hint: &Path) -> Result<PathBuf> {
    let work_dir = WorkDir::new(work_dir_hint)?;
    work_dir.main_project_root().ok_or_else(|| {
        anyhow!("Could not resolve the main project root to locate {CASES_RELATIVE_PATH}")
    })
}

fn run_case(
    case: &EvalCase,
    cli_budget: Option<usize>,
    work_dir_hint: &Path,
    config: &RetrievalConfig,
) -> Result<CaseResult> {
    let budget = resolve_budget(case, cli_budget);
    let mut query = StageQuery::new(work_dir_hint, build_query_text(case));
    query.required_ids = case.require_ids.clone();

    let pack = retrieve_for_stage(&query, budget)
        .with_context(|| format!("eval case '{}' failed retrieval", case.name))?;
    Ok(score_case(case, &pack, config))
}

#[cfg(test)]
#[path = "tests_eval.rs"]
mod tests;
