//! Worktree settings management
//!
//! Handles creation of settings files (.claude/, CLAUDE.md) for worktrees.
//! Also supports hooks configuration when session context is available.

use anyhow::{Context, Result};
#[allow(unused_imports)] // Required for lock_shared() method on File
use fs2::FileExt;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use crate::fs::memory::SPOOL_RELPATH as MEMORY_SPOOL_RELPATH;
use crate::fs::stage_request::SPOOL_RELPATH as REQUEST_SPOOL_RELPATH;
use crate::fs::work_dir::{Layout, WorkDir};
use crate::hooks::{setup_hooks_for_worktree, HooksConfig};
use crate::plan::schema::PermissionMode;
use crate::telemetry::TELEMETRY_SPOOL_RELPATH;
const SPOOL_RELPATHS: [&str; 3] = [
    MEMORY_SPOOL_RELPATH,
    TELEMETRY_SPOOL_RELPATH,
    REQUEST_SPOOL_RELPATH,
];

/// Whether a repo-relative path is worktree scaffolding loom itself creates.
///
/// `create_worktree` plants the state-root symlink (`.loom/work` under a real
/// `.loom/` on the nested layout, `.work` on a legacy workspace — see
/// [`crate::fs::work_dir`]), `.claude/` (dir with a CLAUDE.md symlink and
/// generated settings) and — when the checkout has none — a root `CLAUDE.md`
/// symlink. Repos that gitignore these see nothing; repos that do not see
/// them as untracked. Callers reading `git status` to judge whether a
/// worktree holds *agent work* must discount them either way.
///
/// Also discounted: the memory, telemetry and stage-request spools, plus
/// `.loom/cache/`, loom's own runtime paths, written lazily during a
/// stage's execution rather than planted by `create_worktree` — but just as
/// much loom's own output, so they discount the same way. The bare `.loom`
/// entry is discounted too, for a worktree whose whole `.loom/` (holding only
/// the `work` symlink, `cache/` and the two spools) is entirely untracked, so
/// `git status` reports it as one line rather than enumerating its children.
/// This is still narrower than `.loom/` as a whole: a project may
/// legitimately track `.loom/config.toml`, which git then reports as its own
/// individual entry once anything else under `.loom/` is tracked, and that
/// entry is NOT matched by any arm below.
///
/// Keep this in sync with the scaffold `create_worktree` writes.
pub fn is_worktree_scaffold_path(path: &str) -> bool {
    let path = path.trim_end_matches('/');
    path == ".work"
        || path == "CLAUDE.md"
        || path == ".claude"
        || path.starts_with(".claude/")
        || path.starts_with(".work/")
        || path == ".loom"
        || path == ".loom/work"
        || path.starts_with(".loom/work/")
        || SPOOL_RELPATHS.contains(&path)
        || path == ".loom/cache"
        || path.starts_with(".loom/cache/")
}

/// Creates or restores the state-root symlink in a worktree.
///
/// Used during worktree creation and merge failure recovery. The link's
/// spelling follows the main repo's resolved layout ([`WorkDir::layout`]):
/// on the nested layout it points from `.worktrees/{stage_id}/.loom/work` to
/// `../../../.loom/work` (the main repo's `.loom/work/`), with `.loom/`
/// created as a real directory first; on a legacy workspace it points from
/// `.worktrees/{stage_id}/.work` to `../../.work` (the main repo's `.work/`).
pub fn ensure_work_symlink(worktree_path: &Path, repo_root: &Path) -> Result<()> {
    let work_dir = WorkDir::new(repo_root)?;
    let main_state_root = work_dir.root();
    let (link_path, target) = match work_dir.layout() {
        Layout::Nested => (
            worktree_path.join(".loom").join("work"),
            Path::new("../../../.loom/work"),
        ),
        Layout::Legacy => (worktree_path.join(".work"), Path::new("../../.work")),
    };

    if main_state_root.exists() && !link_path.exists() {
        if work_dir.layout() == Layout::Nested {
            let loom_dir = worktree_path.join(".loom");
            std::fs::create_dir_all(&loom_dir)
                .with_context(|| format!("Failed to create {} in worktree", loom_dir.display()))?;
        }

        #[cfg(unix)]
        std::os::unix::fs::symlink(target, &link_path)
            .with_context(|| "Failed to create .work symlink in worktree")?;

        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(target, &link_path)
            .with_context(|| "Failed to create .work symlink in worktree")?;
    }
    Ok(())
}

