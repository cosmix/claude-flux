use clap::Args;

/// Dashboard-only options kept separate to preserve the main CLI type's size budget.
#[derive(Args)]
pub struct StatusWebArgs {
    /// Serve the live dashboard on 127.0.0.1 (omit PORT to find a port from 7373; 0 picks a free port)
    #[arg(
        long,
        value_name = "PORT",
        num_args = 0..=1,
        conflicts_with_all = ["live", "compact", "verbose"]
    )]
    pub web: Option<Option<u16>>,
}
