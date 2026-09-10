//! Pure tests for `commands/knowledge/eval`; none opens the live index.

use super::cases::{
    build_query_text, resolve_budget, validate_cases, CasesFile, EvalCase, EvalMode,
    DEFAULT_PROMPT_BUDGET_TOKENS, DEFAULT_STAGE_BUDGET_TOKENS,
};
use super::metrics::{
    forbid_violations, hit_at_5, mandatory_recall, mrr, precision_at_5, relevant_token_fraction,
    score_case, CaseResult,
};
use super::report::{aggregate, exit_reason, format_quality_metrics, Aggregates};
use crate::context::config::RetrievalConfig;
use crate::context::schema::{
    Channel, ChunkId, Confidence, ContextItem, ContextPack, Freshness, ItemKind, LifecycleState,
    OmissionSummary, SelectionReason, SourcePointer, UnmetRequirement,
};
use std::path::PathBuf;

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn case(name: &str, query: &str, expect: &[&str], forbid: &[&str]) -> EvalCase {
    EvalCase {
        name: name.to_string(),
        query: query.to_string(),
        mode: EvalMode::Prompt,
        budget_tokens: None,
        stage_fields: Vec::new(),
        require_ids: Vec::new(),
        expect: ids(expect),
        relevant: Vec::new(),
        forbid: ids(forbid),
        abstain: false,
        max_rendered_tokens: None,
    }
}

fn cases_file(cases: Vec<EvalCase>) -> CasesFile {
    CasesFile {
        pass_floor: 0.5,
        precision_floor: 0.0,
        cases,
    }
}

fn item(id: &str, token_count: usize) -> ContextItem {
    ContextItem {
        id: ChunkId::from(id),
        kind: ItemKind::KnowledgeChunk,
        pointer: SourcePointer {
            path: PathBuf::from("doc/loom/knowledge/test.md"),
            anchor: "test".to_string(),
            line_start: None,
            line_end: None,
        },
        summary: id.to_string(),
        source: Channel::Knowledge,
        token_count,
        score: 1.0,
        reasons: vec![SelectionReason::Lexical],
        confidence: Confidence::Low,
        state: LifecycleState::Active,
        content_hash: format!("sha256:{id}"),
        excerpt: Some(format!("## {id}\n\nfixture")),
        truncated: false,
        matched_term_count: 0,
    }
}

fn pack(items: Vec<ContextItem>, estimated_tokens: usize) -> ContextPack {
    ContextPack {
        query: "query".to_string(),
        scope: vec![Channel::Knowledge],
        budget_tokens: 1600,
        estimated_tokens,
        structural_freshness: Freshness::default(),
        semantic_freshness: Freshness::default(),
        items,
        unmet_required: Vec::new(),
        omitted: OmissionSummary::default(),
        dropped_terms: Vec::new(),
        degraded: None,
    }
}

#[test]
fn parses_a_valid_cases_file() {
    let yaml = r#"
pass_floor: 0.6
precision_floor: 0.2
cases:
  - name: alpha
    query: "find the widget"
    expect: ["a.md#b#0"]
    relevant: ["a.md#c#0"]
  - name: beta
    query: "stage query"
    mode: stage
    stage_fields: ["src/foo.rs"]
    forbid: ["noisy#id"]
"#;
    let parsed: CasesFile = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(parsed.pass_floor, 0.6);
    assert_eq!(parsed.precision_floor, 0.2);
    assert_eq!(parsed.cases.len(), 2);
    assert_eq!(parsed.cases[0].expect, ids(&["a.md#b#0"]));
    assert_eq!(parsed.cases[0].relevant, ids(&["a.md#c#0"]));
    assert_eq!(parsed.cases[1].mode, EvalMode::Stage);
    assert_eq!(parsed.cases[1].stage_fields, ids(&["src/foo.rs"]));
    validate_cases(&parsed).unwrap();
}

#[test]
fn default_floors_apply_when_absent() {
    let yaml = "cases:\n  - name: a\n    query: q\n    expect: [\"x\"]\n";
    let parsed: CasesFile = serde_yaml::from_str(yaml).unwrap();
    assert_eq!((parsed.pass_floor, parsed.precision_floor), (0.5, 0.0));
}

#[test]
fn validate_cases_rejects_a_duplicate_name() {
    let file = cases_file(vec![
        case("dup", "q1", &["x"], &[]),
        case("dup", "q2", &["y"], &[]),
    ]);
    assert!(validate_cases(&file)
        .unwrap_err()
        .to_string()
        .contains("dup"));
}

