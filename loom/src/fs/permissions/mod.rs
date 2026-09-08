//! Claude Code and Codex permissions and hook management for Loom.
//!
//! Manages two settings files:
//! - `.claude/settings.json` - team-shared permissions (committed to git)
//! - `.claude/settings.local.json` - user-local hooks and env vars (gitignored)

mod codex_hooks;
mod codex_sandbox;
pub mod constants;
mod drift;
mod hooks;
pub(crate) mod settings;
pub(crate) mod state_root;
mod sync;
mod trust;
pub(crate) mod write_rules;

#[cfg(test)]
mod tests;

// Re-export public API
pub use codex_hooks::{
    codex_hooks_need_install, codex_hooks_need_install_in, install_codex_hooks,
    install_codex_hooks_to,
};
pub use codex_sandbox::settings_local_has_allowances as settings_local_has_codex_sandbox;
pub use constants::{LOOM_PERMISSIONS, LOOM_PERMISSIONS_WORKTREE};
pub use drift::{
    hook_drift, hook_drift_for_dir, hook_scripts_needing_install,
    loom_hook_scripts_needing_install, main_repo_settings_identity_drift,
    settings_local_has_agent_teams_env, settings_local_has_worktree_isolation_disabled,
    settings_local_hook_drift, HookDrift,
};
pub use hooks::{
    configure_loom_hooks, get_installed_hooks_dir, install_loom_hooks, install_loom_hooks_to,
    loom_hooks_config,
};
pub use settings::{
    ensure_loom_hooks_local, ensure_loom_permissions, ensure_loom_permissions_quiet,
    ensure_loom_permissions_to, scrub_main_repo_settings_identity, scrub_session_identity_env,
    scrub_stale_work_dir_env, settings_json_has_hooks, SESSION_IDENTITY_ENV_KEYS,
};
pub use sync::{sync_worktree_permissions, sync_worktree_permissions_with_working_dir, SyncResult};
pub use trust::{migrate_legacy_trust, trust_worktree, untrust_worktree};
