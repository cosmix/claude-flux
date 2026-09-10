use super::cases::CasesFile;
use super::metrics::CaseResult;
use crate::context::untrusted::inline_safe;
use anyhow::Result;

/// Aggregate scores and gate counts over every case in a run.
#[derive(Debug, Default)]
pub(super) struct Aggregates {
    pub(super) hit_rate_at_5: f32,
    pub(super) hit_rate_hits: usize,
    pub(super) hit_rate_applicable: usize,
    pub(super) mean_mrr: f32,
    pub(super) precision_at_5: f32,
    pub(super) precision_applicable: usize,
    pub(super) relevant_token_fraction: f32,
    pub(super) relevant_token_fraction_applicable: usize,
    pub(super) mandatory_recall: f32,
    pub(super) mandatory_recall_applicable: usize,
    pub(super) abstention_accuracy: f32,
    pub(super) abstention_correct: usize,
    pub(super) abstention_applicable: usize,
    pub(super) rendered_tokens_mean: f32,
    pub(super) rendered_tokens_max: usize,
    pub(super) forbid_violations: usize,
    pub(super) unmet_required: usize,
    pub(super) abstention_failures: usize,
    pub(super) rendered_token_violations: usize,
}

struct AggregateMeans {
    precision: (f32, usize),
    relevant_tokens: (f32, usize),
    mandatory: (f32, usize),
    abstention: (f32, usize),
}

pub(super) fn aggregate(results: &[CaseResult]) -> Aggregates {
    let hit_results: Vec<_> = results
        .iter()
        .filter(|result| result.counts_toward_hit_rate)
        .collect();
    let hit_rate_hits = hit_results.iter().filter(|result| result.hit_at_5).count();
    let means = aggregate_means(results);
    let rendered_total: usize = results.iter().map(|result| result.rendered_tokens).sum();

    Aggregates {
        hit_rate_at_5: divide(hit_rate_hits as f32, hit_results.len()),
        hit_rate_hits,
        hit_rate_applicable: hit_results.len(),
        mean_mrr: divide(
            hit_results.iter().map(|result| result.mrr).sum(),
            hit_results.len(),
        ),
        precision_at_5: means.precision.0,
        precision_applicable: means.precision.1,
        relevant_token_fraction: means.relevant_tokens.0,
        relevant_token_fraction_applicable: means.relevant_tokens.1,
        mandatory_recall: means.mandatory.0,
        mandatory_recall_applicable: means.mandatory.1,
        abstention_accuracy: means.abstention.0,
        abstention_correct: results
            .iter()
            .filter(|result| result.abstention_correct == Some(true))
            .count(),
        abstention_applicable: means.abstention.1,
        rendered_tokens_mean: divide(rendered_total as f32, results.len()),
        rendered_tokens_max: results
            .iter()
            .map(|result| result.rendered_tokens)
            .max()
            .unwrap_or(0),
        forbid_violations: results
            .iter()
            .map(|result| result.forbid_violations.len())
            .sum(),
        unmet_required: results.iter().map(|result| result.unmet_required).sum(),
        abstention_failures: results
            .iter()
            .filter(|result| result.abstention_correct == Some(false))
            .count(),
        rendered_token_violations: results
            .iter()
            .filter(|result| result.exceeds_rendered_limit())
            .count(),
    }
}

fn aggregate_means(results: &[CaseResult]) -> AggregateMeans {
    AggregateMeans {
        precision: option_mean(results.iter().map(|result| result.precision_at_5)),
        relevant_tokens: option_mean(results.iter().map(|result| result.relevant_token_fraction)),
        mandatory: option_mean(
            results
                .iter()
                .map(|result| result.mandatory_recall.map(|recall| recall.ratio())),
        ),
        abstention: option_mean(results.iter().map(|result| {
            result
                .abstention_correct
                .map(|correct| if correct { 1.0 } else { 0.0 })
        })),
    }
}

