use super::*;
use crate::models::stage::StageType;
use keys::{spec, ValueKind};

// Every test here parses a `UserConfig` from an in-memory TOML string and
// checks the registry (spec lookup, typing, defaults, origin tracking) —
// never touching disk. File-backed behavior (`set_in`/`unset_in`/`load`)
// lives in `tests/persistence.rs`.

mod persistence;

#[test]
fn each_key_parses_a_valid_value() {
    assert_eq!(
        spec("update.check")
            .unwrap()
            .parse("true")
            .unwrap()
            .as_bool(),
        Some(true)
    );
    assert_eq!(
        spec("update.check_interval_hours")
            .unwrap()
            .parse("6")
            .unwrap()
            .as_integer(),
        Some(6)
    );
    assert_eq!(
        spec("terminal.backend")
            .unwrap()
            .parse("tmux")
            .unwrap()
            .as_str(),
        Some("tmux")
    );
    assert_eq!(
        spec("context.ceiling_tokens")
            .unwrap()
            .parse("123456")
            .unwrap()
            .as_integer(),
        Some(123456)
    );
}

#[test]
fn each_key_rejects_a_type_mismatched_value() {
    for (key, bad) in [
        ("update.check", "maybe"),
        ("update.check_interval_hours", "not-a-number"),
        ("terminal.backend", "ssh"),
        ("context.ceiling_tokens", "-5"),
    ] {
        let err = spec(key).unwrap().parse(bad).unwrap_err().to_string();
        assert!(
            err.contains(key),
            "error for {key} should name the key: {err}"
        );
        assert!(
            err.contains(bad),
            "error for {key} should quote the offending text: {err}"
        );
    }
}

#[test]
fn unknown_key_lists_every_valid_key() {
    let err = spec("no.such.key").unwrap_err().to_string();
    assert!(err.contains("no.such.key"));
    for key in keys::KEYS {
        assert!(
            err.contains(key.name),
            "valid-key list missing {}: {err}",
            key.name
        );
    }
}

#[test]
fn keys_are_typed_as_documented() {
    assert_eq!(spec("update.check").unwrap().kind, ValueKind::Bool);
    assert_eq!(
        spec("update.check_interval_hours").unwrap().kind,
        ValueKind::U32
    );
    assert_eq!(
        spec("terminal.backend").unwrap().kind,
        ValueKind::Enum(&["native", "tmux"])
    );
    assert_eq!(spec("context.ceiling_tokens").unwrap().kind, ValueKind::U32);
}

#[test]
fn defaults_when_the_file_is_absent() {
    let config = UserConfig::default();
    assert!(config.update_check());
    assert_eq!(config.update_check_interval_hours(), 24);
    assert_eq!(config.terminal_backend(), SessionBackendKind::Native);
    assert_eq!(
        config.context_ceiling_tokens(),
        DEFAULT_CONTEXT_CEILING_TOKENS
    );
    assert_eq!(config.terminal_backend_set(), None);
    assert_eq!(config.context_ceiling_tokens_set(), None);
    assert_eq!(config.pressure_claude_model(), "opus");
    assert_eq!(config.pressure_claude_effort(), "xhigh");
    assert_eq!(config.pressure_codex_model(), "gpt-5.6-sol");
    assert_eq!(config.pressure_codex_effort(), "xhigh");
    assert_eq!(config.pressure_address_model(), "opus");
    assert_eq!(config.pressure_address_effort(), "high");

    assert_eq!(config.stage_model(StageType::Standard), "opus");
    assert_eq!(config.stage_reasoning_effort(StageType::Standard), "high");
    assert_eq!(config.stage_model(StageType::Knowledge), "opus");
    assert_eq!(
        config.stage_reasoning_effort(StageType::Knowledge),
        "medium"
    );
    assert_eq!(config.stage_model(StageType::KnowledgeDistill), "sonnet");
    assert_eq!(
        config.stage_reasoning_effort(StageType::KnowledgeDistill),
        "high"
    );
    assert_eq!(config.stage_model(StageType::IntegrationVerify), "opus");
    assert_eq!(
        config.stage_reasoning_effort(StageType::IntegrationVerify),
        "xhigh"
    );
}

