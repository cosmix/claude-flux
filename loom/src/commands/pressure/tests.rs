use super::paths::*;
use super::spawn::*;
use super::*;
use std::fs;
use tempfile::TempDir;

fn canonical(dir: &TempDir) -> PathBuf {
    dir.path().canonicalize().unwrap()
}

#[test]
fn test_resolve_plan_path_direct_relative() {
    let temp = TempDir::new().unwrap();
    let root = canonical(&temp);
    let plans = root.join("doc/plans");
    fs::create_dir_all(&plans).unwrap();
    fs::write(plans.join("PLAN-foo.md"), "# plan").unwrap();

    let resolved = resolve_plan_path("doc/plans/PLAN-foo.md", &root).unwrap();
    assert_eq!(resolved.invocation, "doc/plans/PLAN-foo.md");
    assert_eq!(resolved.fs_path, plans.join("PLAN-foo.md"));
}

#[test]
fn test_resolve_plan_path_falls_back_to_doc_plans() {
    let temp = TempDir::new().unwrap();
    let root = canonical(&temp);
    let plans = root.join("doc/plans");
    fs::create_dir_all(&plans).unwrap();
    fs::write(plans.join("PLAN-bar.md"), "# plan").unwrap();

    // Bare filename: not present at root, present under doc/plans.
    let resolved = resolve_plan_path("PLAN-bar.md", &root).unwrap();
    assert_eq!(resolved.invocation, "doc/plans/PLAN-bar.md");
}

#[test]
fn test_resolve_plan_path_no_double_prefix() {
    let temp = TempDir::new().unwrap();
    let root = canonical(&temp);
    fs::create_dir_all(root.join("doc/plans")).unwrap();
    // arg starts with doc/plans/ but the file is absent → must NOT try
    // doc/plans/doc/plans/..., it should bail outright.
    let err = resolve_plan_path("doc/plans/missing.md", &root).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("plan file not found"), "got: {msg}");
    assert!(
        !msg.contains("doc/plans/doc/plans"),
        "double-prefixed: {msg}"
    );
}

#[test]
fn test_resolve_plan_path_absolute_outside_repo() {
    let repo = TempDir::new().unwrap();
    let other = TempDir::new().unwrap();
    let root = canonical(&repo);
    let plan = canonical(&other).join("PLAN-ext.md");
    fs::write(&plan, "# plan").unwrap();

    let resolved = resolve_plan_path(plan.to_str().unwrap(), &root).unwrap();
    // Outside the repo → invocation is the absolute path, never cwd-relative.
    assert_eq!(resolved.invocation, plan.to_string_lossy());
    assert!(Path::new(&resolved.invocation).is_absolute());
}

#[test]
fn test_resolve_plan_path_rejects_directory() {
    let temp = TempDir::new().unwrap();
    let root = canonical(&temp);
    // A directory that happens to be named like a plan must not be accepted.
    fs::create_dir_all(root.join("doc/plans/PLAN-dir.md")).unwrap();
    let err = resolve_plan_path("doc/plans/PLAN-dir.md", &root).unwrap_err();
    assert!(
        err.to_string().contains("plan file not found"),
        "got: {err}"
    );
}

#[test]
fn test_codex_report_path_is_sibling() {
    let report = codex_report_path(Path::new("/repo/doc/plans/PLAN-foo.md"));
    assert_eq!(report, PathBuf::from("/repo/doc/plans/codex-PLAN-foo.md"));
}

