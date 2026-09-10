use super::*;
use crate::context::local_overlay::{local_overlay_key, OverlayScope};
use crate::context::refresh::SnapshotPolicy;
use crate::models::stage::Stage;
use serial_test::serial;
use std::path::Path;
use tempfile::TempDir;

fn mapped_checkout() -> TempDir {
    let temp = TempDir::new().unwrap();
    std::fs::create_dir_all(temp.path().join(".loom").join("work")).unwrap();
    temp
}

fn enter(root: &Path, stage_id: Option<&str>) {
    std::env::set_var("LOOM_WORK_DIR", root);
    match stage_id {
        Some(stage_id) => std::env::set_var("LOOM_STAGE_ID", stage_id),
        None => std::env::remove_var("LOOM_STAGE_ID"),
    }
}

fn leave() {
    std::env::remove_var("LOOM_STAGE_ID");
    std::env::remove_var("LOOM_WORK_DIR");
}

#[test]
#[serial]
fn a_real_stage_resolves_one_stage_overlay_and_snapshot_policy() {
    let temp = mapped_checkout();
    let work_dir = temp.path().join(".loom").join("work");
    let stage = Stage {
        id: "stage-a".to_string(),
        name: "Stage A".to_string(),
        plan_id: Some("test-plan".to_string()),
        ..Stage::default()
    };
    crate::verify::transitions::create_stage(&stage, &work_dir).unwrap();
    enter(&work_dir, Some(&stage.id));

    let target = HookTarget::from_environment();

    leave();
    let target = target.expect("the stage record resolves");
    assert_eq!(target.work_dir, work_dir);
    assert_eq!(target.project_root.as_path(), temp.path());
    assert_eq!(target.plan, "test-plan");
    assert_eq!(target.stage_id, "stage-a");
    assert_eq!(target.pull_stage.as_deref(), Some("stage-a"));
    assert_eq!(
        target.overlay,
        OverlayScope::Stage {
            plan: "test-plan".to_string(),
            stage: "stage-a".to_string(),
        }
    );
    assert_eq!(
        target.snapshot_policy(),
        SnapshotPolicy::StageOverlay {
            plan: "test-plan".to_string(),
            stage: "stage-a".to_string(),
        }
    );
    assert!(target.exists());
}

#[test]
#[serial]
fn a_checkout_resolves_the_local_overlay_and_snapshot_policy() {
    let temp = mapped_checkout();
    enter(temp.path(), None);

    let target = HookTarget::from_environment();

    leave();
    let target = target.expect("the checkout resolves");
    let (plan, stage_id) = local_overlay_key(temp.path());
    assert_eq!(target.plan, plan);
    assert_eq!(target.stage_id, stage_id);
    assert_eq!(target.pull_stage, None);
    assert_eq!(target.overlay, OverlayScope::Local);
    assert_eq!(target.snapshot_policy(), SnapshotPolicy::LocalCurrent);
    assert!(target.exists());
}

#[test]
#[serial]
fn invalid_or_missing_stage_records_fall_back_to_the_checkout() {
    let temp = mapped_checkout();

    for stage_id in ["../escape", "missing-stage"] {
        enter(temp.path(), Some(stage_id));
        let target = HookTarget::from_environment().expect("checkout fallback");
        assert_eq!(target.pull_stage, None);
        assert_eq!(target.overlay, OverlayScope::Local);
    }

    leave();
}

#[test]
#[serial]
fn a_checkout_target_can_name_a_state_root_that_does_not_exist_yet() {
    let temp = TempDir::new().unwrap();
    enter(temp.path(), None);

    let target = HookTarget::from_environment();

    leave();
    let target = target.expect("a checkout does not require initialized state");
    assert!(!target.exists());
    assert_eq!(target.work_dir, temp.path().join(".loom").join("work"));
}
