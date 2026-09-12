//! Tests for `native/launch.rs`, split out to keep that module under the
//! 400-line ceiling (CLAUDE.md Rule 17), matching `native/tests.rs`.

use super::*;
use crate::models::stage::StageType;
use crate::orchestrator::terminal::native::build_claude_command;
use crate::orchestrator::terminal::native::capsule::capsule_from;
use crate::remote_control::RemoteControlInvocation;
use tempfile::TempDir;

fn stage_named(id: &str, name: &str) -> Stage {
    Stage {
        id: id.to_string(),
        name: name.to_string(),
        ..Stage::default()
    }
}

/// A fresh `.loom/work/` with `[context] prompt_cache_split = <value>` set.
fn work_dir_with_context_flag(value: bool) -> (TempDir, PathBuf) {
    let temp = TempDir::new().unwrap();
    let work_dir = temp.path().join(".loom").join("work");
    std::fs::create_dir_all(&work_dir).unwrap();
    crate::fs::work_dir::update_config(&work_dir, |doc| {
        let table = doc.entry("context").or_insert(toml_edit::table());
        if let Some(table) = table.as_table_mut() {
            table["prompt_cache_split"] = toml_edit::value(value);
        }
        Ok(())
    })
    .unwrap();
    (temp, work_dir)
}

/// A fresh `.loom/work/` with no `config.toml` at all — the `[context]` key is not
/// merely false, it is absent.
fn work_dir_without_config() -> (TempDir, PathBuf) {
    let temp = TempDir::new().unwrap();
    let work_dir = temp.path().join(".loom").join("work");
    std::fs::create_dir_all(&work_dir).unwrap();
    (temp, work_dir)
}

/// The `claude` command line a spawn builds from `prefix_file`, with
/// `--append-system-prompt-file` advertised as supported so the split is the
/// only thing that can change the bytes.
fn command_line_for(prefix_file: Option<String>) -> String {
    let capsule = capsule_from(false, false, false, true, None, prefix_file);
    build_claude_command(
        "/usr/bin/claude",
        "opus",
        "xhigh",
        "auto",
        &capsule,
        &RemoteControlInvocation::Disabled,
        "'prompt'",
    )
}

/// The bytes a spawn emits when the capsule contributes nothing — what the
/// disabled default must keep emitting, unchanged.
const NO_CAPSULE_COMMAND_LINE: &str =
    "/usr/bin/claude --model opus --effort xhigh --permission-mode auto 'prompt'";

#[test]
fn prompt_cache_split_enabled_defaults_to_false_when_config_missing() {
    let (_temp, work_dir) = work_dir_without_config();
    assert!(!prompt_cache_split_enabled(&work_dir));
}

#[test]
fn prompt_cache_split_enabled_reads_explicit_true() {
    let (_temp, work_dir) = work_dir_with_context_flag(true);
    assert!(prompt_cache_split_enabled(&work_dir));
}

#[test]
fn prompt_cache_split_enabled_reads_explicit_false() {
    let (_temp, work_dir) = work_dir_with_context_flag(false);
    assert!(!prompt_cache_split_enabled(&work_dir));
}

#[test]
fn a_disabled_split_writes_no_prefix_file_and_changes_no_bytes() {
    // Absent key and explicit `false` are the same default, and the default is
    // what ships: nothing on disk, and a command line byte-identical to the
    // one every spawn emits today.
    for (_temp, work_dir) in [work_dir_without_config(), work_dir_with_context_flag(false)] {
        let stage = stage_named("my-stage", "My Stage");
        let resolved = resolve_prompt_cache_split_prefix_file(&work_dir, &stage);

        assert_eq!(resolved, None);
        assert!(
            !work_dir.join("signals").join("prefix").exists(),
            "the disabled default must not touch the filesystem"
        );
        assert_eq!(command_line_for(resolved), NO_CAPSULE_COMMAND_LINE);
    }
}

