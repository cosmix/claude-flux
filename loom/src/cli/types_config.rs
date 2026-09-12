//! `loom config` CLI args: read or write a key in `~/.loom/config.toml`.
//!
//! Split out from `types.rs` for the same reason as `types_ops.rs` /
//! `types_stage.rs` / `types_memory.rs`: the top-level `Commands` enum sits at
//! its line budget, so a new subcommand's flags live in their own file.
//! `Commands::Config` wraps [`ConfigArgs`] as a tuple variant
//! (`Config(ConfigArgs)`) rather than following `Map`/`Usage`'s
//! struct-with-`#[command(flatten)]` form — clap 4 supports both identically,
//! and the tuple form costs three fewer lines here.

/// Flags for `loom config`.
#[derive(Debug, clap::Args)]
pub struct ConfigArgs {
    /// Config key to read, or to write when a value follows (e.g. update.check_interval_hours)
    #[arg(short = 'k', long = "key")]
    pub key: Option<String>,

    /// New value for --key; omit to print the key's current value
    #[arg(requires = "key")]
    pub value: Option<String>,

    /// List every key with its value and origin (~/.loom/config.toml alone;
    /// a workspace .loom/work/config.toml, when present, overrides it: whole
    /// `[terminal]`/`[context]` sections, `[pressure]`/`[models]` key by key)
    #[arg(long, conflicts_with_all = ["key", "value"])]
    pub list: bool,

    /// Print the resolved user config as TOML
    #[arg(long, conflicts_with_all = ["key", "value", "list"])]
    pub print: bool,
}