fn option_mean(values: impl Iterator<Item = Option<f32>>) -> (f32, usize) {
    let values: Vec<f32> = values.flatten().collect();
    (divide(values.iter().sum(), values.len()), values.len())
}

fn divide(numerator: f32, denominator: usize) -> f32 {
    if denominator == 0 {
        0.0
    } else {
        numerator / denominator as f32
    }
}

/// The combined reason a run fails, or `None` when every gate passes.
pub(super) fn exit_reason(
    aggregates: &Aggregates,
    pass_floor: f32,
    precision_floor: f32,
) -> Option<String> {
    let mut reasons = Vec::new();
    if aggregates.hit_rate_at_5 < pass_floor {
        reasons.push(format!(
            "hit@5 {:.2} < pass_floor {:.2}",
            aggregates.hit_rate_at_5, pass_floor
        ));
    }
    if aggregates.precision_at_5 < precision_floor {
        reasons.push(format!(
            "precision@5 {:.2} < precision_floor {:.2}",
            aggregates.precision_at_5, precision_floor
        ));
    }
    push_count_reason(
        &mut reasons,
        aggregates.forbid_violations,
        "forbid violation(s)",
    );
    push_count_reason(
        &mut reasons,
        aggregates.abstention_failures,
        "abstention case(s) emitted",
    );
    push_count_reason(
        &mut reasons,
        aggregates.unmet_required,
        "required id(s) unmet",
    );
    push_count_reason(
        &mut reasons,
        aggregates.rendered_token_violations,
        "rendered-token ceiling violation(s)",
    );
    (!reasons.is_empty()).then(|| reasons.join("; "))
}

fn push_count_reason(reasons: &mut Vec<String>, count: usize, label: &str) {
    if count > 0 {
        reasons.push(format!("{count} {label}"));
    }
}

pub(super) fn print_human(
    results: &[CaseResult],
    cases_file: &CasesFile,
    aggregates: &Aggregates,
    reason: Option<&str>,
) {
    for result in results {
        println!("{}", format_case_line(result));
        print_case_failures(result);
    }
    println!("\nAggregate:");
    println!(
        "  hit@5={:.2} ({}/{})  mean_mrr={:.2}",
        aggregates.hit_rate_at_5,
        aggregates.hit_rate_hits,
        aggregates.hit_rate_applicable,
        aggregates.mean_mrr,
    );
    println!("{}", format_quality_metrics(aggregates));
    println!(
        "  Rendered cost: mean={:.2}  max={}",
        aggregates.rendered_tokens_mean, aggregates.rendered_tokens_max
    );
    println!(
        concat!(
            "  Gates: pass_floor={:.2}  precision_floor={:.2}  forbid={}  ",
            "unmet={}  abstain_fail={}  rendered_over={}"
        ),
        cases_file.pass_floor,
        cases_file.precision_floor,
        aggregates.forbid_violations,
        aggregates.unmet_required,
        aggregates.abstention_failures,
        aggregates.rendered_token_violations,
    );
    match reason {
        Some(reason) => println!("Result: FAIL - {reason}"),
        None => println!("Result: PASS"),
    }
}

pub(super) fn format_quality_metrics(aggregates: &Aggregates) -> String {
    format!(
        "  Quality metrics: p@5={:.2}  rel-tok={:.2}  mand={:.2}  abstention={:.2}",
        aggregates.precision_at_5,
        aggregates.relevant_token_fraction,
        aggregates.mandatory_recall,
        aggregates.abstention_accuracy,
    )
}

fn format_case_line(result: &CaseResult) -> String {
    let hit = if !result.counts_toward_hit_rate {
        "—"
    } else if result.hit_at_5 {
        "✓"
    } else {
        "✗"
    };
    format!(
        "  {:<40}  hit={}  p@5={}  rel-tok={}  mand={}  emit={}  rendered={}",
        inline_safe(&result.name),
        hit,
        optional_metric(result.precision_at_5),
        optional_metric(result.relevant_token_fraction),
        mandatory_display(result),
        if result.would_emit { "yes" } else { "no" },
        result.rendered_tokens,
    )
}