#[test]
fn an_enabled_split_writes_the_stable_prefix_verbatim() {
    let (_temp, work_dir) = work_dir_with_context_flag(true);
    let stage = stage_named("my-stage", "My Stage");

    let resolved = resolve_prompt_cache_split_prefix_file(&work_dir, &stage)
        .expect("an enabled split must resolve a prefix file");
    assert_eq!(
        PathBuf::from(&resolved),
        work_dir.join("signals").join("prefix").join("my-stage.md")
    );

    let written = std::fs::read_to_string(&resolved).unwrap();
    assert_eq!(
        written,
        crate::orchestrator::signals::stable_prefix_for(stage.stage_type),
        "the prefix file must be the stable prefix byte for byte, never an approximation of it"
    );
    // This stage retired the restated CLAUDE.md doctrine (the Rule 5
    // subagent-restrictions fence included) from every stable prefix in favor
    // of a pointer to the binding rules file, which is already in context in
    // the same session. The prefix file must carry that pointer, not the
    // doctrine it replaced.
    assert!(
        written.contains("Binding rules: ~/.claude/CLAUDE.md. This signal overrides none of them."),
        "the prefix file must carry the doctrine pointer"
    );
}

#[test]
fn an_enabled_split_puts_the_escaped_prefix_path_on_the_command_line() {
    let (_temp, work_dir) = work_dir_with_context_flag(true);
    let stage = stage_named("my-stage", "My Stage");

    let resolved = resolve_prompt_cache_split_prefix_file(&work_dir, &stage)
        .expect("an enabled split must resolve a prefix file");
    let command_line = command_line_for(Some(resolved.clone()));

    let escaped = escape(Cow::Borrowed(resolved.as_str()));
    assert!(
        command_line.contains(&format!("--append-system-prompt-file {escaped}")),
        "command line: {command_line}"
    );
    assert_ne!(command_line, NO_CAPSULE_COMMAND_LINE);
}

#[test]
fn remote_control_session_name_stage_uses_bare_name() {
    let stage = stage_named("my-stage", "My Stage");
    assert_eq!(
        remote_control_session_name(SessionType::Stage, &stage),
        "My Stage"
    );
}

#[test]
fn remote_control_session_name_merge_is_prefixed() {
    let stage = stage_named("my-stage", "My Stage");
    assert_eq!(
        remote_control_session_name(SessionType::Merge, &stage),
        "Merge: My Stage"
    );
}

#[test]
fn remote_control_session_name_base_conflict_is_prefixed() {
    let stage = stage_named("my-stage", "My Stage");
    assert_eq!(
        remote_control_session_name(SessionType::BaseConflict, &stage),
        "Base conflict: My Stage"
    );
}

#[test]
fn remote_control_session_name_knowledge_is_prefixed() {
    let stage = stage_named("my-stage", "My Stage");
    assert_eq!(
        remote_control_session_name(SessionType::Knowledge, &stage),
        "Knowledge: My Stage"
    );
}

#[test]
fn remote_control_session_name_adjudication_is_prefixed() {
    let stage = stage_named("my-stage", "My Stage");
    assert_eq!(
        remote_control_session_name(SessionType::Adjudication, &stage),
        "Adjudication: My Stage"
    );
}

/// The adjudicator must not run on the model the disputing plan chose — that
/// would let a plan pick the judge of its own criteria — and it must honour the
/// operator's `[adjudication] model` override.
#[test]
fn adjudication_runs_on_opus_unless_the_config_says_otherwise() {
    let (_temp, work_dir) = work_dir_without_config();
    let mut stage = stage_named("my-stage", "My Stage");
    stage.model = Some("haiku".to_string());

    let (model, effort) = model_and_effort(SessionType::Adjudication, &stage, &work_dir);
    assert_eq!(model, "opus");
    assert_eq!(effort, "high");

    std::fs::write(
        work_dir.join("config.toml"),
        "[adjudication]\nmodel = \"sonnet\"\n",
    )
    .unwrap();
    let (model, _) = model_and_effort(SessionType::Adjudication, &stage, &work_dir);
    assert_eq!(model, "sonnet", "the config override must reach the spawn");
}