/// Set up .claude/ directory for worktree
///
/// We create a real directory and symlink CLAUDE.md from main repo.
/// settings.json is created separately by the hooks system with merged global + session hooks.
/// This ensures:
/// 1. Instructions (CLAUDE.md) are shared
/// 2. Permissions (settings.json) include both global hooks and session-specific hooks
pub fn setup_claude_directory(worktree_path: &Path, repo_root: &Path) -> Result<()> {
    let main_claude_dir = repo_root.join(".claude");
    let worktree_claude_dir = worktree_path.join(".claude");

    if main_claude_dir.exists() && !worktree_claude_dir.exists() {
        // Create real .claude/ directory in worktree
        std::fs::create_dir_all(&worktree_claude_dir)
            .with_context(|| "Failed to create .claude directory in worktree")?;

        // Symlink CLAUDE.md from main repo for instruction inheritance
        let main_claude_md = main_claude_dir.join("CLAUDE.md");
        if main_claude_md.exists() {
            let worktree_claude_md = worktree_claude_dir.join("CLAUDE.md");
            let relative_claude_md = Path::new("../../../.claude/CLAUDE.md");

            #[cfg(unix)]
            std::os::unix::fs::symlink(relative_claude_md, &worktree_claude_md)
                .with_context(|| "Failed to create CLAUDE.md symlink in worktree")?;

            #[cfg(windows)]
            std::os::windows::fs::symlink_file(relative_claude_md, &worktree_claude_md)
                .with_context(|| "Failed to create CLAUDE.md symlink in worktree")?;
        }

        // Create settings.json with trust and auto-accept settings merged with main repo settings
        let main_settings = main_claude_dir.join("settings.json");
        let worktree_settings = worktree_claude_dir.join("settings.json");
        create_worktree_settings(&main_settings, &worktree_settings, worktree_path)?;

        // Copy settings.local.json if it exists (contains user-granted runtime permissions)
        // Use file locking to prevent reading a partially written file during concurrent syncs
        let main_settings_local = main_claude_dir.join("settings.local.json");
        let worktree_settings_local = worktree_claude_dir.join("settings.local.json");
        if main_settings_local.exists() {
            copy_file_with_shared_lock(&main_settings_local, &worktree_settings_local)
                .with_context(|| "Failed to copy settings.local.json to worktree")?;
            // The main repo's copy may carry per-session identity env vars from a
            // previous main-repo session (older loom versions persisted them);
            // they must not leak into this worktree's settings.
            scrub_copied_settings_env(&worktree_settings_local);
        }
    }

    Ok(())
}

/// Symlink project-root CLAUDE.md (distinct from .claude/CLAUDE.md)
///
/// This ensures instances in worktrees have access to project instructions
/// without needing to read from the main repo outside the worktree
pub fn setup_root_claude_md(worktree_path: &Path, repo_root: &Path) -> Result<()> {
    let main_root_claude_md = repo_root.join("CLAUDE.md");
    let worktree_root_claude_md = worktree_path.join("CLAUDE.md");

    if main_root_claude_md.exists() && !worktree_root_claude_md.exists() {
        // Relative path from .worktrees/{stage_id}/CLAUDE.md to ../../CLAUDE.md
        let relative_root_claude_md = Path::new("../../CLAUDE.md");

        #[cfg(unix)]
        std::os::unix::fs::symlink(relative_root_claude_md, &worktree_root_claude_md)
            .with_context(|| "Failed to create root CLAUDE.md symlink in worktree")?;

        #[cfg(windows)]
        std::os::windows::fs::symlink_file(relative_root_claude_md, &worktree_root_claude_md)
            .with_context(|| "Failed to create root CLAUDE.md symlink in worktree")?;
    }

    Ok(())
}

