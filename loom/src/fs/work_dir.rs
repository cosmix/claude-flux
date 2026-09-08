use anyhow::{bail, Context, Result};
use std::fs;
use std::os::unix::fs::DirBuilderExt;
#[cfg(test)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::fs::knowledge::KnowledgeDir;

mod config_sections;
pub use config_sections::{
    insert_key, read_config, read_context_config, read_plan_sandbox, read_remote_control_config,
    read_terminal_config, remove_key, resolve_context_ceiling_tokens, update_config, write_config,
    write_context_config, write_plan_sandbox, write_remote_control_config, write_terminal_config,
};

mod context_config;
pub use context_config::ContextConfig;

/// Parsed config.toml structure
#[derive(Debug, Clone)]
pub struct Config {
    inner: toml::Value,
}

impl Config {
    /// Get a string value from the plan section (e.g., "source_path", "base_branch", "plan_id")
    pub fn get_plan_str(&self, key: &str) -> Option<&str> {
        self.inner
            .get("plan")
            .and_then(|p| p.get(key))
            .and_then(|v| v.as_str())
    }

    /// Get the plan source path
    pub fn source_path(&self) -> Option<PathBuf> {
        self.get_plan_str("source_path").map(PathBuf::from)
    }

    /// Get the base branch for merging
    pub fn base_branch(&self) -> Option<String> {
        self.get_plan_str("base_branch").map(String::from)
    }

    /// Get the plan ID
    pub fn plan_id(&self) -> Option<&str> {
        self.get_plan_str("plan_id")
    }

    /// Get mutable access to the underlying TOML value for updates
    pub fn as_toml_mut(&mut self) -> &mut toml::Value {
        &mut self.inner
    }

    /// Serialize the config back to TOML string
    pub fn to_toml_string(&self) -> Result<String> {
        toml::to_string_pretty(&self.inner).context("Failed to serialize config")
    }
}

/// Load and parse config.toml from a work directory
///
/// # Arguments
/// * `work_dir` - Path to the `.loom/work` directory (not the config file itself)
///
/// # Returns
/// * `Ok(Some(Config))` - Config loaded and parsed successfully
/// * `Ok(None)` - Config file doesn't exist
/// * `Err(_)` - Failed to read or parse config
pub fn load_config(work_dir: &Path) -> Result<Option<Config>> {
    let config_path = work_dir.join("config.toml");

    if !config_path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&config_path).context("Failed to read config.toml")?;

    let inner: toml::Value = toml::from_str(&content).context("Failed to parse config.toml")?;

    Ok(Some(Config { inner }))
}

/// Load config.toml, returning an error if it doesn't exist
///
/// Use this when config.toml is required (e.g., during execution).
pub fn load_config_required(work_dir: &Path) -> Result<Config> {
    load_config(work_dir)?
        .ok_or_else(|| anyhow::anyhow!("No active plan. Run 'loom init <plan-path>' first."))
}

/// Directory holding loom's per-project data; `work/` is one of its children,
/// `cache/` (written by `loom map`) another.
const LOOM_DIR: &str = ".loom";
/// The state root's own name under [`LOOM_DIR`].
const WORK_DIR: &str = "work";
/// The pre-move spelling of the state root, directly under the project root.
const LEGACY_WORK_DIR: &str = ".work";

/// Which on-disk spelling a resolved workspace uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// `<repo>/.loom/work` — the current layout.
    Nested,
    /// `<repo>/.work` — a workspace created before the move.
    Legacy,
}

/// The workspace rooted at `dir`, if either layout has a `config.toml` there.
///
/// Keyed on the config FILE, never on directory existence: `~/.loom/config.toml`
/// is a user-level file and `.loom/cache/` appears in any project that has run
/// `loom map`, so a bare `.loom/` marks nothing. Nested wins over legacy when
/// both are present.
fn workspace_at(dir: &Path) -> Option<(PathBuf, Layout)> {
    let nested = dir.join(LOOM_DIR).join(WORK_DIR);
    if nested.join("config.toml").exists() {
        return Some((nested, Layout::Nested));
    }
    let legacy = dir.join(LEGACY_WORK_DIR);
    if legacy.join("config.toml").exists() {
        return Some((legacy, Layout::Legacy));
    }
    None
}

