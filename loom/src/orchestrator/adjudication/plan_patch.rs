//! Normalising an adjudicator's `plan_patch` into an [`AmendmentRequest`].
//!
//! Three separate adjudicator sessions all emitted the same FLAT shape —
//! `{"field": ..., "op": ..., "index": ..., "value": ..., "reason": ...}` —
//! instead of the nested `{"field": ..., "patch": {"op": ...}}` shape
//! [`AmendmentRequest`] actually decodes. The prompt only described the shape
//! as "AmendmentRequest JSON", a Rust type the model cannot see, so every
//! adjudicator guessed the same flat shape independently.
//!
//! [`normalize`] accepts both, so a verdict already stuck on disk self-heals
//! on the next apply attempt with no state surgery, and `verdict.rs` accepts
//! both at record time so a fresh verdict never wedges in the first place.

use anyhow::Result;
use serde_json::{Map, Value};

use crate::models::dispute::PlanPatch;
use crate::plan::amendment::{AmendmentField, AmendmentPatch, AmendmentRequest};

/// Build an [`AmendmentRequest`] from a recorded verdict's `plan_patch`,
/// accepting either the nested `{"field", "patch": {...}}` shape or the flat
/// `{"field", "op", "index", "value"}` shape emitted in practice. Same
/// behaviour as before this module existed for the nested shape.
pub(super) fn build_amendment_request(
    stage_id: String,
    plan_patch: &PlanPatch,
    dispute_id: u32,
) -> Result<AmendmentRequest> {
    let (field, patch, reason) =
        normalize(&plan_patch.inner).map_err(|msg| anyhow::anyhow!(msg))?;
    Ok(AmendmentRequest {
        stage_id,
        field,
        patch,
        reason,
        dispute_id: Some(dispute_id.to_string()),
    })
}

/// Decode `inner` into `(field, patch, reason)`, accepting either shape.
///
/// Returns a human-readable error STRING rather than `anyhow::Error`:
/// `verdict.rs` needs the message verbatim inside a re-prompt question, and
/// `apply.rs` wraps it with `anyhow::anyhow!` for its own error path.
pub(super) fn normalize(
    inner: &Value,
) -> Result<(AmendmentField, AmendmentPatch, Option<String>), String> {
    let field = match inner.get("field").and_then(|v| v.as_str()) {
        Some("acceptance") => AmendmentField::Acceptance,
        Some("wiring") => AmendmentField::Wiring,
        Some(other) => {
            return Err(format!(
                "plan_patch field '{other}' must be acceptance|wiring"
            ))
        }
        None => return Err("plan_patch missing 'field' string".to_string()),
    };

    let patch: AmendmentPatch = if let Some(patch_obj) = inner.get("patch") {
        serde_json::from_value(patch_obj.clone())
            .map_err(|e| format!("nested 'patch' object malformed: {e}"))?
    } else if inner.get("op").and_then(|v| v.as_str()).is_some() {
        // Collect the sibling op/index/value keys into a fresh object and
        // decode THAT through the same AmendmentPatch deserializer, rather
        // than matching op names by hand — this keeps op-name handling and
        // error messages identical between the two shapes.
        let mut map = Map::new();
        if let Some(op) = inner.get("op") {
            map.insert("op".to_string(), op.clone());
        }
        if let Some(index) = inner.get("index") {
            map.insert("index".to_string(), index.clone());
        }
        if let Some(value) = inner.get("value") {
            map.insert("value".to_string(), value.clone());
        }
        serde_json::from_value(Value::Object(map))
            .map_err(|e| format!("flat 'op'/'index'/'value' shape malformed: {e}"))?
    } else {
        return Err(
            "plan_patch has neither a nested 'patch' object nor a top-level 'op'; expected \
             {\"field\": \"acceptance\", \"patch\": {\"op\": \"replace\", \"index\": 0, \"value\": \"<YAML>\"}}"
                .to_string(),
        );
    };

    let reason = inner
        .get("reason")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok((field, patch, reason))
}

