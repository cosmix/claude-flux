//! sccache discovery for sharing compiled dependencies across stage worktrees.
//!
//! Every stage worktree has its own `target/`, so absent this, each stage
//! recompiles every dependency from scratch (minutes per stage) before it can
//! run a single test. A shared `CARGO_TARGET_DIR` is not an option — parallel
//! stages would overwrite each other's `debug/loom` binary, which acceptance
//! criteria invoke by relative path. sccache instead caches individual rustc
//! invocations by input hash, so dependency crates compile once per machine
//! while each worktree keeps its own `target/`.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Operator override for [`find_sccache_path`]. `"0"` disables sccache
/// outright (the escape hatch for a misbehaving install); any other value
/// pins the exact binary to use instead of searching.
const LOOM_SCCACHE_ENV: &str = "LOOM_SCCACHE";

/// Find the absolute path to the `sccache` binary, honouring `LOOM_SCCACHE`.
///
/// Mirrors [`crate::codex::find_codex_path`]: `which::which` first (uses the
/// current PATH), then a fixed list of common install locations. Spawned
/// terminals/children may not inherit the parent's PATH, so this resolves
/// eagerly rather than deferring to the child's own lookup.
///
/// `LOOM_SCCACHE=0` disables the resolver unconditionally, even when a real
/// `sccache` sits on PATH. `LOOM_SCCACHE=<path>` pins the exact binary
/// instead of searching; a pin that does not exist on disk is treated as
/// absent (logged via `tracing::warn!`), never silently substituted with a
/// PATH search.
///
/// Only the PATH/candidate-list search is memoized (`OnceLock`, for the
/// process lifetime, matching `codex::codex_lane_available` — installation
/// state does not change while the daemon runs). The `LOOM_SCCACHE` env
/// lookup itself is re-read on every call: it is a plain env access, not a
/// filesystem walk, so there is nothing worth caching, and caching it would
/// make an explicit override invisible to a later call in the same process.
pub(crate) fn find_sccache_path() -> Option<PathBuf> {
    match std::env::var(LOOM_SCCACHE_ENV) {
        Ok(value) => resolve_pinned(&value),
        Err(_) => cached_search_sccache_path(),
    }
}

/// Interpret an explicit `LOOM_SCCACHE` value: `"0"` disables, anything else
/// must be a path that exists on disk.
fn resolve_pinned(value: &str) -> Option<PathBuf> {
    if value == "0" {
        return None;
    }
    let path = PathBuf::from(value);
    if path.exists() {
        return Some(path);
    }
    tracing::warn!(
        path = %path.display(),
        "LOOM_SCCACHE points at a path that does not exist; sccache disabled"
    );
    None
}

fn cached_search_sccache_path() -> Option<PathBuf> {
    static CACHE: OnceLock<Option<PathBuf>> = OnceLock::new();
    CACHE.get_or_init(search_sccache_path).clone()
}

fn search_sccache_path() -> Option<PathBuf> {
    if let Ok(path) = which::which("sccache") {
        return Some(path);
    }

    let candidates = [
        dirs::home_dir().map(|h| h.join(".cargo/bin/sccache")),
        dirs::home_dir().map(|h| h.join(".local/bin/sccache")),
        Some(PathBuf::from("/opt/homebrew/bin/sccache")),
        Some(PathBuf::from("/usr/local/bin/sccache")),
    ];

    candidates.into_iter().flatten().find(|c| c.exists())
}

/// `RUSTC_WRAPPER=<path>` to export when sccache is available, or `None`.
pub(crate) fn rustc_wrapper_env() -> Option<String> {
    find_sccache_path().map(|path| format!("RUSTC_WRAPPER={}", path.display()))
}