#[test]
fn validate_cases_rejects_an_empty_query() {
    let file = cases_file(vec![case("blank", "   ", &["x"], &[])]);
    assert!(validate_cases(&file)
        .unwrap_err()
        .to_string()
        .contains("blank"));
}

#[test]
fn validate_cases_rejects_neither_expect_forbid_nor_abstain() {
    let file = cases_file(vec![case("toothless", "q", &[], &[])]);
    assert!(validate_cases(&file)
        .unwrap_err()
        .to_string()
        .contains("toothless"));
}

#[test]
fn validate_cases_rejects_abstain_with_expect() {
    let mut invalid = case("contradiction", "q", &["x"], &[]);
    invalid.abstain = true;
    let error = validate_cases(&cases_file(vec![invalid])).unwrap_err();
    assert!(error.to_string().contains("abstain: true"));
}

#[test]
fn hit_at_5_only_looks_at_the_first_five_items() {
    let expect = ids(&["target"]);
    assert!(!hit_at_5(
        &expect,
        &ids(&["a", "b", "c", "d", "e", "target"])
    ));
    assert!(hit_at_5(
        &expect,
        &ids(&["a", "b", "target", "d", "e", "f"])
    ));
}

#[test]
fn mrr_is_the_reciprocal_of_the_first_matching_rank() {
    assert_eq!(mrr(&ids(&["target"]), &ids(&["a", "target", "b"])), 0.5);
}

#[test]
fn mrr_is_zero_when_expect_never_appears() {
    assert_eq!(mrr(&ids(&["target"]), &ids(&["a", "b"])), 0.0);
}

#[test]
fn forbid_violations_lists_every_forbidden_id_present() {
    assert_eq!(
        forbid_violations(
            &ids(&["bad1", "bad2", "absent"]),
            &ids(&["bad1", "ok", "bad2"])
        ),
        ids(&["bad1", "bad2"])
    );
}

#[test]
fn precision_at_5_counts_only_relevant_items_among_the_first_five() {
    let precision = precision_at_5(
        &ids(&["target"]),
        &[],
        &ids(&["target", "a", "b", "c", "d"]),
    );
    let aggregates = aggregate(&[CaseResult {
        precision_at_5: Some(precision),
        ..CaseResult::default()
    }]);
    assert_eq!(precision, 0.2);
    assert_eq!(
        format_quality_metrics(&aggregates),
        "  Quality metrics: p@5=0.20  rel-tok=0.00  mand=0.00  abstention=0.00"
    );
}

#[test]
fn hit_rate_and_precision_disagree_on_a_one_relevant_pack() {
    let eval_case = case("one", "q", &["target"], &[]);
    let result = score_case(
        &eval_case,
        &pack(
            ["target", "a", "b", "c", "d"]
                .into_iter()
                .map(|id| item(id, 10))
                .collect(),
            100,
        ),
        &RetrievalConfig::default(),
    );
    let aggregates = aggregate(&[result]);
    assert_eq!(
        (aggregates.hit_rate_at_5, aggregates.precision_at_5),
        (1.0, 0.2)
    );
}

#[test]
fn relevant_token_fraction_is_zero_for_an_empty_pack() {
    assert_eq!(
        relevant_token_fraction(&ids(&["target"]), &[], &pack(Vec::new(), 0)),
        0.0
    );
}

#[test]
fn mandatory_recall_counts_present_require_ids() {
    let recall = mandatory_recall(&ids(&["a", "b"]), &ids(&["b", "c"])).unwrap();
    assert_eq!(
        (recall.present, recall.required, recall.ratio()),
        (1, 2, 0.5)
    );
}

#[test]
fn aggregate_excludes_forbid_only_cases_from_hit_rate() {
    let results = vec![
        CaseResult {
            counts_toward_hit_rate: true,
            hit_at_5: true,
            mrr: 1.0,
            ..CaseResult::default()
        },
        CaseResult {
            forbid_violations: ids(&["x"]),
            ..CaseResult::default()
        },
    ];
    let aggregates = aggregate(&results);
    assert_eq!(
        (aggregates.hit_rate_applicable, aggregates.hit_rate_at_5),
        (1, 1.0)
    );
    assert_eq!(aggregates.forbid_violations, 1);
}

