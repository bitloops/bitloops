use std::io::{self, Write};
use std::path::Path;

use anyhow::Result;
use clap::Args;

mod agent_hooks;
mod agent_selection;
mod telemetry;
use crate::config::ensure_daemon_config_exists;

pub use agent_selection::detect_or_select_agent;

pub type AgentSelector = dyn Fn(&[String]) -> std::result::Result<Vec<String>, String>;

#[derive(Args)]
pub struct InitArgs {
    /// Deprecated: hook installation moved to `bitloops enable`
    #[arg(long, short = 'f')]
    pub force: bool,

    /// Deprecated: agent setup moved to `bitloops enable`
    #[arg(long)]
    pub agent: Option<String>,

    /// Enable anonymous usage analytics
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub telemetry: bool,

    /// Deprecated: daemon initialisation no longer performs repo ingestion.
    #[arg(long, default_value_t = false)]
    pub skip_baseline: bool,
}

pub async fn run(args: InitArgs) -> Result<()> {
    let mut out = io::stdout().lock();
    run_with_writer(args, &mut out, None)
}

fn run_with_writer(
    args: InitArgs,
    out: &mut dyn Write,
    _select_fn: Option<&AgentSelector>,
) -> Result<()> {
    let config_path = ensure_daemon_config_exists()?;
    telemetry::maybe_capture_telemetry_consent(Path::new("."), args.telemetry, true, out)?;

    if args.force || args.agent.is_some() || args.skip_baseline {
        writeln!(
            out,
            "Note: `bitloops init` now configures the global daemon only. Use `bitloops enable` for hooks."
        )?;
    }

    writeln!(out)?;
    writeln!(out, "Daemon config: {}", config_path.display())?;
    writeln!(out, "Bitloops daemon configuration is ready.")?;
    Ok(())
}

#[cfg(test)]
mod tests;
