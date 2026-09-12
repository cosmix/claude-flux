//! Tests for the `/api/config` projection and its write path.
//!
//! None of these open a socket: the transport gates (`Origin`, CSRF, the body
//! cap) are exercised over loopback in `web::tests::config_api`, while
//! everything here calls [`super::payload`] and [`super::update`] directly.
//!
//! Every test runs against a scratch tree AND a redirected user config, so the
//! suite can never read or write the operator's real `~/.loom/config.toml` —
//! see `user_config::redirect` for why that redirect is the only way there from
//! the lib test binary.

use std::path::PathBuf;

use tempfile::TempDir;

use crate::fs::work_dir::{insert_key, update_config, WorkDir};
use crate::models::constants::DEFAULT_CONTEXT_CEILING_TOKENS;
use crate::user_config::keys::{spec, KEYS};
use crate::user_config::{redirect_user_config, UserConfigRedirect};

use super::wire::{ConfigKind, ConfigPayload, Source};
use super::{entries, payload, workspace};

mod resolution;
mod updates;

const FIXTURE: &str = include_str!("../../../../../../web/src/api/fixtures/config.json");

/// A scratch tree plus a user-config redirect, both inside one `TempDir`.
struct Scratch {
    /// Held for its `Drop`: the tree disappears with it.
    _temp: TempDir,
    /// The served base — the directory holding `.loom/work`.
    base: PathBuf,
    /// Held for its `Drop`: the redirect lasts exactly as long as the tree.
    _redirect: UserConfigRedirect,
}

impl Scratch {
    /// The `.loom/work` directory, for the resolution functions this suite
    /// pins the projection against.
    fn work(&self) -> PathBuf {
        self.base.join(".loom").join("work")
    }

    /// Set a project-scope key the way an operator's editor would.
    fn write_project(&self, section: &str, field: &str, value: toml_edit::Value) {
        update_config(&self.work(), |doc| insert_key(doc, section, field, value))
            .expect("write the workspace config");
    }
}

fn scratch_tree() -> (TempDir, PathBuf, UserConfigRedirect) {
    let temp = tempfile::tempdir().expect("create scratch tree");
    let base = temp.path().to_path_buf();
    // The recorded incident this guards against is a scratch root that came
    // back empty and sent writes at the operator's real home.
    assert!(
        base.is_absolute() && base.components().count() > 2,
        "scratch tree resolved to {}, which is not a temporary directory",
        base.display()
    );
    let redirect = redirect_user_config(base.join("user-config.toml"));
    (temp, base, redirect)
}

/// A scratch tree with an initialized workspace.
fn scratch() -> Scratch {
    let (temp, base, redirect) = scratch_tree();
    WorkDir::new(&base)
        .expect("build work dir")
        .initialize()
        .expect("initialize work dir");
    Scratch {
        _temp: temp,
        base,
        _redirect: redirect,
    }
}

/// A scratch tree with no workspace at all — the project scope unavailable.
fn scratch_without_workspace() -> Scratch {
    let (temp, base, redirect) = scratch_tree();
    Scratch {
        _temp: temp,
        base,
        _redirect: redirect,
    }
}

fn parse(base: &std::path::Path) -> ConfigPayload {
    serde_json::from_str(&payload(base).expect("build the config payload"))
        .expect("payload is JSON")
}

fn entry<'a>(payload: &'a ConfigPayload, name: &str) -> &'a super::wire::ConfigEntry {
    payload
        .entries
        .iter()
        .find(|entry| entry.name == name)
        .unwrap_or_else(|| panic!("payload carries {name}"))
}

#[test]
fn entries_are_the_whole_registry_in_registry_order() {
    let scratch = scratch();
    let payload = parse(&scratch.base);
    let names: Vec<&str> = payload.entries.iter().map(|e| e.name.as_str()).collect();
    let expected: Vec<&str> = KEYS.iter().map(|key| key.name).collect();
    assert_eq!(names, expected);
    for (entry, key) in payload.entries.iter().zip(KEYS) {
        assert_eq!(entry.help, key.help);
        assert_eq!(entry.kind, ConfigKind::from(&key.kind));
    }
}

