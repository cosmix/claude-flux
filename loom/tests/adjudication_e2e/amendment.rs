//! Amendment-application happy-path tests for adjudication, split out of the
//! parent module (`adjudication_e2e.rs`) to keep it under the
//! maintainability line limit. `use super::*;` reaches every fixture in the
//! parent — Rust visibility lets a child module see its ancestors' private
//! items.

use super::*;

/// The FLAT `plan_patch` shape three real adjudicators emitted instead of the
/// nested shape [`verdict_accept_delete_first`] uses: `op`/`index` sit as
/// siblings of `field` rather than nested under `patch`. Same dispute target
/// as `verdict_accept_delete_first` — deletes acceptance[0] on
/// [`make_stage_two_criteria`] — so the two tests can assert identical
/// outcomes.
fn verdict_accept_delete_first_flat() -> serde_json::Value {
    serde_json::json!({
        "verdict": "accept",
        "reasoning": "acceptance[0] references a path that cannot exist; criterion has no valid interpretation",
        "citations": [
            {
                "file": "PLAN.md",
                "line": 1,
                "excerpt": "intentionally_wrong",
                "claim": "path does not exist in the project root"
            }
        ],
        "plan_patch": {
            "stage_id": "s1",
            "field": "acceptance",
            "op": "delete",
            "index": 0,
            "reason": "criterion path is mechanically wrong"
        }
    })
}

/// Shared assertions for the amendment-applied happy path, covering both
/// `plan_patch` shapes: the plan-version snapshot, the audit row, the live
/// plan file, and the stage's resulting state. `shape` names which shape is
/// under test so a failed assertion says which one broke.
fn assert_amendment_applied(work: &Path, plan: &Path, shape: &str) {
    let snapshot = work.join("plan_versions").join("1.md");
    assert!(
        snapshot.exists(),
        "plan_versions/1.md must exist after the {shape} amendment",
    );

    let audit = std::fs::read_to_string(work.join("plan_versions").join("audit.md"))
        .unwrap_or_else(|_| panic!("audit.md must exist after the {shape} amendment"));
    assert!(
        audit.contains("s1"),
        "audit row must mention stage_id 's1' — audit:\n{audit}",
    );
    assert!(
        audit.contains("delete"),
        "audit row must record the patch op — audit:\n{audit}",
    );

    let live_plan = std::fs::read_to_string(plan).unwrap();
    assert!(
        !live_plan.contains("intentionally_wrong"),
        "live plan must no longer contain the deleted criterion ({shape})",
    );

    let after = loom::verify::transitions::load_stage("s1", work).unwrap();
    assert_eq!(after.status, StageStatus::Queued);
    assert_eq!(
        after.acceptance.len(),
        1,
        "acceptance[0] should be deleted, leaving 1 criterion ({shape})",
    );
    match &after.acceptance[0] {
        AcceptanceCriterion::Simple(cmd) => assert_eq!(cmd, "ls /tmp"),
        other => panic!("expected Simple criterion, got {other:?}"),
    }
    assert_eq!(
        after.amendments_applied, 1,
        "stage.amendments_applied must increment to 1 ({shape})",
    );

    assert!(applied_marker(&work.join("disputes"), "s1", 1).exists());
}

/// True end-to-end coverage of the autonomous-criteria-adjudication
/// happy path:
///
/// 1. Stage `s1` has a mechanically wrong acceptance criterion at
///    index 0 plus a passing one at index 1.
/// 2. A dispute is filed against index 0.
/// 3. The adjudication session records an Accept verdict whose
///    `plan_patch` deletes acceptance[0].
/// 4. `apply_pending_verdicts` MUST succeed (no silent fallthrough).
/// 5. Assertions cover every observable side-effect of a successful
///    amendment: `plan_versions/1.md` snapshot, `audit.md` row, live
///    plan file rewritten, stage transitions back to `Queued`, and
///    `amendments_applied` is incremented.
#[test]
fn dispute_to_amendment_to_pass() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path();
    let plan = write_plan_with_metadata_markers(work);
    write_stage(work, &make_stage_two_criteria("s1"));
    write_dispute(work, "s1", 1);

    let reg = AdjudicatorRegistry::new();
    drive_dispute(&reg, work, "s1", 1, &verdict_accept_delete_first());

    assert_amendment_applied(work, &plan, "nested plan_patch");

    // Prose around the YAML block must survive the splice.
    let live_plan = std::fs::read_to_string(&plan).unwrap();
    assert!(
        live_plan.contains("Prose section that must be preserved"),
        "leading prose must survive amendment",
    );
    assert!(
        live_plan.contains("Trailing prose section"),
        "trailing prose must survive amendment",
    );
}

/// The same happy path as `dispute_to_amendment_to_pass`, but with the FLAT
/// `plan_patch` shape (`op`/`index` as siblings of `field`) real adjudicators
/// emitted instead of the nested shape the schema described. The amendment
/// must apply identically: same snapshot, same audit row, same requeue.
#[test]
fn dispute_to_amendment_to_pass_with_flat_plan_patch() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path();
    let plan = write_plan_with_metadata_markers(work);
    write_stage(work, &make_stage_two_criteria("s1"));
    write_dispute(work, "s1", 1);

    let reg = AdjudicatorRegistry::new();
    drive_dispute(&reg, work, "s1", 1, &verdict_accept_delete_first_flat());

    assert_amendment_applied(work, &plan, "flat plan_patch");
}