/// Rebuild the canonical nested `{"field", "patch": {...}, "reason"}` object
/// from decoded parts, so what lands on disk is always the nested shape
/// regardless of which shape the adjudicator emitted.
pub(super) fn canonical_inner(
    field: AmendmentField,
    patch: &AmendmentPatch,
    reason: Option<&str>,
) -> Value {
    let mut map = Map::new();
    map.insert(
        "field".to_string(),
        serde_json::to_value(field).expect("AmendmentField always serializes"),
    );
    map.insert(
        "patch".to_string(),
        serde_json::to_value(patch).expect("AmendmentPatch always serializes"),
    );
    if let Some(reason) = reason {
        map.insert("reason".to_string(), Value::String(reason.to_string()));
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_patch(v: Value) -> PlanPatch {
        PlanPatch { inner: v }
    }

    #[test]
    fn nested_shape_decodes() {
        let patch = plan_patch(serde_json::json!({
            "field": "acceptance",
            "patch": {"op": "delete", "index": 0},
            "reason": "wrong",
        }));
        let req = build_amendment_request("s1".to_string(), &patch, 1).unwrap();
        assert_eq!(req.field, AmendmentField::Acceptance);
        assert_eq!(req.patch, AmendmentPatch::Delete { index: 0 });
        assert_eq!(req.reason.as_deref(), Some("wrong"));
    }

    #[test]
    fn flat_shape_decodes_to_identical_request() {
        let nested = plan_patch(serde_json::json!({
            "field": "acceptance",
            "patch": {"op": "replace", "index": 2, "value": "loom knowledge check"},
            "reason": "criterion wrong",
        }));
        let flat = plan_patch(serde_json::json!({
            "stage_id": "s1",
            "field": "acceptance",
            "op": "replace",
            "index": 2,
            "value": "loom knowledge check",
            "reason": "criterion wrong",
        }));
        let nested_req = build_amendment_request("s1".to_string(), &nested, 1).unwrap();
        let flat_req = build_amendment_request("s1".to_string(), &flat, 1).unwrap();
        assert_eq!(nested_req.stage_id, flat_req.stage_id);
        assert_eq!(nested_req.field, flat_req.field);
        assert_eq!(nested_req.patch, flat_req.patch);
        assert_eq!(nested_req.reason, flat_req.reason);
    }

    #[test]
    fn flat_delete_without_value_decodes() {
        let flat = plan_patch(serde_json::json!({
            "field": "wiring",
            "op": "delete",
            "index": 3,
        }));
        let req = build_amendment_request("s1".to_string(), &flat, 5).unwrap();
        assert_eq!(req.field, AmendmentField::Wiring);
        assert_eq!(req.patch, AmendmentPatch::Delete { index: 3 });
    }

    #[test]
    fn neither_shape_present_errors_naming_both() {
        let patch = plan_patch(serde_json::json!({"field": "acceptance"}));
        let err = normalize(&patch.inner).unwrap_err();
        assert!(err.contains("patch"), "error: {err}");
        assert!(err.contains("op"), "error: {err}");
    }

    #[test]
    fn unknown_field_errors() {
        let patch = plan_patch(serde_json::json!({
            "field": "bogus",
            "patch": {"op": "delete", "index": 0},
        }));
        let err = normalize(&patch.inner).unwrap_err();
        assert!(err.contains("bogus"), "error: {err}");
    }

    #[test]
    fn canonical_inner_round_trips_through_normalize() {
        let field = AmendmentField::Acceptance;
        let patch = AmendmentPatch::Replace {
            index: 1,
            value: "cargo test".to_string(),
        };
        let inner = canonical_inner(field, &patch, Some("because"));
        let (out_field, out_patch, out_reason) = normalize(&inner).unwrap();
        assert_eq!(out_field, field);
        assert_eq!(out_patch, patch);
        assert_eq!(out_reason.as_deref(), Some("because"));
    }
}