#[test]
fn enum_kinds_carry_their_variants_verbatim() {
    let scratch = scratch();
    let payload = parse(&scratch.base);
    assert_eq!(
        entry(&payload, "terminal.backend").kind,
        ConfigKind::Enum {
            variants: vec!["native".to_owned(), "tmux".to_owned()],
        }
    );
    let ConfigKind::Enum { variants } = &entry(&payload, "pressure.claude_model").kind else {
        panic!("pressure.claude_model is an enum key");
    };
    assert_eq!(variants, crate::claude::CLAUDE_MODELS);
}

#[test]
fn only_the_workspace_backed_keys_carry_a_project_scope() {
    let scratch = scratch();
    let payload = parse(&scratch.base);
    for entry in &payload.entries {
        let key = KEYS.iter().find(|key| key.name == entry.name).unwrap();
        let project = matches!(key.section, "pressure" | "models")
            || matches!(
                entry.name.as_str(),
                "terminal.backend" | "context.ceiling_tokens"
            );
        let expected = if project {
            vec!["user".to_owned(), "project".to_owned()]
        } else {
            vec!["user".to_owned()]
        };
        assert_eq!(entry.scopes, expected, "{}", entry.name);
        assert_eq!(entry.project.is_some(), project, "{}", entry.name);
    }
}

#[test]
fn without_a_workspace_every_project_scope_is_null() {
    let scratch = scratch_without_workspace();
    let payload = parse(&scratch.base);
    assert!(!payload.project.available);
    assert_eq!(payload.project.path, super::PROJECT_CONFIG_PATH);
    for entry in &payload.entries {
        assert!(entry.project.is_none(), "{}", entry.name);
        assert_ne!(entry.effective.source, Source::Project, "{}", entry.name);
    }
}

#[test]
fn a_default_tree_resolves_every_key_to_its_built_in() {
    let scratch = scratch();
    let payload = parse(&scratch.base);
    for entry in &payload.entries {
        assert!(!entry.user.set, "{}", entry.name);
        assert_eq!(entry.effective.source, Source::Default, "{}", entry.name);
        assert_eq!(entry.effective.value, entry.user.value, "{}", entry.name);
    }
    assert_eq!(
        entry(&payload, "context.ceiling_tokens").effective.value,
        DEFAULT_CONTEXT_CEILING_TOKENS.to_string()
    );
}