/// Warns once when the daemon's own environment forwards an operator
/// `RUSTC_WRAPPER` that differs from the sccache path this call resolves.
/// `ENV_ALLOWLIST` (in `wrapper.rs`) forwards the operator's own
/// `RUSTC_WRAPPER` into the generated wrapper script first, but
/// [`rustc_wrapper_env`]'s own `RUSTC_WRAPPER=<resolved>` assignment is
/// appended after it, so the resolved path silently wins. Called once per
/// wrapper script, from [`rustc_wrapper_candidate`], only when a resolved
/// path exists to override.
pub(crate) fn warn_if_operator_rustc_wrapper_overridden() {
    let Some(resolved) = find_sccache_path() else {
        return;
    };
    let Ok(operator_value) = std::env::var("RUSTC_WRAPPER") else {
        return;
    };
    if std::path::Path::new(&operator_value) == resolved {
        return;
    }
    tracing::warn!(
        target: "loom::build_cache",
        "operator's RUSTC_WRAPPER={} is overridden by {} for loom sessions; set LOOM_SCCACHE=0 \
         to keep the operator's value",
        operator_value,
        resolved.display(),
    );
}

/// Resolves the `RUSTC_WRAPPER` value to export for a session that allows sccache (see
/// [`sccache_usable_in`]): the resolver's own path wins — logging via
/// [`warn_if_operator_rustc_wrapper_overridden`] when it silently overrides the operator's own
/// value forwarded through `ENV_ALLOWLIST` — else the operator's own non-empty `RUSTC_WRAPPER`,
/// exactly as forwarded. Called once per wrapper script, from `wrapper::sccache_env`.
pub(crate) fn rustc_wrapper_candidate() -> Option<String> {
    match rustc_wrapper_env() {
        Some(assignment) => {
            warn_if_operator_rustc_wrapper_overridden();
            assignment
                .strip_prefix("RUSTC_WRAPPER=")
                .map(str::to_string)
        }
        None => std::env::var("RUSTC_WRAPPER")
            .ok()
            .filter(|v| !v.is_empty()),
    }
}

/// Whether sccache can run inside a session governed by `config`.
///
/// Claude Code sandboxes each Bash command with its own network namespace, so
/// no host sccache server on 127.0.0.1:4226 is reachable, and with a seccomp
/// filter that denies `AF_UNIX` socket creation. Finding no server to talk to,
/// sccache spawns one and waits for its startup notice over an `AF_UNIX`
/// socket, which the filter refuses — only `network.allow_all_unix_sockets`
/// lifts that filter; a path-scoped `allow_unix_sockets` list cannot. The
/// wrapper script and the daemon both run outside the sandboxed Bash call, so
/// neither can probe for this failure; hence a decision from the config
/// instead of a runtime check. With the sandbox disabled entirely, no seccomp
/// filter applies and sccache runs unhindered.
pub(crate) fn sccache_usable_in(config: &crate::sandbox::MergedSandboxConfig) -> bool {
    !config.enabled || config.network.allow_all_unix_sockets
}