#[test]
fn parses_every_key_out_of_a_document() {
    let config = parse_document(
        "[update]\ncheck = false\ncheck_interval_hours = 6\n\n\
         [terminal]\nbackend = \"tmux\"\n\n\
         [context]\nceiling_tokens = 111111\n\n\
         [pressure]\nclaude_model = \"fable\"\nclaude_effort = \"low\"\n\
         codex_model = \"gpt-6-astra\"\ncodex_effort = \"medium\"\n\
         address_model = \"sonnet\"\naddress_effort = \"low\"\n\n\
         [models]\nstandard_model = \"sonnet\"\nstandard_effort = \"low\"\n\
         knowledge_model = \"fable\"\nknowledge_effort = \"medium\"\n\
         knowledge_distill_model = \"opus\"\nknowledge_distill_effort = \"xhigh\"\n\
         integration_verify_model = \"haiku\"\nintegration_verify_effort = \"max\"\n",
    )
    .unwrap();
    assert!(!config.update_check());
    assert_eq!(config.update_check_interval_hours(), 6);
    assert_eq!(config.terminal_backend(), SessionBackendKind::Tmux);
    assert_eq!(config.context_ceiling_tokens(), 111111);
    assert_eq!(config.pressure_claude_model(), "fable");
    assert_eq!(config.pressure_claude_effort(), "low");
    assert_eq!(config.pressure_codex_model(), "gpt-6-astra");
    assert_eq!(config.pressure_codex_effort(), "medium");
    assert_eq!(config.pressure_address_model(), "sonnet");
    assert_eq!(config.pressure_address_effort(), "low");

    assert_eq!(config.stage_model(StageType::Standard), "sonnet");
    assert_eq!(config.stage_reasoning_effort(StageType::Standard), "low");
    assert_eq!(config.stage_model(StageType::Knowledge), "fable");
    assert_eq!(
        config.stage_reasoning_effort(StageType::Knowledge),
        "medium"
    );
    assert_eq!(config.stage_model(StageType::KnowledgeDistill), "opus");
    assert_eq!(
        config.stage_reasoning_effort(StageType::KnowledgeDistill),
        "xhigh"
    );
    assert_eq!(config.stage_model(StageType::IntegrationVerify), "haiku");
    assert_eq!(
        config.stage_reasoning_effort(StageType::IntegrationVerify),
        "max"
    );
}

#[test]
fn a_type_mismatched_field_in_the_document_is_an_error() {
    assert!(parse_document("[update]\ncheck = \"nope\"\n").is_err());
    assert!(parse_document("[context]\nceiling_tokens = \"nope\"\n").is_err());
    assert!(parse_document("[terminal]\nbackend = \"carrier-pigeon\"\n").is_err());
}

#[test]
fn pressure_keys_reject_an_unknown_model_variant() {
    let err = parse_document("[pressure]\nclaude_model = \"gpt-5.6-sol\"\n")
        .unwrap_err()
        .to_string();
    assert!(err.contains("pressure.claude_model"), "{err}");
    assert!(err.contains("gpt-5.6-sol"), "{err}");

    let err = parse_document("[pressure]\ncodex_model = \"opus\"\n")
        .unwrap_err()
        .to_string();
    assert!(err.contains("pressure.codex_model"), "{err}");
    assert!(err.contains("opus"), "{err}");
}

#[test]
fn pressure_and_models_keys_reject_an_unknown_effort_variant() {
    // "max" is a valid Claude effort but not a Codex one - exercises that the
    // two effort value sets are validated independently.
    let err = parse_document("[pressure]\ncodex_effort = \"max\"\n")
        .unwrap_err()
        .to_string();
    assert!(err.contains("pressure.codex_effort"), "{err}");
    assert!(err.contains("max"), "{err}");

    let err = parse_document("[models]\nstandard_effort = \"carrier-pigeon\"\n")
        .unwrap_err()
        .to_string();
    assert!(err.contains("models.standard_effort"), "{err}");
    assert!(err.contains("carrier-pigeon"), "{err}");
}

#[test]
fn models_keys_reject_an_unknown_model_variant() {
    let err = parse_document("[models]\nknowledge_distill_model = \"gpt-5.6-sol\"\n")
        .unwrap_err()
        .to_string();
    assert!(err.contains("models.knowledge_distill_model"), "{err}");
    assert!(err.contains("gpt-5.6-sol"), "{err}");
}

#[test]
fn origin_is_set_only_for_keys_the_document_wrote() {
    let config = parse_document("[update]\ncheck_interval_hours = 6\n").unwrap();

    let (value, origin) = config.value_of(spec("update.check_interval_hours").unwrap());
    assert_eq!(value, "6");
    assert_eq!(origin, Origin::Set);
    assert_eq!(origin.to_string(), "set");

    let (value, origin) = config.value_of(spec("terminal.backend").unwrap());
    assert_eq!(value, "native");
    assert_eq!(origin, Origin::Default);
    assert_eq!(origin.to_string(), "default");
}