/// The layout `base` names when it already IS a state root rather than a
/// project root, in either spelling.
///
/// Hook entry points (see `commands/hook/reconcile_graph.rs`) hand `WorkDir::new`
/// `LOOM_WORK_DIR`, which names the state directory ITSELF, not its parent — so
/// a `base` that already names one must resolve to itself rather than get a
/// second state root appended under it. Both spellings need recognising: after
/// the move the pinned value ends `.loom/work`, whose final component alone is
/// the unremarkable `work`, while a workspace created before the move still
/// pins a single `.work`. Miss either and a stale pin materializes a phantom
/// `<...>/.loom/work/.loom/work` (or `<...>/.work/.work`), whose `repo_root()`
/// is the state directory itself. `initialize()` creates the root this returns,
/// so the branch keeps that creation correct for a state-root-named hint too.
fn base_names_state_root(base: &Path) -> Option<Layout> {
    let name = base.file_name()?;
    if name == std::ffi::OsStr::new(WORK_DIR)
        && base.parent().and_then(Path::file_name) == Some(std::ffi::OsStr::new(LOOM_DIR))
    {
        return Some(Layout::Nested);
    }
    if name == std::ffi::OsStr::new(LEGACY_WORK_DIR) {
        return Some(Layout::Legacy);
    }
    None
}

/// The nearest ancestor of `dir` (inclusive) holding a `.git` entry, or `None`
/// when there is none.
///
/// This is the bound on the upward workspace search: a `.git` marks the one
/// tree whose `config.toml` can legitimately be this base's workspace. `.git`
/// is an ENTRY, not necessarily a directory — a linked worktree's is a file
/// pointing at the main repo's gitdir — so existence, not `is_dir`, is the
/// test.
fn nearest_git_root(dir: &Path) -> Option<&Path> {
    let mut current = dir;
    loop {
        if current.join(".git").exists() {
            return Some(current);
        }
        match current.parent() {
            Some(parent) if parent != current => current = parent,
            _ => return None,
        }
    }
}

/// Apply the layout's hop count to a state-root path.
fn repo_root_of(root: &Path, layout: Layout) -> Option<&Path> {
    match layout {
        Layout::Nested => root.parent()?.parent(),
        Layout::Legacy => root.parent(),
    }
}

pub struct WorkDir {
    root: PathBuf,
    layout: Layout,
}

impl WorkDir {
    /// Resolve the workspace for `base_path`, picking the root exactly once.
    ///
    /// Whatever root resolves is the workspace for reads AND writes: a project
    /// still on `.work/` keeps getting its signals, stages and config writes
    /// there. Nothing here ever produces a `.work/` that does not already
    /// exist — the fallback is always the nested layout, so `initialize()` on a
    /// fresh repo lands at `.loom/work`.
    pub fn new<P: AsRef<Path>>(base_path: P) -> Result<Self> {
        let base = base_path.as_ref();
        // `base` itself first, uncanonicalized: callers passing "." (see
        // `commands/status.rs`) get a root relative to it, as they always have.
        if let Some((root, layout)) = workspace_at(base) {
            return Ok(Self { root, layout });
        }

        // Search upward for a workspace (like git searches for .git), bounded
        // at the enclosing repository: the walk covers `base` up to and
        // including the nearest ancestor holding a `.git`, and NO base outside
        // a repository walks at all. Without that second half the walk ran to
        // `/`, inspecting `$HOME` and the OS temp root on the way, and silently
        // adopted the first `config.toml` it met — so one `loom init` in
        // `$HOME` would claim every later command issued from any non-git
        // directory beneath it, for writes as well as reads.
        if let Ok(abs) = base.canonicalize() {
            if let Some(repo_root) = nearest_git_root(&abs) {
                let mut current = abs.as_path();
                loop {
                    if let Some((root, layout)) = workspace_at(current) {
                        return Ok(Self { root, layout });
                    }
                    if current == repo_root {
                        break;
                    }
                    match current.parent() {
                        Some(parent) if parent != current => current = parent,
                        _ => break,
                    }
                }
            }
        }

        if let Some(layout) = base_names_state_root(base) {
            return Ok(Self {
                root: base.to_path_buf(),
                layout,
            });
        }

        Ok(Self {
            root: base.join(LOOM_DIR).join(WORK_DIR),
            layout: Layout::Nested,
        })
    }