/// One-line operator-facing status, shared verbatim by `loom doctor` and
/// `loom run`'s startup preflight so the two surfaces can never drift apart.
pub(crate) fn sccache_status_line() -> String {
    match find_sccache_path() {
        Some(path) => format!(
            "sccache: {} (set LOOM_SCCACHE=0 to disable)",
            path.display()
        ),
        None => "sccache: not installed; each stage worktree compiles its dependencies from \
                  scratch. Install it (brew install sccache) to share them."
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// Pins an env var for a test's duration and restores it on drop.
    /// `LOOM_SCCACHE` and `PATH` are process-global, so every test touching
    /// either runs `#[serial]` (see other guards of this shape in this
    /// crate, e.g. `commands::stage::tests::state`).
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
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    /// Writes an executable placeholder file, returning its path.
    fn fake_executable(dir: &std::path::Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).unwrap();
        }
        path
    }

    #[test]
    #[serial]
    fn disabled_override_wins_even_with_a_candidate_on_path() {
        let dir = tempfile::tempdir().unwrap();
        fake_executable(dir.path(), "sccache");
        let original_path = std::env::var("PATH").unwrap_or_default();
        let _path = EnvVarGuard::set("PATH", &format!("{}:{original_path}", dir.path().display()));
        let _pin = EnvVarGuard::set(LOOM_SCCACHE_ENV, "0");
        assert!(find_sccache_path().is_none());
    }

    #[test]
    #[serial]
    fn pinned_path_is_used_when_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let fake = fake_executable(dir.path(), "sccache");
        let _pin = EnvVarGuard::set(LOOM_SCCACHE_ENV, fake.to_str().unwrap());
        assert_eq!(find_sccache_path(), Some(fake));
    }

    #[test]
    #[serial]
    fn nonexistent_pin_is_treated_as_absent() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        let _pin = EnvVarGuard::set(LOOM_SCCACHE_ENV, missing.to_str().unwrap());
        assert!(find_sccache_path().is_none());
    }

    #[test]
    #[serial]
    fn rustc_wrapper_env_wraps_the_resolved_path() {
        let dir = tempfile::tempdir().unwrap();
        let fake = fake_executable(dir.path(), "sccache");
        let _pin = EnvVarGuard::set(LOOM_SCCACHE_ENV, fake.to_str().unwrap());
        assert_eq!(
            rustc_wrapper_env(),
            Some(format!("RUSTC_WRAPPER={}", fake.display()))
        );
    }

    #[test]
    #[serial]
    fn status_line_names_the_resolved_path() {
        let dir = tempfile::tempdir().unwrap();
        let fake = fake_executable(dir.path(), "sccache");
        let _pin = EnvVarGuard::set(LOOM_SCCACHE_ENV, fake.to_str().unwrap());
        assert_eq!(
            sccache_status_line(),
            format!(
                "sccache: {} (set LOOM_SCCACHE=0 to disable)",
                fake.display()
            )
        );
    }

    #[test]
    #[serial]
    fn status_line_nudges_installation_when_disabled() {
        let _pin = EnvVarGuard::set(LOOM_SCCACHE_ENV, "0");
        assert!(sccache_status_line().starts_with("sccache: not installed"));
    }

    /// Builds a `MergedSandboxConfig` with only `enabled` and
    /// `network.allow_all_unix_sockets`/`allow_unix_sockets` varying; every
    /// other field is a default, following the pattern in
    /// `sandbox::config::tests::test_validate_config_rejects_bypass_permissions_unconditionally`.
    fn config_with(
        enabled: bool,
        network: crate::plan::schema::NetworkConfig,
    ) -> crate::sandbox::MergedSandboxConfig {
        crate::sandbox::MergedSandboxConfig {
            enabled,
            auto_allow: false,
            allow_unsandboxed_escape: false,
            excluded_commands: vec![],
            filesystem: Default::default(),
            network,
            linux: Default::default(),
            permission_mode: crate::models::stage::PermissionMode::Auto,
            implementers: Default::default(),
            command_confinement: Default::default(),
        }
    }

    #[test]
    fn sccache_usable_when_sandbox_disabled() {
        let config = config_with(false, Default::default());
        assert!(sccache_usable_in(&config));
    }

    #[test]
    fn sccache_unusable_when_sandboxed_without_unix_socket_allowance() {
        let config = config_with(true, Default::default());
        assert!(!sccache_usable_in(&config));
    }

    #[test]
    fn sccache_usable_when_sandboxed_with_all_unix_sockets_allowed() {
        let network = crate::plan::schema::NetworkConfig {
            allow_all_unix_sockets: true,
            ..Default::default()
        };
        let config = config_with(true, network);
        assert!(sccache_usable_in(&config));
    }

    #[test]
    fn sccache_unusable_when_sandboxed_with_only_a_path_allowlist() {
        let network = crate::plan::schema::NetworkConfig {
            allow_unix_sockets: vec!["/tmp/some.sock".to_string()],
            ..Default::default()
        };
        let config = config_with(true, network);
        assert!(!sccache_usable_in(&config));
    }
}