/// Best-effort removal of per-session identity env vars and a stale
/// `LOOM_WORK_DIR` pin from a copied settings file. Leaves the file
/// untouched if it cannot be parsed.
fn scrub_copied_settings_env(path: &Path) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(mut settings) = serde_json::from_str::<Value>(&content) else {
        return;
    };
    let identity_removed = crate::fs::permissions::scrub_session_identity_env(&mut settings);
    let stale_work_dir_removed = crate::fs::permissions::scrub_stale_work_dir_env(&mut settings);
    if identity_removed || stale_work_dir_removed {
        if let Ok(updated) = serde_json::to_string_pretty(&settings) {
            let _ = std::fs::write(path, updated);
        }
    }
}

/// Copy a file with a shared (read) lock on the source.
///
/// This prevents reading a partially written file during concurrent writes.
/// The source file is locked with a shared lock (allowing other readers),
/// and the content is read and written to the destination atomically.
fn copy_file_with_shared_lock(src: &Path, dst: &Path) -> Result<()> {
    // Open the source file and acquire a shared lock
    let src_file = File::open(src)
        .with_context(|| format!("Failed to open source file: {}", src.display()))?;

    src_file
        .lock_shared()
        .with_context(|| format!("Failed to acquire shared lock on: {}", src.display()))?;

    // Read content while holding the lock
    let mut content = Vec::new();
    let mut reader = &src_file;
    reader
        .read_to_end(&mut content)
        .with_context(|| format!("Failed to read source file: {}", src.display()))?;

    // Lock is released when src_file is dropped, but we can write to dst now
    // since we have the complete content

    // Write to destination
    let mut dst_file = File::create(dst)
        .with_context(|| format!("Failed to create destination file: {}", dst.display()))?;

    dst_file
        .write_all(&content)
        .with_context(|| format!("Failed to write to destination file: {}", dst.display()))?;

    Ok(())
}

/// Merge permissions from main repo's settings.local.json into a worktree.
///
/// This is the public interface for refreshing permissions in a worktree.
/// Instead of overwriting, it merges permissions from both sources:
/// - Permissions from the main repo's settings.local.json
/// - Existing permissions in the worktree's settings.local.json (if any)
///
/// This ensures worktree-specific permissions are preserved while still
/// receiving updates from the main repo.
pub fn refresh_worktree_settings_local(worktree_path: &Path, repo_root: &Path) -> Result<bool> {
    let main_settings_local = repo_root.join(".claude/settings.local.json");
    let worktree_settings_local = worktree_path.join(".claude/settings.local.json");

    if !main_settings_local.exists() {
        return Ok(false);
    }

    // Ensure .claude directory exists in worktree
    let worktree_claude_dir = worktree_path.join(".claude");
    if !worktree_claude_dir.exists() {
        std::fs::create_dir_all(&worktree_claude_dir)
            .with_context(|| "Failed to create .claude directory in worktree")?;
    }

    // Read main repo settings with shared lock
    let main_settings = read_settings_with_shared_lock(&main_settings_local)?;

    // Read existing worktree settings (if any)
    let worktree_settings = if worktree_settings_local.exists() {
        read_settings(&worktree_settings_local)?
    } else {
        json!({})
    };

    // Extract permissions from both
    let (main_allow, main_deny) = extract_permissions(&main_settings);
    let (wt_allow, wt_deny) = extract_permissions(&worktree_settings);

    // Merge permissions (union with deduplication)
    let merged_allow = merge_permission_vecs(main_allow, wt_allow);
    let mut merged_deny = merge_permission_vecs(main_deny, wt_deny);
    merged_deny.retain(|perm| !perm.starts_with("Read("));

    // Build merged settings. The base MUST be the worktree's own settings when they exist:
    // they carry session-specific hooks and the stage-resolved permission mode. Using the main
    // repo's settings as base (as this function once did) clobbered all of that mid-session
    // with whatever the last main-repo session left behind. Only permissions are refreshed
    // from the main repo. When the worktree has no settings yet, fall back to the main copy,
    // scrubbed of per-session identity env vars.
    let mut merged = if worktree_settings_local.exists() {
        worktree_settings
    } else {
        let mut base = main_settings.clone();
        crate::fs::permissions::scrub_session_identity_env(&mut base);
        crate::fs::permissions::scrub_stale_work_dir_env(&mut base);
        base
    };
    set_permissions(&mut merged, merged_allow, merged_deny)?;

    // Write merged result
    let content =
        serde_json::to_string_pretty(&merged).with_context(|| "Failed to serialize settings")?;
    std::fs::write(&worktree_settings_local, content)
        .with_context(|| format!("Failed to write {}", worktree_settings_local.display()))?;

    Ok(true)
}