#[test]
fn stage_sessions_still_run_on_the_stage_model() {
    let (_temp, work_dir) = work_dir_without_config();
    let mut stage = stage_named("my-stage", "My Stage");
    stage.model = Some("haiku".to_string());

    let (model, _) = model_and_effort(SessionType::Stage, &stage, &work_dir);
    assert_eq!(model, "haiku");
    let (merge_model, merge_effort) = model_and_effort(SessionType::Merge, &stage, &work_dir);
    assert_eq!(
        (merge_model.as_str(), merge_effort.as_str()),
        ("opus", "high")
    );
}

/// A stage with no plan-level model/effort resolves through the project
/// config tier — the same tier the dashboard's summary reads — not just the
/// plan-field/built-in pair `Stage::effective_model()` stops at.
#[test]
fn stage_sessions_resolve_the_project_config_tier_when_the_plan_sets_nothing() {
    let (_temp, work_dir) = work_dir_without_config();
    std::fs::write(
        work_dir.join("config.toml"),
        "[models]\nstandard_model = \"sonnet\"\nstandard_effort = \"low\"\n",
    )
    .unwrap();
    let stage = stage_named("my-stage", "My Stage");

    let (model, effort) = model_and_effort(SessionType::Stage, &stage, &work_dir);
    assert_eq!(model, "sonnet");
    assert_eq!(effort, "low");
}

/// An explicit plan field still wins over the project config tier — a
/// project-wide default must never override a stage's own choice.
#[test]
fn a_plan_field_wins_over_the_project_config_tier() {
    let (_temp, work_dir) = work_dir_without_config();
    std::fs::write(
        work_dir.join("config.toml"),
        "[models]\nstandard_model = \"sonnet\"\nstandard_effort = \"low\"\n",
    )
    .unwrap();
    let mut stage = stage_named("my-stage", "My Stage");
    stage.model = Some("haiku".to_string());
    stage.reasoning_effort = Some("xhigh".to_string());

    let (model, effort) = model_and_effort(SessionType::Stage, &stage, &work_dir);
    assert_eq!(model, "haiku");
    assert_eq!(effort, "xhigh");
}

/// A project section that sets only `standard_model` must not shadow
/// `standard_effort` for the whole section — the fallback to the built-in is
/// key-level.
#[test]
fn a_project_model_alone_leaves_effort_at_the_built_in() {
    let (_temp, work_dir) = work_dir_without_config();
    std::fs::write(
        work_dir.join("config.toml"),
        "[models]\nstandard_model = \"sonnet\"\n",
    )
    .unwrap();
    let stage = stage_named("my-stage", "My Stage");

    let (model, effort) = model_and_effort(SessionType::Stage, &stage, &work_dir);
    assert_eq!(model, "sonnet");
    assert_eq!(effort, "high");
}

/// Pin the stage-type built-ins a bare spawn launches on, so a later edit to
/// the defaults table cannot silently change what a plan-field-free stage
/// runs on.
#[test]
fn integration_verify_and_knowledge_stages_launch_on_their_built_ins() {
    let (_temp, work_dir) = work_dir_without_config();

    let mut verify_stage = stage_named("verify-stage", "Verify");
    verify_stage.stage_type = StageType::IntegrationVerify;
    let (model, effort) = model_and_effort(SessionType::Stage, &verify_stage, &work_dir);
    assert_eq!((model.as_str(), effort.as_str()), ("opus", "xhigh"));

    let mut knowledge_stage = stage_named("knowledge-stage", "Knowledge");
    knowledge_stage.stage_type = StageType::Knowledge;
    let (model, effort) = model_and_effort(SessionType::Knowledge, &knowledge_stage, &work_dir);
    assert_eq!((model.as_str(), effort.as_str()), ("opus", "medium"));
}

#[test]
fn remote_control_session_name_falls_back_to_stage_id_when_name_empty() {
    let stage = stage_named("my-stage", "   ");
    assert_eq!(
        remote_control_session_name(SessionType::Stage, &stage),
        "my-stage"
    );
}

#[test]
fn remote_control_session_name_handles_fully_empty_stage() {
    let stage = Stage::default();
    assert_eq!(
        remote_control_session_name(SessionType::Merge, &stage),
        "Merge: "
    );
}