#[test]
fn exit_reason_is_none_when_scores_clear_floors_and_nothing_is_forbidden() {
    let aggregates = Aggregates {
        hit_rate_at_5: 0.75,
        precision_at_5: 0.4,
        ..Aggregates::default()
    };
    assert!(exit_reason(&aggregates, 0.5, 0.2).is_none());
}

#[test]
fn exit_reason_names_a_low_hit_rate() {
    let aggregates = Aggregates {
        hit_rate_at_5: 0.2,
        ..Aggregates::default()
    };
    assert!(exit_reason(&aggregates, 0.5, 0.0)
        .unwrap()
        .contains("hit@5"));
}

#[test]
fn exit_reason_names_a_low_true_precision_score() {
    let aggregates = Aggregates {
        hit_rate_at_5: 1.0,
        precision_at_5: 0.2,
        ..Aggregates::default()
    };
    assert!(exit_reason(&aggregates, 0.5, 0.4)
        .unwrap()
        .contains("precision@5"));
}

#[test]
fn exit_reason_names_a_forbid_violation_even_with_perfect_hit_rate() {
    let aggregates = Aggregates {
        hit_rate_at_5: 1.0,
        forbid_violations: 3,
        ..Aggregates::default()
    };
    assert!(exit_reason(&aggregates, 0.5, 0.0)
        .unwrap()
        .contains("forbid violation"));
}

#[test]
fn an_abstain_case_fails_when_the_hook_would_emit() {
    let mut abstain = case("quiet", "q", &[], &[]);
    abstain.abstain = true;
    let mut strong = item("strong", 10);
    strong.reasons = vec![SelectionReason::ExactSymbol];
    let result = score_case(
        &abstain,
        &pack(vec![strong], 100),
        &RetrievalConfig::default(),
    );
    let reason = exit_reason(&aggregate(&[result]), 0.0, 0.0).unwrap();
    assert!(reason.contains("abstention case"));
}

#[test]
fn an_unmet_required_id_fails_the_run() {
    let mut required = case("required", "q", &[], &["noise"]);
    required.require_ids = ids(&["must-have"]);
    let mut missing = pack(Vec::new(), 128);
    missing.unmet_required.push(UnmetRequirement {
        id: "must-have".to_string(),
        needed_tokens: 200,
        available_tokens: 20,
        reason: "required representation exceeds budget".to_string(),
    });
    let result = score_case(&required, &missing, &RetrievalConfig::default());
    assert!(exit_reason(&aggregate(&[result]), 0.0, 0.0)
        .unwrap()
        .contains("required id"));
}

#[test]
fn max_rendered_tokens_gates_a_case() {
    let mut limited = case("limited", "q", &[], &["noise"]);
    limited.max_rendered_tokens = Some(0);
    let result = score_case(&limited, &pack(Vec::new(), 0), &RetrievalConfig::default());
    assert!(exit_reason(&aggregate(&[result]), 0.0, 0.0)
        .unwrap()
        .contains("rendered-token ceiling"));
}

#[test]
fn build_query_text_ignores_stage_fields_in_prompt_mode() {
    let mut prompt = case("p", "just the prompt", &["x"], &[]);
    prompt.stage_fields = ids(&["ignored"]);
    assert_eq!(build_query_text(&prompt), "just the prompt");
}

#[test]
fn build_query_text_folds_stage_fields_in_stage_mode() {
    let mut stage = case("s", "the query", &["x"], &[]);
    stage.mode = EvalMode::Stage;
    stage.stage_fields = ids(&["src/a.rs", "src/b.rs"]);
    assert_eq!(build_query_text(&stage), "the query\nsrc/a.rs\nsrc/b.rs");
}

#[test]
fn resolve_budget_prefers_cli_then_case_then_mode_default() {
    let mut prompt = case("b", "q", &["x"], &[]);
    assert_eq!(resolve_budget(&prompt, None), DEFAULT_PROMPT_BUDGET_TOKENS);
    prompt.budget_tokens = Some(999);
    assert_eq!(resolve_budget(&prompt, None), 999);
    assert_eq!(resolve_budget(&prompt, Some(42)), 42);
}

#[test]
fn resolve_budget_defaults_stage_mode_higher_than_prompt_mode() {
    let mut stage = case("s", "q", &["x"], &[]);
    stage.mode = EvalMode::Stage;
    assert_eq!(resolve_budget(&stage, None), DEFAULT_STAGE_BUDGET_TOKENS);
}