    /// The spelling of the resolved state root.
    pub fn layout(&self) -> Layout {
        self.layout
    }

    pub fn initialize(&self) -> Result<()> {
        if self.root.exists() {
            bail!("{} already exists", self.root.display());
        }

        // Recursive: the nested layout's `.loom/` parent may not exist yet.
        let mut root_builder = fs::DirBuilder::new();
        root_builder.recursive(true).mode(0o700);
        root_builder
            .create(&self.root)
            .with_context(|| format!("Failed to create {}", self.root.display()))?;

        self.ensure_layout()
    }

    /// Adopt a state directory that already exists on disk but holds no
    /// orchestration state — e.g. a phantom directory a stale `LOOM_WORK_DIR`
    /// pin caused a hook to materialize against a since-deleted workspace.
    /// `commands/init/execute.rs` decides when that is actually safe (no
    /// `config.toml`, no stage/session files) before calling this.
    ///
    /// Requires `self.root` to already exist — bails otherwise, unlike
    /// `initialize()`, which requires the opposite and creates it. Never
    /// creates the root itself, so a caller's `InitGuard` must not delete a
    /// directory this call did not create.
    pub fn adopt_existing(&self) -> Result<()> {
        if !self.root.exists() {
            bail!("{} does not exist; nothing to adopt", self.root.display());
        }

        self.ensure_layout()
    }

    /// The layout work both `initialize()` and `adopt_existing()` need once
    /// `self.root` exists: the private-mode subdirectories, the README, and
    /// the knowledge directory. Idempotent — skips anything already present,
    /// so `adopt_existing()` can call it against a state root that already
    /// holds some (but not all) of this layout.
    fn ensure_layout(&self) -> Result<()> {
        // Session wrappers, PID files, captured stderr and the memory journal live here.
        let subdirs = [
            "signals", "handoffs", "archive", "stages", "sessions", "crashes", "memory",
            "wrappers", "pids", "logs",
        ];

        for subdir in &subdirs {
            let path = self.root.join(subdir);
            if path.exists() {
                continue;
            }
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            builder
                .create(&path)
                .with_context(|| format!("Failed to create {subdir} directory"))?;
        }

        if !self.root.join("README.md").exists() {
            self.create_readme()?;
        }

        // Initialize knowledge directory with template files.
        // KnowledgeDir expects the project root, not the state root — the hop
        // count differs per layout, which is why this goes through
        // `project_root()` rather than a bare `parent()`.
        if let Some(project_root) = self.project_root() {
            let knowledge = KnowledgeDir::new(project_root);
            knowledge.initialize()?;
        }

        Ok(())
    }

    pub fn load(&self) -> Result<()> {
        if !self.root.exists() {
            bail!(
                "{} does not exist. Run 'loom init' first.",
                self.root.display()
            );
        }

        self.validate_structure()?;

        Ok(())
    }

    fn validate_structure(&self) -> Result<()> {
        let required_dirs = [
            "signals", "handoffs", "archive", "stages", "sessions", "crashes", "memory",
            "wrappers", "pids", "logs",
        ];

        for dir in &required_dirs {
            let path = self.root.join(dir);
            if !path.exists() {
                // Auto-create missing directories instead of failing, at the
                // same 0o700 mode `initialize()`/`ensure_layout()` use — a
                // plain `create_dir` would land these at the process umask
                // instead, inside an otherwise-0700 state root.
                let mut builder = fs::DirBuilder::new();
                builder.mode(0o700);
                builder
                    .create(&path)
                    .with_context(|| format!("Failed to create missing directory: {dir}"))?;
            }
        }

        Ok(())
    }