#[test]
fn origin_of_pressure_keys_reflects_set_versus_unset() {
    let config =
        parse_document("[pressure]\nclaude_model = \"fable\"\nclaude_effort = \"low\"\n").unwrap();

    let (value, origin) = config.value_of(spec("pressure.claude_model").unwrap());
    assert_eq!(value, "fable");
    assert_eq!(origin, Origin::Set);

    let (value, origin) = config.value_of(spec("pressure.claude_effort").unwrap());
    assert_eq!(value, "low");
    assert_eq!(origin, Origin::Set);

    let (value, origin) = config.value_of(spec("pressure.codex_model").unwrap());
    assert_eq!(value, "gpt-5.6-sol");
    assert_eq!(origin, Origin::Default);

    let (value, origin) = config.value_of(spec("pressure.codex_effort").unwrap());
    assert_eq!(value, "xhigh");
    assert_eq!(origin, Origin::Default);
}

#[test]
fn origin_of_models_keys_reflects_set_versus_unset() {
    let config =
        parse_document("[models]\nstandard_model = \"sonnet\"\nstandard_effort = \"low\"\n")
            .unwrap();

    let (value, origin) = config.value_of(spec("models.standard_model").unwrap());
    assert_eq!(value, "sonnet");
    assert_eq!(origin, Origin::Set);

    let (value, origin) = config.value_of(spec("models.standard_effort").unwrap());
    assert_eq!(value, "low");
    assert_eq!(origin, Origin::Set);

    let (value, origin) = config.value_of(spec("models.knowledge_model").unwrap());
    assert_eq!(value, "opus");
    assert_eq!(origin, Origin::Default);

    let (value, origin) = config.value_of(spec("models.knowledge_effort").unwrap());
    assert_eq!(value, "medium");
    assert_eq!(origin, Origin::Default);
}

#[test]
fn to_toml_string_renders_every_key_resolved() {
    let config = parse_document("[context]\nceiling_tokens = 55555\n").unwrap();
    let rendered = config.to_toml_string();

    // Section order: context, models, pressure, terminal, update.
    let context_at = rendered.find("[context]").unwrap();
    let models_at = rendered.find("[models]").unwrap();
    let pressure_at = rendered.find("[pressure]").unwrap();
    let terminal_at = rendered.find("[terminal]").unwrap();
    let update_at = rendered.find("[update]").unwrap();
    assert!(
        context_at < models_at
            && models_at < pressure_at
            && pressure_at < terminal_at
            && terminal_at < update_at,
        "{rendered}"
    );

    // Every key's resolved `field = ` line must appear somewhere in the
    // rendered blob - a loop over the real registry means a key added to
    // `keys::KEYS` without a matching render can never go unnoticed.
    for key in keys::KEYS {
        let (value, _) = config.value_of(key);
        let expected = match key.kind {
            ValueKind::Bool | ValueKind::U32 => format!("{} = {value}", key.field),
            ValueKind::Enum(_) => format!("{} = \"{value}\"", key.field),
        };
        assert!(
            rendered.contains(&expected),
            "rendered config missing {expected:?} for {}: {rendered}",
            key.name
        );
    }
}

#[test]
fn value_of_has_an_arm_for_every_registered_key() {
    // `value_of`'s match on `spec.name` has an `unreachable!()` fallback arm
    // that nothing checks at compile time - a key added to `keys::KEYS`
    // without a matching arm would panic `loom config --list` at runtime.
    // Looping over the real registry here means that panic happens in this
    // test instead of in the field.
    let config = UserConfig::default();
    for key in keys::KEYS {
        let (value, origin) = config.value_of(key);
        assert_eq!(origin, Origin::Default, "{}", key.name);
        match key.name {
            "update.check" => assert_eq!(value, "true"),
            "update.check_interval_hours" => assert_eq!(value, "24"),
            "terminal.backend" => assert_eq!(value, "native"),
            "context.ceiling_tokens" => {
                assert_eq!(value, DEFAULT_CONTEXT_CEILING_TOKENS.to_string())
            }
            "pressure.claude_model" => assert_eq!(value, "opus"),
            "pressure.claude_effort" => assert_eq!(value, "xhigh"),
            "pressure.codex_model" => assert_eq!(value, "gpt-5.6-sol"),
            "pressure.codex_effort" => assert_eq!(value, "xhigh"),
            "pressure.address_model" => assert_eq!(value, "opus"),
            "pressure.address_effort" => assert_eq!(value, "high"),
            "models.standard_model" => assert_eq!(value, "opus"),
            "models.standard_effort" => assert_eq!(value, "high"),
            "models.knowledge_model" => assert_eq!(value, "opus"),
            "models.knowledge_effort" => assert_eq!(value, "medium"),
            "models.knowledge_distill_model" => assert_eq!(value, "sonnet"),
            "models.knowledge_distill_effort" => assert_eq!(value, "high"),
            "models.integration_verify_model" => assert_eq!(value, "opus"),
            "models.integration_verify_effort" => assert_eq!(value, "xhigh"),
            other => panic!("no expected default wired up for key {other}"),
        }
    }
}
