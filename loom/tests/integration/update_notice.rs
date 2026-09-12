//! An update notice must never leak into `--json` stdout, and a dev build
//! must never print one at all.

use std::fs;
use tempfile::TempDir;

use super::helpers::loom_cmd;

/// A minimal valid plan (standard stage with acceptance, no artifacts).
fn minimal_valid_plan(name: &str) -> String {
    format!(
        r#"# {name}

<!-- loom METADATA -->

```yaml
loom:
  version: 1
  stages:
    - id: stage-one
      name: "Stage One"
      stage_type: standard
      working_dir: "."
      acceptance:
        - "true"
```

<!-- END loom METADATA -->
"#
    )
}

#[test]
fn test_dev_build_prints_no_update_notice_and_json_stdout_stays_pure() {
    // Two invariants at once. An update notice only ever goes to stderr
    // (`update_check::notify_and_maybe_refresh`'s `eprintln!`), so `--json`
    // stdout stays pure JSON whatever the update state says. And the binary
    // under test is a dev build (`build.rs` derives a prerelease version for
    // anything but a tagged release), which `update_check::decide` exempts
    // from the notice altogether: a dev build can never act on it, so even a
    // far-future release on record must print nothing on either stream.
    // `loom_cmd()`'s shared scratch `LOOM_HOME` opts out of the check
    // entirely (`check = false`); this test deliberately opts back in with
    // its own scratch home. Its `last_checked` stamp is "now" so `decide()`
    // would never schedule a detached refresh fetch either, were the dev-build
    // exemption ever lost — that would be a real network spawn this test must
    // not trigger.
    let loom_home = TempDir::new().unwrap();
    let state = format!(
        r#"{{"last_checked":"{}","latest_version":"99.0.0"}}"#,
        chrono::Utc::now().to_rfc3339()
    );
    fs::write(loom_home.path().join("update-state.json"), state).unwrap();

    let temp = TempDir::new().unwrap();
    let plan = temp.path().join("PLAN-update-notice.md");
    fs::write(&plan, minimal_valid_plan("Update Notice Plan")).unwrap();

    let out = loom_cmd()
        .env("LOOM_HOME", loom_home.path())
        .args(["plan", "verify", "--json"])
        .arg(&plan)
        .output()
        .expect("failed to run loom plan verify");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    serde_json::from_str::<serde_json::Value>(&stdout)
        .expect("stdout must be pure JSON even with a newer release on record");
    assert!(!stdout.contains("loom update"), "stdout: {stdout}");
    assert!(
        !stderr.contains("loom update"),
        "a dev build must not print the update notice, got: {stderr}"
    );
}