    fn create_readme(&self) -> Result<()> {
        let readme_content = r#"# loom Work Directory

This directory is managed by loom CLI and contains:

- `signals/` - Inter-agent communication
- `handoffs/` - Context handoff records
- `archive/` - Archived entities
- `stages/` - Stage definitions and status
- `sessions/` - Active session tracking
- `crashes/` - Crash reports and diagnostics
- `logs/` - Per-session captured stderr (`<session-id>.stderr.log`)
- `knowledge/` - Curated codebase knowledge (entry points, patterns, conventions)

Do not manually edit these files unless you know what you're doing.
"#;

        let readme_path = self.root.join("README.md");
        fs::write(readme_path, readme_content).context("Failed to create README.md")?;

        Ok(())
    }

    pub fn signals_dir(&self) -> PathBuf {
        self.root.join("signals")
    }

    pub fn handoffs_dir(&self) -> PathBuf {
        self.root.join("handoffs")
    }

    pub fn archive_dir(&self) -> PathBuf {
        self.root.join("archive")
    }

    pub fn stages_dir(&self) -> PathBuf {
        self.root.join("stages")
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }

    pub fn crashes_dir(&self) -> PathBuf {
        self.root.join("crashes")
    }

    pub fn knowledge_dir(&self) -> PathBuf {
        self.root.join("knowledge")
    }

    /// Path to `.loom/work/disputes/` — adjudication artifacts. See
    /// `models/dispute.rs` for the per-id directory schema.
    pub fn disputes_dir(&self) -> PathBuf {
        self.root.join("disputes")
    }

    /// Path to `.loom/work/plan_versions/` — plan amendment snapshots and
    /// audit log. Populated by the Stage 3 plan-amendment pipeline.
    pub fn plan_versions_dir(&self) -> PathBuf {
        self.root.join("plan_versions")
    }

    /// Get the config.toml path
    pub fn config_path(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    /// Ensure a subdirectory exists, creating it if needed
    ///
    /// # Arguments
    /// * `name` - The subdirectory name relative to the state root
    ///
    /// # Returns
    /// The full path to the directory
    pub fn ensure_dir(&self, name: &str) -> Result<PathBuf> {
        let dir = self.root.join(name);
        fs::create_dir_all(&dir).with_context(|| format!("Failed to create {name} directory"))?;
        Ok(dir)
    }

    /// Load and parse config.toml
    ///
    /// # Returns
    /// * `Ok(Some(Config))` - Config loaded and parsed successfully
    /// * `Ok(None)` - Config file doesn't exist
    /// * `Err(_)` - Failed to read or parse config
    pub fn load_config(&self) -> Result<Option<Config>> {
        load_config(&self.root)
    }

    /// Load config.toml, returning an error if it doesn't exist
    pub fn load_config_required(&self) -> Result<Config> {
        load_config_required(&self.root)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The project root holding this state directory, layout-aware.
    ///
    /// Two hops up from `.loom/work`, one from a legacy `.work`. Every caller
    /// goes through here (or [`Self::project_root`], its alias) so the hop
    /// count exists in exactly one place.
    pub fn repo_root(&self) -> Option<&Path> {
        repo_root_of(&self.root, self.layout)
    }

    /// Get the project root.
    pub fn project_root(&self) -> Option<&Path> {
        self.repo_root()
    }

    /// Get the main project root by following symlinks.
    ///
    /// In a worktree the state root is a symlink into the main repo's:
    /// `.loom/work -> ../../../.loom/work`, or `.work -> ../../.work` under the
    /// legacy layout. This method resolves that symlink to find the true main
    /// repository root.
    ///
    /// - If the state root is a symlink, follows it and applies the layout's
    ///   hop count to the resolved path.
    /// - Otherwise returns the regular project root (same as `project_root()`).
    pub fn main_project_root(&self) -> Option<PathBuf> {
        if self.root.is_symlink() {
            // Read the symlink target
            if let Ok(link_target) = fs::read_link(&self.root) {
                // If the symlink is relative, resolve it against the directory
                // holding the link itself.
                let resolved = if link_target.is_relative() {
                    self.root.parent()?.join(&link_target)
                } else {
                    link_target
                };

                // Canonicalize to get the absolute path
                if let Ok(canonical) = resolved.canonicalize() {
                    return repo_root_of(&canonical, self.layout).map(|p| p.to_path_buf());
                }
            }
            None
        } else {
            // Not a symlink, return regular project root
            self.project_root().map(|p| p.to_path_buf())
        }
    }
}

#[cfg(test)]
mod tests;
