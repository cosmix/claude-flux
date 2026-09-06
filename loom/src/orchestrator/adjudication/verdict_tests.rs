//! Tests for verdict parsing/validation. Split out of `verdict.rs` to keep
//! that file under the maintainability limit (CLAUDE.md Rule 17); the module
//! path is unchanged.

use super::*;

#[test]
fn accept_with_citations_round_trips() {
    let raw = r#"{
        "verdict": "accept",
        "reasoning": "criterion was unreachable",
        "citations": [
            {"file": "src/a.rs", "line": 10, "excerpt": "fn foo()", "claim": "missing function"}
        ],
        "plan_patch": {"stage_id": "x", "field": "acceptance", "patch": {"op": "delete", "index": 1}}
    }"#;
    let out = parse_and_validate(raw);
    match out {
        ValidationOutcome::Verdict(DisputeVerdict::Accept {
            citations,
            reasoning,
            ..
        }) => {
            assert_eq!(citations.len(), 1);
            assert_eq!(reasoning, "criterion was unreachable");
        }
        other => panic!("expected Accept, got {other:?}"),
    }
}

#[test]
fn accept_without_citations_coerced_to_needs_more() {
    let raw = r#"{
        "verdict": "accept",
        "reasoning": "feels right",
        "citations": [],
        "plan_patch": {}
    }"#;
    match parse_and_validate(raw) {
        ValidationOutcome::Verdict(DisputeVerdict::NeedsMoreEvidence { questions }) => {
            assert!(questions[0].contains("citation"));
        }
        other => panic!("expected NeedsMoreEvidence, got {other:?}"),
    }
}

/// The wedge this module exists to fix: three adjudicators emitted a FLAT
/// `plan_patch` (`op`/`index`/`value` as siblings of `field`, not nested
/// under `patch`). It must still validate as Accept, and what lands on disk
/// must be the canonical nested shape so `apply.rs` never has to normalise a
/// second time.
#[test]
fn flat_plan_patch_yields_accept_with_canonical_stored_shape() {
    let raw = r#"{
        "verdict": "accept",
        "reasoning": "criterion was overspecified",
        "citations": [
            {"file": "src/a.rs", "line": 10, "excerpt": "fn foo()", "claim": "missing function"}
        ],
        "plan_patch": {
            "stage_id": "s1",
            "field": "acceptance",
            "op": "replace",
            "index": 2,
            "value": "loom knowledge check",
            "reason": "criterion was wrong"
        }
    }"#;
    match parse_and_validate(raw) {
        ValidationOutcome::Verdict(DisputeVerdict::Accept { plan_patch, .. }) => {
            assert_eq!(
                plan_patch.inner,
                serde_json::json!({
                    "field": "acceptance",
                    "patch": {"op": "replace", "index": 2, "value": "loom knowledge check"},
                    "reason": "criterion was wrong",
                }),
                "stored plan_patch must be the canonical nested shape: {:?}",
                plan_patch.inner
            );
        }
        other => panic!("expected Accept, got {other:?}"),
    }
}

/// A `plan_patch` malformed under BOTH accepted shapes must re-prompt rather
/// than wedge — this is what lets an already-broken verdict on disk self-heal
/// instead of retrying the same failure forever.
#[test]
fn plan_patch_malformed_under_both_shapes_needs_more_evidence() {
    let raw = r#"{
        "verdict": "accept",
        "reasoning": "criterion was overspecified",
        "citations": [
            {"file": "src/a.rs", "line": 10, "excerpt": "fn foo()", "claim": "missing function"}
        ],
        "plan_patch": {"field": "acceptance"}
    }"#;
    match parse_and_validate(raw) {
        ValidationOutcome::Verdict(DisputeVerdict::NeedsMoreEvidence { questions }) => {
            assert!(
                questions[0].contains("plan_patch"),
                "question must name plan_patch: {questions:?}"
            );
        }
        other => panic!("expected NeedsMoreEvidence, got {other:?}"),
    }
}

#[test]
fn reject_with_citations() {
    let raw = r#"{
        "verdict": "reject",
        "reasoning": "criterion is right",
        "citations": [{"file":"a","excerpt":"e","claim":"c"}]
    }"#;
    match parse_and_validate(raw) {
        ValidationOutcome::Verdict(DisputeVerdict::Reject { citations, .. }) => {
            assert_eq!(citations.len(), 1);
            assert!(citations[0].line.is_none());
        }
        other => panic!("expected Reject, got {other:?}"),
    }
}

#[test]
fn needs_more_evidence_with_questions() {
    let raw = r#"{"verdict": "needs-more-evidence", "questions": ["what is X?"]}"#;
    match parse_and_validate(raw) {
        ValidationOutcome::Verdict(DisputeVerdict::NeedsMoreEvidence { questions }) => {
            assert_eq!(questions, vec!["what is X?".to_string()]);
        }
        other => panic!("expected NeedsMoreEvidence, got {other:?}"),
    }
}

#[test]
fn needs_more_evidence_with_no_questions_escalates() {
    let raw = r#"{"verdict": "needs-more-evidence", "questions": []}"#;
    match parse_and_validate(raw) {
        ValidationOutcome::Escalate { reason } => {
            assert!(reason.contains("pathological"));
        }
        other => panic!("expected Escalate, got {other:?}"),
    }
}

#[test]
fn unknown_verdict_tag_coerces_to_needs_more() {
    let raw = r#"{"verdict": "bogus"}"#;
    match parse_and_validate(raw) {
        ValidationOutcome::Verdict(DisputeVerdict::NeedsMoreEvidence { questions }) => {
            assert!(questions[0].contains("unknown verdict tag"));
        }
        other => panic!("expected NeedsMoreEvidence, got {other:?}"),
    }
}

#[test]
fn malformed_json_coerces_to_needs_more() {
    let raw = "not json at all";
    match parse_and_validate(raw) {
        ValidationOutcome::Verdict(DisputeVerdict::NeedsMoreEvidence { questions }) => {
            assert!(
                questions[0].contains("not valid JSON"),
                "got: {:?}",
                questions
            );
        }
        other => panic!("expected NeedsMoreEvidence, got {other:?}"),
    }
}

#[test]
fn fenced_json_is_accepted() {
    let raw = "```json\n{\"verdict\":\"reject\",\"reasoning\":\"r\",\"citations\":[{\"file\":\"f\",\"excerpt\":\"e\",\"claim\":\"c\"}]}\n```";
    assert!(matches!(
        parse_and_validate(raw),
        ValidationOutcome::Verdict(DisputeVerdict::Reject { .. })
    ));
}

#[test]
fn json_with_leading_prose_extracts_first_object() {
    let raw = "Sure! Here is the verdict:\n{\"verdict\":\"reject\",\"reasoning\":\"r\",\"citations\":[{\"file\":\"f\",\"excerpt\":\"e\",\"claim\":\"c\"}]}\nLet me know if you want more.";
    assert!(matches!(
        parse_and_validate(raw),
        ValidationOutcome::Verdict(DisputeVerdict::Reject { .. })
    ));
}