/// Read and parse a settings.json file with a shared lock
fn read_settings_with_shared_lock(path: &Path) -> Result<Value> {
    let file = File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;

    file.lock_shared()
        .with_context(|| format!("Failed to acquire shared lock on {}", path.display()))?;

    let mut content = String::new();
    let mut reader = &file;
    reader
        .read_to_string(&mut content)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    // Lock released when file is dropped
    serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse {} as JSON", path.display()))
}

/// Read and parse a settings.json file
fn read_settings(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse {} as JSON", path.display()))
}

/// Extract allow and deny permission arrays from settings
fn extract_permissions(settings: &Value) -> (Vec<String>, Vec<String>) {
    let permissions = settings.get("permissions");

    let allow = permissions
        .and_then(|p| p.get("allow"))
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let deny = permissions
        .and_then(|p| p.get("deny"))
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    (allow, deny)
}

/// Merge two permission vectors, removing duplicates
fn merge_permission_vecs(a: Vec<String>, b: Vec<String>) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut result = Vec::new();

    for perm in a.into_iter().chain(b) {
        if seen.insert(perm.clone()) {
            result.push(perm);
        }
    }

    result
}

/// Set permissions in a settings Value
fn set_permissions(settings: &mut Value, allow: Vec<String>, deny: Vec<String>) -> Result<()> {
    let obj = settings
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Settings must be a JSON object"))?;

    let permissions = obj
        .entry("permissions")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("permissions must be a JSON object"))?;

    if !allow.is_empty() {
        permissions.insert("allow".to_string(), json!(allow));
    }
    if !deny.is_empty() {
        permissions.insert("deny".to_string(), json!(deny));
    }

    Ok(())
}

