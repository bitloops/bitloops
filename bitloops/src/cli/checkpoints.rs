use anyhow::{Result, bail};
use clap::{Args, Subcommand};

#[path = "status.rs"]
mod status_impl;

pub use status_impl::StatusArgs;

pub const MISSING_SUBCOMMAND_MESSAGE: &str =
    "missing subcommand. Use `bitloops checkpoints status`";

#[derive(Args, Debug, Clone, Default)]
pub struct CheckpointsArgs {
    #[command(subcommand)]
    pub command: Option<CheckpointsCommand>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum CheckpointsCommand {
    /// Show repository/session checkpoint status.
    Status(StatusArgs),
}

pub async fn run(args: CheckpointsArgs) -> Result<()> {
    let Some(command) = args.command else {
        bail!(MISSING_SUBCOMMAND_MESSAGE);
    };

    match command {
        CheckpointsCommand::Status(args) => status_impl::run(args).await,
    }
}