#[test]
fn test_plan_steps_order_and_count() {
    let report = PathBuf::from("/repo/doc/plans/codex-PLAN-foo.md");
    let steps = plan_steps(2, "doc/plans/PLAN-foo.md", &report);
    // 3 steps per round (delete, parallel pressure, address) + 1 final delete.
    assert_eq!(steps.len(), 2 * 3 + 1);
    assert_eq!(steps[0], Step::DeleteReport(report.clone()));
    assert_eq!(
        steps[1],
        Step::Pressure {
            claude: "/pressure doc/plans/PLAN-foo.md".into(),
            codex: "$pressure doc/plans/PLAN-foo.md".into(),
        }
    );
    assert_eq!(
        steps[2],
        Step::Address("/address doc/plans/PLAN-foo.md".into())
    );
    // Per-round delete must precede that round's parallel pressure step.
    assert_eq!(steps[3], Step::DeleteReport(report.clone()));
    assert_eq!(*steps.last().unwrap(), Step::DeleteReport(report));
}

#[test]
fn test_plan_steps_single_round() {
    let report = PathBuf::from("/r/codex-P.md");
    let steps = plan_steps(1, "P.md", &report);
    assert_eq!(steps.len(), 4);
}

#[test]
fn test_claude_marker_path_is_inside_repo_not_temp_dir() {
    let repo = Path::new("/repo");
    let marker = claude_marker_path(repo);
    // The agent's sandbox mounts /tmp read-only, so a temp-dir marker can never
    // be created and the session would never auto-close.
    assert!(
        !marker.starts_with(std::env::temp_dir()),
        "marker in temp dir: {}",
        marker.display()
    );
    assert!(
        marker.starts_with(repo.join(".loom").join("work").join("pressure")),
        "got: {}",
        marker.display()
    );
    assert_eq!(marker.extension().unwrap(), "done");
}

#[test]
fn test_ensure_marker_dir_creates_parent_and_is_idempotent() {
    let temp = TempDir::new().unwrap();
    let root = canonical(&temp);
    let marker = root
        .join(".loom")
        .join("work")
        .join("pressure")
        .join("claude-1.done");
    assert!(!marker.parent().unwrap().exists());

    ensure_marker_dir(&marker).unwrap();
    assert!(marker.parent().unwrap().is_dir());

    // Idempotent: calling again on an already-existing dir is still Ok.
    ensure_marker_dir(&marker).unwrap();
    assert!(marker.parent().unwrap().is_dir());
}

fn default_models() -> PressureModels {
    PressureModels::resolve(
        crate::cli::types_pressure::PressureModelFlags::default(),
        &crate::fs::work_dir::PressureConfig::default(),
        &crate::user_config::UserConfig::default(),
    )
}

#[test]
fn test_render_dry_run_shows_real_argv() {
    let report = PathBuf::from("/repo/doc/plans/codex-PLAN-foo.md");
    let repo = Path::new("/repo");
    let marker = PathBuf::from("/repo/.loom/work/pressure/claude-1.done");
    let codex_log = PathBuf::from("/tmp/loom-pressure-codex-1.log");
    let out = render_dry_run(
        1,
        "doc/plans/PLAN-foo.md",
        &report,
        repo,
        &marker,
        &codex_log,
        &default_models(),
    );
    assert!(out.contains("Dry run: 1 round"));
    // The preview must show the REAL argv so it matches what spawns.
    assert!(out.contains("--permission-mode auto --model opus --effort xhigh"));
    // The auto-close instruction is injected via --append-system-prompt.
    assert!(out.contains("--append-system-prompt"));
    assert!(out.contains("/pressure doc/plans/PLAN-foo.md"));
    assert!(out.contains(
        "codex exec --sandbox workspace-write -m gpt-5.6-sol -c model_reasoning_effort=xhigh \
         -C /repo $pressure doc/plans/PLAN-foo.md"
    ));
    assert!(out.contains("/address doc/plans/PLAN-foo.md"));
    assert!(out.contains("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1"));
    assert!(out.contains("codex-PLAN-foo.md"));
    // The parallel pressure step is labelled and the codex log path shown.
    assert!(out.contains("[parallel]"));
    assert!(out.contains("loom-pressure-codex-1.log"));
    assert!(out.contains(".loom/work/pressure/claude-1.done"));
    // The resolved model+effort selection is surfaced in the header.
    assert!(out.contains(
        "Models:                  claude=opus/xhigh  codex=gpt-5.6-sol/xhigh  address=opus/high"
    ));
}