/// Create settings.json for a worktree with trust setting and inherited config.
///
/// This function:
/// 1. Reads the main repo's settings.json (if it exists)
/// 2. Sets `hasTrustDialogAccepted: true` to skip the trust prompt
/// 3. Strips stale per-session identity vars (`LOOM_MAIN_AGENT_PID`,
///    `LOOM_STAGE_ID`, `LOOM_SESSION_ID`) and a stale `LOOM_WORK_DIR` pin
///    from the inherited `env` block
/// 4. Writes the merged result to the worktree
///
/// Note: We deliberately do NOT write `permissions.defaultMode` here. The
/// resolved permission mode (stage-type default + plan override + stage
/// override) lives in `settings.local.json` written by `sandbox::write_settings`
/// using `apply_default_mode`. Writing it here would race the sandbox-merge
/// step and undercut the resolved value. See finding #5 (option 2).
///
/// This creates the base settings.json. The hooks system later merges in
/// session-specific hooks via setup_worktree_hooks().
fn create_worktree_settings(
    main_settings: &Path,
    worktree_settings: &Path,
    worktree_path: &Path,
) -> Result<()> {
    // Start with main repo settings or empty object
    let mut settings: Value = if main_settings.exists() {
        let content = std::fs::read_to_string(main_settings)
            .with_context(|| "Failed to read main repo settings.json")?;
        serde_json::from_str(&content).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    // Ensure settings is an object
    let obj = settings
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("settings.json must be a JSON object"))?;

    // Set hasTrustDialogAccepted to skip the trust prompt
    obj.insert("hasTrustDialogAccepted".to_string(), json!(true));

    // Ensure a permissions object exists so the `.work` allow-list block below can attach to it.
    // A hand-edited settings.json can carry `permissions` as some other JSON type; normalize
    // rather than leave it, since the state-root allow block below depends on this being an
    // object. We intentionally do NOT seed `defaultMode` here — that's the sandbox-resolved
    // value's job (see fn-level docs).
    if !matches!(obj.get("permissions"), Some(Value::Object(_))) {
        obj.insert("permissions".to_string(), json!({}));
    }

    // Scrub the copied env block: per-session identity (LOOM_MAIN_AGENT_PID,
    // LOOM_STAGE_ID, LOOM_SESSION_ID) is set dynamically by the wrapper
    // script, so any inherited value is stale. A LOOM_WORK_DIR pin naming a
    // now-deleted .work/ is scrubbed alongside — see scrub_stale_work_dir_env.
    crate::fs::permissions::scrub_session_identity_env(&mut settings);
    crate::fs::permissions::scrub_stale_work_dir_env(&mut settings);

    // Resolve the state-root symlink to its absolute target path and add
    // permissions. Claude Code resolves symlinks before checking permission
    // patterns, so the relative Read(.loom/work/**) pattern from the main
    // repo's settings doesn't match the resolved absolute path. Adding the
    // resolved path here ensures agents can read/write state files without
    // permission prompts. See `fs::permissions::state_root` for the shared
    // resolution and the S-1 rationale (blanket read/write over this path
    // exposes `admin.token` / `user.token` — a daemon RPC privilege
    // escalation).
    //
    // IMPORTANT: Claude Code requires the // prefix for absolute filesystem paths.
    // A single / means "relative to project root", NOT absolute. See:
    // https://code.claude.com/docs/en/permissions.md
    if let Some(resolved) = crate::fs::permissions::state_root::resolve_state_root(worktree_path) {
        let resolved_str = resolved.to_string_lossy();

        // Collect the permissions to add
        // Use / prefix on absolute paths for Claude Code's // convention
        //
        // There is deliberately NO broad write grant over the resolved
        // `.work` root here, in either spelling. `Edit(/{resolved}/**)`
        // would restore the grant narrowed elsewhere (S-1, see
        // `sandbox/settings.rs`) because it exposed `.work/admin.token`
        // and `.work/user.token` to a sandboxed worktree agent — a daemon
        // RPC privilege escalation. `Write(/{resolved}/**)` sat here as
        // its inert stand-in until it was REMOVED rather than converted:
        // Claude Code's permission check consults only `Edit(path)`, so it
        // granted nothing and warned every session start.
        //
        // For the same S-1 reason, there is also no blanket
        // `Read(/{resolved}/**)` grant: it exposed `admin.token` and
        // `user.token` to read just as readily as a broad `Edit` would
        // have exposed them to write. The three narrow entries below are
        // the whole read grant.
        //
        // This file (`.claude/settings.json`) is written by a different
        // code path than `settings.local.json`, so it must be safe
        // standalone rather than depending on `sandbox::write_settings`
        // always running afterwards. (settings.json is the team-shareable
        // file per `fs/permissions/settings.rs`'s module doc, though
        // `.claude/` is gitignored in this repo.)
        //
        // It carries no `Read(...)` deny either, in any shape: Claude Code reads
        // EVERY settings file when deciding whether a Bash command touches a
        // denied path, and one `Read(` deny rule anywhere makes every
        // relative-path `rg`/`grep`/`diff`/`git`/`cp`/`mv` after a `cd` prompt the
        // operator. The token files are protected instead by the OS-level
        // `sandbox.filesystem.denyRead` list written into `settings.local.json`
        // and by `hooks/credential-guard.sh` for the native file tools. The narrow
        // entries below are the whole read grant.
        let work_perms = vec![
            format!("Read(/{}/signals/**)", resolved_str),
            format!("Read(/{}/config.toml)", resolved_str),
            format!("Read(/{}/handoffs/**)", resolved_str),
        ];

        // Get or create the allow array within permissions.
        // `permissions` is guaranteed to be an object here (normalized
        // above), but a hand-edited settings.json can still carry `allow`
        // as some other JSON type. `array_entry` normalizes that to an
        // empty array rather than skip the state-root grant below.
        if let Some(perms_obj) = settings
            .as_object_mut()
            .and_then(|o| o.get_mut("permissions"))
            .and_then(|p| p.as_object_mut())
        {
            push_unique_perms(array_entry(perms_obj, "allow"), work_perms);
        }
    }

    // Write the merged settings
    let content =
        serde_json::to_string_pretty(&settings).with_context(|| "Failed to serialize settings")?;
    std::fs::write(worktree_settings, content)
        .with_context(|| "Failed to write worktree settings.json")?;

    Ok(())
}