/// `web/src/api/fixtures/config.json` is what the page's zod schema is written
/// against, so it is pinned to a payload this server actually emits — one
/// covering all three `effective.source` values. Adding a registry key, or a
/// variant to one of the enum keys' model lists, breaks this test until the
/// fixture is regenerated, which is the point: the page would otherwise be
/// parsing a shape the server no longer sends.
///
/// To regenerate, print the payload this test builds and write it to the
/// fixture with `serde_json::to_string_pretty` plus a trailing newline — the
/// scenario is the four lines above, and nothing but `csrf_token` varies
/// between runs.
#[test]
fn the_config_fixture_matches_a_real_payload() {
    let scratch = scratch();
    crate::user_config::set(spec("update.check").unwrap(), toml_edit::Value::from(false))
        .expect("set the user update check");
    scratch.write_project(
        "context",
        "ceiling_tokens",
        toml_edit::Value::from(900_000_i64),
    );

    let fixture: ConfigPayload = serde_json::from_str(FIXTURE).expect("fixture is JSON");
    let payload = parse(&scratch.base);
    assert_eq!(fixture.project, payload.project);
    assert_eq!(fixture.entries, payload.entries);
    // The token is minted per process, so only its shape can be pinned.
    assert_eq!(fixture.csrf_token.len(), 64);
    assert!(fixture.csrf_token.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn the_csrf_token_is_a_stable_64_character_hex_string() {
    let token = super::csrf::token();
    assert_eq!(token.len(), 64);
    assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(token, super::csrf::token(), "one token per process");
    assert!(super::csrf::verify(Some(token)));
    assert!(!super::csrf::verify(None));
    assert!(!super::csrf::verify(Some("")));
    assert!(!super::csrf::verify(Some(&"0".repeat(64))));
    assert!(!super::csrf::verify(Some(&token[..63])));
}

/// `default` has to survive both scopes setting the key, which is the whole
/// reason it is on the wire: `user.value` is the RESOLVED user value, so it
/// stops reporting the built-in the moment the file sets one, and the dialog
/// would have nothing left to offer as "reset".
#[test]
fn the_built_in_default_stays_reported_after_both_scopes_set_a_key() {
    let scratch = scratch();
    let built_in = DEFAULT_CONTEXT_CEILING_TOKENS.to_string();
    assert_eq!(
        entry(&parse(&scratch.base), "context.ceiling_tokens").default,
        built_in
    );

    crate::user_config::set(
        spec("context.ceiling_tokens").unwrap(),
        toml_edit::Value::from(640_000_i64),
    )
    .expect("set the user ceiling");
    scratch.write_project(
        "context",
        "ceiling_tokens",
        toml_edit::Value::from(900_000_i64),
    );

    let payload = parse(&scratch.base);
    let ceiling = entry(&payload, "context.ceiling_tokens");
    assert_eq!(ceiling.default, built_in);
    assert_eq!(ceiling.user.value, "640000");
    assert_eq!(ceiling.project.as_ref().unwrap().value, "900000");
    assert_eq!(ceiling.effective.value, "900000");

    // And for a key with no project tier at all.
    crate::user_config::set(spec("update.check").unwrap(), toml_edit::Value::from(false))
        .expect("set the user update check");
    let payload = parse(&scratch.base);
    let check = entry(&payload, "update.check");
    assert_eq!(check.default, "true");
    assert_eq!(check.user.value, "false");
}

/// Every key's `default` must be what an all-`None` config resolves to - the
/// same numbers the getters own, not a second table on the wire.
#[test]
fn every_entry_reports_the_registrys_own_default() {
    let scratch = scratch();
    let payload = parse(&scratch.base);
    for (entry, key) in payload.entries.iter().zip(KEYS) {
        let (expected, origin) = crate::user_config::UserConfig::default().value_of(key);
        assert_eq!(origin, crate::user_config::Origin::Default, "{}", key.name);
        assert_eq!(entry.default, expected, "{}", key.name);
    }
}

/// The project scope covers the two section-level keys plus every
/// `[pressure]`/`[models]` key (sixteen total), and each carries the tier its
/// section implies. The key-level set is derived from [`KEYS`] rather than
/// typed out fourteen times, so this test keeps testing the right thing when
/// a fifteenth key-level key is added.
#[test]
fn project_scoped_covers_the_section_level_pair_and_every_key_level_key() {
    let key_level: Vec<&str> = KEYS
        .iter()
        .filter(|key| matches!(key.section, "pressure" | "models"))
        .map(|key| key.name)
        .collect();
    let expected: Vec<&str> = ["terminal.backend", "context.ceiling_tokens"]
        .into_iter()
        .chain(key_level.iter().copied())
        .collect();

    let named: Vec<&str> = KEYS
        .iter()
        .filter(|key| entries::project_scoped(key))
        .map(|key| key.name)
        .collect();
    assert_eq!(named, expected);

    for name in ["terminal.backend", "context.ceiling_tokens"] {
        assert_eq!(
            workspace::tier_of(spec(name).unwrap()),
            Some(workspace::ProjectTier::Section),
            "{name}"
        );
    }
    for name in key_level {
        assert_eq!(
            workspace::tier_of(spec(name).unwrap()),
            Some(workspace::ProjectTier::Key),
            "{name}"
        );
    }
}