#[test]
fn test_render_dry_run_shows_a_non_default_model_selection_per_step() {
    // A non-default, all-distinct selection must reach the right step's argv -
    // this is the whole point of three independently-resolved slots.
    let report = PathBuf::from("/repo/doc/plans/codex-PLAN-foo.md");
    let repo = Path::new("/repo");
    let marker = PathBuf::from("/repo/.loom/work/pressure/claude-1.done");
    let codex_log = PathBuf::from("/tmp/loom-pressure-codex-1.log");
    let models = PressureModels {
        claude: "fable".to_string(),
        claude_effort: "low".to_string(),
        codex: "gpt-6-astra".to_string(),
        codex_effort: "medium".to_string(),
        address: "sonnet".to_string(),
        address_effort: "max".to_string(),
    };
    let out = render_dry_run(
        1,
        "doc/plans/PLAN-foo.md",
        &report,
        repo,
        &marker,
        &codex_log,
        &models,
    );
    // The header shows model AND effort for all three slots.
    assert!(out.contains(
        "Models:                  claude=fable/low  codex=gpt-6-astra/medium  address=sonnet/max"
    ));
    // /pressure step: claude model = fable, codex model = gpt-6-astra, each
    // with its own resolved effort.
    assert!(out.contains("--permission-mode auto --model fable --effort low"));
    assert!(out.contains("-m gpt-6-astra"));
    assert!(out.contains("-c model_reasoning_effort=medium"));
    // /address step's model+effort are independent of /pressure's — sonnet/max,
    // not fable/low.
    assert!(out.contains("--model sonnet --effort max --append-system-prompt"));
}

#[test]
fn test_claude_args_shape() {
    let marker = PathBuf::from("/repo/.loom/work/pressure/claude-1.done");
    let args = claude_args("/pressure doc/plans/PLAN-foo.md", &marker, "opus", "xhigh");
    // Interactive (no -p): keeps subscription billing. Auto permission mode.
    assert!(!args.iter().any(|a| a == "-p" || a == "--print"));
    assert_eq!(args[0], "--permission-mode");
    assert_eq!(args[1], "auto");
    assert_eq!(args[2], "--model");
    assert_eq!(args[3], "opus");
    assert_eq!(args[4], "--effort");
    assert_eq!(args[5], "xhigh");
    // The slash invocation is the final positional argument.
    assert_eq!(args.last().unwrap(), "/pressure doc/plans/PLAN-foo.md");
    // The appended system prompt names the marker so the driver can auto-close.
    let joined = args.join(" ");
    assert!(joined.contains("--append-system-prompt"));
    assert!(joined.contains("/repo/.loom/work/pressure/claude-1.done"));
}

#[test]
fn test_codex_args_shape() {
    let repo = Path::new("/repo");
    let args = codex_args(
        repo,
        "$pressure doc/plans/PLAN-foo.md",
        "gpt-5.6-sol",
        "high",
    );
    assert_eq!(args[0], "exec");
    assert!(args.contains(&"gpt-5.6-sol".to_string()));
    assert!(args.contains(&"model_reasoning_effort=high".to_string()));
    assert_eq!(args.last().unwrap(), "$pressure doc/plans/PLAN-foo.md");
}

#[test]
fn test_classify_code_all_arms() {
    assert_eq!(classify_code(Some(0)), ExitAction::Continue);
    assert_eq!(classify_code(Some(130)), ExitAction::Abort);
    assert_eq!(classify_code(Some(2)), ExitAction::Abort);
    assert_eq!(classify_code(None), ExitAction::Abort); // signal-killed
    assert_eq!(classify_code(Some(1)), ExitAction::Warn);
    assert_eq!(classify_code(Some(42)), ExitAction::Warn);
}
