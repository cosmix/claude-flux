use super::cases::EvalCase;
use crate::context::config::RetrievalConfig;
use crate::context::schema::{estimate_tokens, ContextPack};
use crate::orchestrator::signals::format_knowledge_brief;
use std::collections::BTreeSet;

/// Counts behind the mandatory-recall ratio for one case.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct MandatoryRecall {
    pub(super) present: usize,
    pub(super) required: usize,
}

impl MandatoryRecall {
    pub(super) fn ratio(self) -> f32 {
        self.present as f32 / self.required as f32
    }
}

/// One case's pack-derived metrics and gate inputs.
#[derive(Debug, Default)]
pub(super) struct CaseResult {
    pub(super) name: String,
    pub(super) counts_toward_hit_rate: bool,
    pub(super) hit_at_5: bool,
    pub(super) mrr: f32,
    pub(super) precision_at_5: Option<f32>,
    pub(super) relevant_token_fraction: Option<f32>,
    pub(super) mandatory_recall: Option<MandatoryRecall>,
    pub(super) unmet_required: usize,
    pub(super) would_emit: bool,
    pub(super) abstention_correct: Option<bool>,
    pub(super) rendered_tokens: usize,
    pub(super) max_rendered_tokens: Option<usize>,
    pub(super) forbid_violations: Vec<String>,
}

impl CaseResult {
    pub(super) fn exceeds_rendered_limit(&self) -> bool {
        self.max_rendered_tokens
            .is_some_and(|limit| self.rendered_tokens > limit)
    }
}

pub(super) fn score_case(
    case: &EvalCase,
    pack: &ContextPack,
    config: &RetrievalConfig,
) -> CaseResult {
    let item_ids = item_ids(pack);
    let has_relevance = !case.expect.is_empty() || !case.relevant.is_empty();
    let would_emit = crate::commands::hook::user_prompt::would_emit(pack, config);
    CaseResult {
        name: case.name.clone(),
        counts_toward_hit_rate: !case.expect.is_empty(),
        hit_at_5: hit_at_5(&case.expect, &item_ids),
        mrr: mrr(&case.expect, &item_ids),
        precision_at_5: has_relevance
            .then(|| precision_at_5(&case.expect, &case.relevant, &item_ids)),
        relevant_token_fraction: has_relevance
            .then(|| relevant_token_fraction(&case.expect, &case.relevant, pack)),
        mandatory_recall: mandatory_recall(&case.require_ids, &item_ids),
        unmet_required: pack.unmet_required.len(),
        would_emit,
        abstention_correct: case.abstain.then_some(!would_emit),
        rendered_tokens: estimate_tokens(&format_knowledge_brief(pack, None, "eval")),
        max_rendered_tokens: case.max_rendered_tokens,
        forbid_violations: forbid_violations(&case.forbid, &item_ids),
    }
}

fn item_ids(pack: &ContextPack) -> Vec<String> {
    pack.items
        .iter()
        .map(|item| item.id.as_str().to_string())
        .collect()
}

/// True when any expected id is among the first five returned items.
pub(super) fn hit_at_5(expect: &[String], item_ids: &[String]) -> bool {
    item_ids
        .iter()
        .take(5)
        .any(|id| expect.iter().any(|wanted| wanted == id))
}

/// Reciprocal rank of the first expected id, or zero when none was returned.
pub(super) fn mrr(expect: &[String], item_ids: &[String]) -> f32 {
    item_ids
        .iter()
        .position(|id| expect.iter().any(|wanted| wanted == id))
        .map_or(0.0, |index| 1.0 / (index + 1) as f32)
}

/// Fraction of the first five returned positions occupied by judged-relevant ids.
pub(super) fn precision_at_5(expect: &[String], relevant: &[String], item_ids: &[String]) -> f32 {
    let relevant = relevant_ids(expect, relevant);
    let denominator = item_ids.len().min(5);
    if denominator == 0 {
        return 0.0;
    }
    let top_five: BTreeSet<&str> = item_ids.iter().take(5).map(String::as_str).collect();
    top_five.intersection(&relevant).count() as f32 / denominator as f32
}

/// Fraction of the delivered pack estimate spent on judged-relevant items.
pub(super) fn relevant_token_fraction(
    expect: &[String],
    relevant: &[String],
    pack: &ContextPack,
) -> f32 {
    if pack.estimated_tokens == 0 {
        return 0.0;
    }
    let relevant = relevant_ids(expect, relevant);
    let relevant_tokens: usize = pack
        .items
        .iter()
        .filter(|item| relevant.contains(item.id.as_str()))
        .map(|item| item.token_count)
        .sum();
    relevant_tokens as f32 / pack.estimated_tokens as f32
}

/// Distinct required ids present in the returned item ids.
pub(super) fn mandatory_recall(
    require_ids: &[String],
    item_ids: &[String],
) -> Option<MandatoryRecall> {
    let required: BTreeSet<&str> = require_ids.iter().map(String::as_str).collect();
    if required.is_empty() {
        return None;
    }
    let returned: BTreeSet<&str> = item_ids.iter().map(String::as_str).collect();
    let present = required.intersection(&returned).count();
    Some(MandatoryRecall {
        present,
        required: required.len(),
    })
}

/// Every forbidden id present anywhere in the result, in case-file order.
pub(super) fn forbid_violations(forbid: &[String], item_ids: &[String]) -> Vec<String> {
    forbid
        .iter()
        .filter(|wanted| item_ids.iter().any(|id| id == *wanted))
        .cloned()
        .collect()
}

fn relevant_ids<'a>(expect: &'a [String], relevant: &'a [String]) -> BTreeSet<&'a str> {
    expect.iter().chain(relevant).map(String::as_str).collect()
}