/// Return `key`'s array within `obj`, replacing a missing or wrong-typed
/// value with an empty array first. A hand-edited settings.json must never
/// cause the narrow state-root allow entries to be silently skipped.
fn array_entry<'a>(obj: &'a mut Map<String, Value>, key: &str) -> &'a mut Vec<Value> {
    if !matches!(obj.get(key), Some(Value::Array(_))) {
        obj.insert(key.to_string(), json!([]));
    }
    obj.get_mut(key)
        .and_then(|v| v.as_array_mut())
        .expect("just inserted or verified an array")
}

/// Push each of `perms` onto `arr` unless an equal string is already present.
fn push_unique_perms(arr: &mut Vec<Value>, perms: impl IntoIterator<Item = String>) {
    for perm in perms {
        if !arr.iter().any(|v| v.as_str() == Some(perm.as_str())) {
            arr.push(json!(perm));
        }
    }
}

/// Configure hooks for a worktree with session context
///
/// This adds Claude Code hooks to the worktree's .claude/settings.json.
/// Hooks enable:
/// - Auto-handoff on PreCompact (context exhaustion)
/// - Learning protection via Stop hook
/// - Session lifecycle tracking
///
/// Session identity (stage/session IDs) is NOT written here: hooks read it
/// from the process environment exported by the session wrapper script.
pub fn setup_worktree_hooks(
    worktree_path: &Path,
    work_dir: &Path,
    hooks_dir: &Path,
    permission_mode: PermissionMode,
) -> Result<()> {
    // Canonicalize work_dir to absolute path so hooks work regardless of
    // Claude Code's current working directory. This fixes "spawn /bin/sh ENOENT"
    // errors that occur when hooks run from a deleted/changed directory.
    let absolute_work_dir = work_dir
        .canonicalize()
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default().join(work_dir));

    let config = HooksConfig::new(hooks_dir.to_path_buf(), absolute_work_dir, permission_mode);

    setup_hooks_for_worktree(worktree_path, &config).with_context(|| {
        format!(
            "Failed to setup hooks for worktree: {}",
            worktree_path.display()
        )
    })
}

/// Append a pattern to a git `info/exclude` file, creating it if absent.
///
/// Idempotent: skips the write when the pattern is already present.
fn add_to_gitignore_exclude(git_dir: &Path, pattern: &str) -> Result<()> {
    let info_dir = git_dir.join("info");
    std::fs::create_dir_all(&info_dir)
        .with_context(|| format!("Failed to create {}", info_dir.display()))?;
    let exclude_path = info_dir.join("exclude");

    if exclude_path.exists() {
        let content = std::fs::read_to_string(&exclude_path)
            .with_context(|| format!("Failed to read {}", exclude_path.display()))?;
        if content.lines().any(|line| line.trim() == pattern) {
            return Ok(());
        }
        let newline = if content.ends_with('\n') { "" } else { "\n" };
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&exclude_path)
            .with_context(|| format!("Failed to open {} for append", exclude_path.display()))?;
        file.write_all(format!("{newline}{pattern}\n").as_bytes())
            .with_context(|| "Failed to append to gitignore exclude")?;
    } else {
        std::fs::write(
            &exclude_path,
            format!("# loom: per-worktree generated paths\n{pattern}\n"),
        )
        .with_context(|| format!("Failed to create {}", exclude_path.display()))?;
    }

    Ok(())
}

/// Patterns loom excludes from git's view of every worktree: the previous
/// session's `.claude/settings.local.json` and loom's `.loom/cache/`, matching
/// this repo's own `.gitignore`. Spool paths come from [`SPOOL_RELPATHS`].
const WORKTREE_EXCLUDE_PATTERNS: &[&str] = &[".claude/settings.local.json", ".loom/cache/"];

