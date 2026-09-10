//! Unit tests for the wrapper script's `LOOM_SESSION_TYPE` export.
//!
//! Split out of `tests.rs` to keep it under the 400-line ceiling (CLAUDE.md
//! Rule 17), matching how `tests_capsule.rs` and `tests_launch.rs` are split
//! out of the same directory.

use super::wrapper::create_session_wrapper_script;
use super::*;
use tempfile::TempDir;

fn wrapper_script_for(kind: SessionType) -> String {
    let work_dir = TempDir::new().unwrap();
    let path = create_wrapper_script(
        work_dir.path(),
        "loom-test-session",
        "feature",
        "session1",
        "claude 'prompt'",
        None,
        kind,
        100_000,
    )
    .unwrap();
    std::fs::read_to_string(path).unwrap()
}

fn wrapper_script_for_with_rustc_wrapper(kind: SessionType, rustc_wrapper_allowed: bool) -> String {
    let work_dir = TempDir::new().unwrap();
    let path = create_session_wrapper_script(
        work_dir.path(),
        "loom-test-session",
        "feature",
        "session1",
        "claude 'prompt'",
        None,
        kind,
        100_000,
        rustc_wrapper_allowed,
    )
    .unwrap();
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn wrapper_script_exports_session_type() {
    let script = wrapper_script_for(SessionType::Adjudication);
    assert!(
        script.contains("LOOM_SESSION_TYPE=adjudication"),
        "{script}"
    );
    let script = wrapper_script_for(SessionType::Stage);
    assert!(script.contains("LOOM_SESSION_TYPE=stage"), "{script}");
}

#[test]
fn wrapper_script_exports_ripgrep_config_path_when_published() {
    let work_dir = TempDir::new().unwrap();
    std::fs::write(work_dir.path().join("ripgreprc"), "").unwrap();
    let path = create_wrapper_script(
        work_dir.path(),
        "loom-test-session",
        "feature",
        "session1",
        "claude 'prompt'",
        None,
        SessionType::Stage,
        100_000,
    )
    .unwrap();
    let script = std::fs::read_to_string(path).unwrap();

    let line = script
        .lines()
        .find(|line| line.contains("RIPGREP_CONFIG_PATH="))
        .unwrap_or_else(|| panic!("no RIPGREP_CONFIG_PATH line in script: {script}"));
    assert!(line.contains("/ripgreprc"), "{line}");
    assert!(script.contains("LOOM_WORK_DIR="), "{script}");
}

#[test]
fn wrapper_script_omits_ripgrep_config_path_when_unpublished() {
    let script = wrapper_script_for(SessionType::Stage);
    assert!(!script.contains("RIPGREP_CONFIG_PATH="), "{script}");
    assert!(script.contains("LOOM_WORK_DIR="), "{script}");
}

/// Pins `LOOM_SCCACHE` for a test's duration and restores it on drop. Process-
/// global, so both tests below run `#[serial]` (same convention as
/// `commands::stage::tests::state::EnvVarGuard`).
struct EnvVarGuard {
    key: &'static str,
    original: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let original = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, original }
    }

    /// Clears `key` for the guard's lifetime, restoring whatever value (or
    /// absence) it had on drop. Used to strip an ambient `RUSTC_WRAPPER`,
    /// `SCCACHE_DIR`, or `SCCACHE_CACHE_SIZE` a developer's own shell may
    /// have set, so the sccache tests below observe only what `LOOM_SCCACHE`
    /// controls.
    fn unset(key: &'static str) -> Self {
        let original = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, original }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

/// Writes an executable fake `sccache` under `dir`, so the resolver in
/// `build_cache::find_sccache_path` (via `LOOM_SCCACHE`) has a real file to
/// find. Its body never runs: the gate that decides `RUSTC_WRAPPER` no
/// longer probes the candidate, it only resolves a path.
fn fake_sccache_executable(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("sccache");
    std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
    }
    path
}