fn optional_metric(value: Option<f32>) -> String {
    value.map_or_else(|| "n/a".to_string(), |value| format!("{value:.2}"))
}

fn mandatory_display(result: &CaseResult) -> String {
    result.mandatory_recall.map_or_else(
        || "n/a".to_string(),
        |recall| format!("{}/{}", recall.present, recall.required),
    )
}

fn print_case_failures(result: &CaseResult) {
    for violation in &result.forbid_violations {
        println!("          forbidden id present: {}", inline_safe(violation));
    }
    for message in case_gate_failures(result) {
        println!("          {message}");
    }
}

fn case_gate_failures(result: &CaseResult) -> Vec<String> {
    let mut failures = Vec::new();
    if result.abstention_correct == Some(false) {
        failures.push("expected abstention but the hook would emit".to_string());
    }
    if result.unmet_required > 0 {
        failures.push(format!("{} required id(s) unmet", result.unmet_required));
    }
    if let Some(limit) = result.max_rendered_tokens {
        if result.rendered_tokens > limit {
            failures.push(format!(
                "rendered {} tokens, over case ceiling {}",
                result.rendered_tokens, limit
            ));
        }
    }
    failures
}

pub(super) fn print_json(
    results: &[CaseResult],
    cases_file: &CasesFile,
    aggregates: &Aggregates,
    reason: Option<&str>,
) -> Result<()> {
    let payload = serde_json::json!({
        "cases": results.iter().map(case_json).collect::<Vec<_>>(),
        "hit_rate_at_5": aggregates.hit_rate_at_5,
        "hit_rate_hits": aggregates.hit_rate_hits,
        "hit_rate_applicable": aggregates.hit_rate_applicable,
        "mean_mrr": aggregates.mean_mrr,
        "precision_at_5": aggregates.precision_at_5,
        "precision_applicable": aggregates.precision_applicable,
        "relevant_token_fraction": aggregates.relevant_token_fraction,
        "relevant_token_fraction_applicable": aggregates.relevant_token_fraction_applicable,
        "mandatory_recall": aggregates.mandatory_recall,
        "mandatory_recall_applicable": aggregates.mandatory_recall_applicable,
        "abstention_accuracy": aggregates.abstention_accuracy,
        "abstention_correct": aggregates.abstention_correct,
        "abstention_applicable": aggregates.abstention_applicable,
        "rendered_tokens_mean": aggregates.rendered_tokens_mean,
        "rendered_tokens_max": aggregates.rendered_tokens_max,
        "forbid_violations": aggregates.forbid_violations,
        "unmet_required": aggregates.unmet_required,
        "abstention_failures": aggregates.abstention_failures,
        "rendered_token_violations": aggregates.rendered_token_violations,
        "pass_floor": cases_file.pass_floor,
        "precision_floor": cases_file.precision_floor,
        "exit_reason": reason,
    });
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(())
}

fn case_json(result: &CaseResult) -> serde_json::Value {
    serde_json::json!({
        "name": result.name,
        "hit_at_5": result.counts_toward_hit_rate.then_some(result.hit_at_5),
        "mrr": result.counts_toward_hit_rate.then_some(result.mrr),
        "precision_at_5": result.precision_at_5,
        "relevant_token_fraction": result.relevant_token_fraction,
        "mandatory_recall": result.mandatory_recall.map(|recall| recall.ratio()),
        "mandatory_present": result.mandatory_recall.map(|recall| recall.present),
        "mandatory_required": result.mandatory_recall.map(|recall| recall.required),
        "unmet_required": result.unmet_required,
        "emit": result.would_emit,
        "abstention_correct": result.abstention_correct,
        "rendered_tokens": result.rendered_tokens,
        "max_rendered_tokens": result.max_rendered_tokens,
        "forbid_violations": result.forbid_violations,
    })
}