/// Write [`WORKTREE_EXCLUDE_PATTERNS`] and [`SPOOL_RELPATHS`] into `git_dir`'s `info/exclude`.
fn add_worktree_exclude_patterns(git_dir: &Path) -> Result<()> {
    for pattern in WORKTREE_EXCLUDE_PATTERNS.iter().chain(&SPOOL_RELPATHS) {
        add_to_gitignore_exclude(git_dir, pattern)?;
    }
    Ok(())
}

/// Exclude loom's own runtime paths from git's view of a worktree.
///
/// Writes to the repository's COMMON `.git/info/exclude`, not a per-worktree
/// file. Git worktrees each get their own metadata directory at
/// `<repo>/.git/worktrees/<stage-id>/`, but `info/exclude` is not among the
/// files git treats as per-worktree there — `git status` always resolves
/// `info/exclude` to the common git dir (`<repo>/.git/info/exclude`), shared
/// by every worktree and the main checkout alike. A previous version of this
/// function wrote to `.git/worktrees/<stage-id>/info/exclude`, believing it
/// acted as an exclude file scoped to that worktree; git never reads that
/// path, so the write was silently inert. Writing to the common dir instead
/// means this function and [`add_settings_local_to_main_gitignore`] now
/// target the same file — kept as two entry points because their callers
/// (worktree creation vs. main-repo knowledge stages) are otherwise
/// unrelated, and future patterns may diverge between them.
pub fn add_settings_local_to_worktree_gitignore(repo_root: &Path) -> Result<()> {
    add_worktree_exclude_patterns(&repo_root.join(".git"))
}

/// Exclude loom's own runtime paths from the main repo's git exclude.
///
/// Used for knowledge stages that run in the main repo without a dedicated worktree.
pub fn add_settings_local_to_main_gitignore(repo_root: &Path) -> Result<()> {
    add_worktree_exclude_patterns(&repo_root.join(".git"))
}

/// Remove worktree-specific settings and symlinks
///
/// Called during worktree removal to clean up:
/// - state-root symlink (`.loom/work` and/or `.work`)
/// - .claude directory (or legacy symlink)
/// - root CLAUDE.md symlink
///
/// Unconditional: it removes `.claude/` and root `CLAUDE.md` whenever they
/// exist, whatever planted them, so it is only safe ahead of `git worktree
/// remove --force` (its sole production caller is the spawn-failure path in
/// `orchestrator/core/stage_executor.rs`). The non-forced removal path must
/// use `git::cleanup::remove_worktree_scaffold` instead, which removes only
/// what loom planted (and leaves anything git tracks alone).
pub fn cleanup_worktree_settings(worktree_path: &Path) {
    // Remove the state-root symlink(s) first to avoid issues. Both paths are
    // no-ops when absent, so this is correct whichever layout planted the
    // link — no layout lookup needed.
    for link in [
        worktree_path.join(".loom").join("work"),
        worktree_path.join(".work"),
    ] {
        if link.exists() || link.is_symlink() {
            std::fs::remove_file(&link).ok(); // Ignore errors
        }
    }
    // Tidy up `.loom/` if removing its `work` link left it empty; harmless
    // no-op otherwise (e.g. it still holds the memory spool or cache).
    std::fs::remove_dir(worktree_path.join(".loom")).ok();

    // Remove the .claude directory (it's a real directory now, not a symlink)
    let claude_dir = worktree_path.join(".claude");
    if claude_dir.exists() {
        std::fs::remove_dir_all(&claude_dir).ok(); // Ignore errors
    } else if claude_dir.is_symlink() {
        // Handle legacy symlink case
        std::fs::remove_file(&claude_dir).ok();
    }

    // Remove the root CLAUDE.md symlink
    let root_claude_md = worktree_path.join("CLAUDE.md");
    if root_claude_md.exists() || root_claude_md.is_symlink() {
        std::fs::remove_file(&root_claude_md).ok(); // Ignore errors
    }
}

#[cfg(test)]
#[path = "tests_settings.rs"]
mod tests;
#[cfg(test)]
#[path = "tests_settings_env.rs"]
mod tests_settings_env;
