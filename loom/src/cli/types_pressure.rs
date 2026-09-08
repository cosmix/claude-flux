//! `loom pressure` CLI args: pressure-test a plan with alternating Claude and
//! Codex review rounds.
//!
//! Split out from `types.rs` for the same reason as `types_ops.rs` /
//! `types_stage.rs` / `types_memory.rs`: the top-level `Commands` enum sits at
//! its line budget, so a new subcommand's flags live in their own file.
//! `Commands::Pressure` wraps [`PressureArgs`] as a tuple variant
//! (`Pressure(PressureArgs)`), the same shape `Commands::Config` uses for
//! [`super::types_config::ConfigArgs`].

/// Flags for `loom pressure`.
#[derive(Debug, clap::Args)]
pub struct PressureArgs {
    /// Path to the plan file (repo-relative or absolute; a bare filename
    /// resolves under doc/plans/)
    pub plan: String,

    /// Number of pressure/address rounds to run (must be >= 1)
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u32).range(1..))]
    pub rounds: u32,

    /// Print the planned steps without spawning Claude or Codex
    #[arg(long)]
    pub dry_run: bool,

    /// Claude model for the /pressure step (default: opus, or pressure.claude_model)
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(crate::claude::CLAUDE_MODELS))]
    pub claude_model: Option<String>,

    /// Codex model for the $pressure step (default: gpt-5.6-sol, or pressure.codex_model)
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(crate::codex::CODEX_MODELS))]
    pub codex_model: Option<String>,

    /// Claude model for the /address step (default: opus, or pressure.address_model)
    #[arg(long, value_parser = clap::builder::PossibleValuesParser::new(crate::claude::CLAUDE_MODELS))]
    pub address_model: Option<String>,
}