/// Pins `LOOM_SCCACHE` at `fake` and strips the two env vars the
/// forwarding logic itself reads, so a test observes only what the pin
/// controls. Returns the guards; dropping them restores the prior state.
fn pin_sccache(fake: &std::path::Path) -> (EnvVarGuard, EnvVarGuard, EnvVarGuard) {
    (
        EnvVarGuard::unset("SCCACHE_DIR"),
        EnvVarGuard::unset("SCCACHE_CACHE_SIZE"),
        EnvVarGuard::set("LOOM_SCCACHE", fake.to_str().unwrap()),
    )
}

#[test]
#[serial_test::serial]
fn wrapper_script_exports_rustc_wrapper_when_allowed_and_resolvable() {
    let sccache_dir = TempDir::new().unwrap();
    let fake = fake_sccache_executable(sccache_dir.path());
    let _rustc_wrapper = EnvVarGuard::unset("RUSTC_WRAPPER");
    let _guards = pin_sccache(&fake);

    let script = wrapper_script_for_with_rustc_wrapper(SessionType::Stage, true);
    assert_eq!(script.matches("RUSTC_WRAPPER=").count(), 1, "{script}");
    assert!(
        script.contains(&format!("RUSTC_WRAPPER={}", fake.display())),
        "{script}"
    );
}

#[test]
#[serial_test::serial]
fn wrapper_script_omits_rustc_wrapper_when_not_allowed_even_if_resolvable() {
    let sccache_dir = TempDir::new().unwrap();
    let fake = fake_sccache_executable(sccache_dir.path());
    let _rustc_wrapper = EnvVarGuard::unset("RUSTC_WRAPPER");
    let _guards = pin_sccache(&fake);

    let script = wrapper_script_for_with_rustc_wrapper(SessionType::Stage, false);
    assert!(!script.contains("RUSTC_WRAPPER"), "{script}");
}

#[test]
#[serial_test::serial]
fn wrapper_script_omits_an_operators_rustc_wrapper_when_not_allowed() {
    // No resolvable sccache at all — the only candidate is the operator's own
    // `RUSTC_WRAPPER`, forwarded from the (unsandboxed) daemon's environment.
    // This is the leak that dropping RUSTC_WRAPPER from ENV_ALLOWLIST closes:
    // it must not surface even though `sccache_env`'s fallback would read it.
    let _sccache_dir = EnvVarGuard::unset("SCCACHE_DIR");
    let _sccache_cache_size = EnvVarGuard::unset("SCCACHE_CACHE_SIZE");
    let _disabled = EnvVarGuard::set("LOOM_SCCACHE", "0");
    let _operator_value = EnvVarGuard::set("RUSTC_WRAPPER", "/usr/bin/operator-sccache");

    let script = wrapper_script_for_with_rustc_wrapper(SessionType::Stage, false);
    assert!(!script.contains("RUSTC_WRAPPER"), "{script}");
}

#[test]
#[serial_test::serial]
fn wrapper_script_escapes_a_sccache_path_containing_a_space() {
    let sccache_dir = TempDir::new().unwrap();
    let nested = sccache_dir.path().join("has space");
    std::fs::create_dir(&nested).unwrap();
    let fake = fake_sccache_executable(&nested);
    let _rustc_wrapper = EnvVarGuard::unset("RUSTC_WRAPPER");
    let _guards = pin_sccache(&fake);

    let script = wrapper_script_for_with_rustc_wrapper(SessionType::Stage, true);
    let expected = escape(format!("RUSTC_WRAPPER={}", fake.display()).into());
    assert!(script.contains(&expected.to_string()), "{script}");
}

#[test]
#[serial_test::serial]
fn wrapper_script_omits_rustc_wrapper_when_nothing_resolves() {
    let _rustc_wrapper = EnvVarGuard::unset("RUSTC_WRAPPER");
    let _sccache_dir = EnvVarGuard::unset("SCCACHE_DIR");
    let _sccache_cache_size = EnvVarGuard::unset("SCCACHE_CACHE_SIZE");
    let _pin = EnvVarGuard::set("LOOM_SCCACHE", "0");

    let script = wrapper_script_for_with_rustc_wrapper(SessionType::Stage, true);
    // No resolved path and no operator override: the script has no
    // `RUSTC_WRAPPER` anywhere, matching the no-sccache shape.
    assert!(!script.contains("RUSTC_WRAPPER"), "{script}");
}
