//! Operational CLI command types: plans, sessions, worktrees, and the
//! deterministic context/hook entry points.
//!
//! Split out of `types.rs` for the same reason as `types_memory.rs` and
//! `types_stage.rs`: the top-level `Commands` enum is large enough on its own,
//! and every new subcommand family that lands beside it costs the file lines it
//! does not have. `types.rs` re-exports everything here, so callers keep naming
//! a single `cli::types` surface.

use crate::validation::clap_id_validator;
use clap::Subcommand;

#[derive(Subcommand)]
pub enum PlanCommands {
    /// Verify a plan file without side effects (no .loom/work/, no git repo required)
    Verify {
        /// Path to the plan file to validate
        path: std::path::PathBuf,

        /// Promote warnings to errors
        #[arg(long)]
        strict: bool,

        /// Machine-readable JSON output (suppresses human text)
        #[arg(long)]
        json: bool,

        /// Disable ANSI color codes
        #[arg(long)]
        no_color: bool,
    },
}

#[derive(Subcommand)]
pub enum SessionsCommands {
    /// List all active sessions
    List,

    /// Kill one or more sessions
    Kill {
        /// Session IDs to kill (alphanumeric, dash, underscore only; max 128 characters)
        #[arg(num_args = 1.., required_unless_present = "stage", value_parser = clap_id_validator)]
        session_ids: Vec<String>,

        /// Kill all sessions for a stage
        #[arg(long, conflicts_with = "session_ids", value_parser = clap_id_validator)]
        stage: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum WorktreeCommands {
    /// List all worktrees
    List,

    /// Remove a specific worktree and branch after merge conflict resolution
    ///
    /// Use this command after resolving merge conflicts manually or in a resolver session.
    /// It cleans up the worktree and branch WITHOUT attempting another merge.
    Remove {
        /// Stage ID to clean up (alphanumeric, dash, underscore only; max 128 characters)
        #[arg(value_parser = clap_id_validator)]
        stage_id: String,

        /// Allow removal when unmerged work is detected
        #[arg(long)]
        force: bool,

        /// Exact confirmation phrase required with --force
        #[arg(long = "confirm", requires = "force", value_name = "PHRASE")]
        confirmation: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum ContextCommands {
    /// Record files an agent edited, so the stage's context overlay stays current
    RecordEdit {
        /// Stage whose overlay the edit belongs to
        #[arg(long)]
        stage: String,

        /// Edited path; repeat for several
        #[arg(long = "path", required = true)]
        paths: Vec<std::path::PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum HookCommands {
    /// UserPromptSubmit entry point: emit a retrieval brief, or nothing
    UserPrompt,

    /// Internal maintenance entry point invoked by the UserPromptSubmit hook
    /// itself as a fire-and-forget nudge when the source graph looks stale.
    /// Not a user-facing command.
    ReconcileGraph,

    /// Internal maintenance entry point invoked by the PreCompact shell hook
    /// to reopen this session's own delivery suppression before its context
    /// window is compacted away. Not a user-facing command.
    PreCompact,

    /// Print Rust-resolved main/subagent context ceilings for session hooks.
    /// Not a user-facing command.
    #[command(hide = true)]
    ContextCeilings,

    /// Discover package-scoped project types for skill recommendations.
    #[command(hide = true)]
    ProjectTypes,
}
