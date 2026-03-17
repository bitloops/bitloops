use std::env;
use std::io::{self, Write};

use anyhow::Result;
use clap::Args;

use crate::commands::enable::find_repo_root;
use crate::engine::settings;

mod agent_hooks;
mod agent_selection;
mod store_backends;
mod telemetry;

pub use agent_selection::detect_or_select_agent;

pub type AgentSelector = dyn Fn(&[String]) -> std::result::Result<Vec<String>, String>;

#[derive(Args)]
pub struct InitArgs {
    /// Remove and reinstall existing hooks for selected agents
    #[arg(long, short = 'f')]
    pub force: bool,

    /// Target a specific agent setup (claude-code|copilot|cursor|gemini-cli|opencode)
    #[arg(long)]
    pub agent: Option<String>,

    /// Enable anonymous usage analytics
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub telemetry: bool,
}

pub async fn run(args: InitArgs) -> Result<()> {
    let mut out = io::stdout().lock();
    run_with_writer(args, &mut out, None)
}

fn run_with_writer(
    args: InitArgs,
    out: &mut dyn Write,
    select_fn: Option<&AgentSelector>,
) -> Result<()> {
    let cwd = env::current_dir()?;
    let repo_root = find_repo_root(&cwd)?;
    store_backends::initialise_store_backends(&repo_root)?;

    let local_dev = settings::load_settings(&repo_root)
        .unwrap_or_default()
        .local_dev;

    let selected_agents = if let Some(agent) = args.agent.as_deref() {
        vec![agent_hooks::normalize_agent_name(agent)?]
    } else {
        detect_or_select_agent(&repo_root, out, select_fn)?
    };

    let mut total_installed = 0usize;
    let mut selected_labels = Vec::new();

    for agent in &selected_agents {
        let (label, count) =
            agent_hooks::install_agent_hooks(&repo_root, agent, local_dev, args.force)?;
        selected_labels.push(label.clone());
        total_installed += count;
        if count > 0 {
            writeln!(out, "Installed {count} {label} hook(s).")?;
        } else {
            writeln!(out, "{label} hooks are already initialized.")?;
        }
    }

    telemetry::maybe_capture_telemetry_consent(
        &repo_root,
        args.telemetry,
        args.agent.is_none(),
        out,
    )?;

    writeln!(out)?;
    writeln!(out, "Initialized agents: {}", selected_labels.join(", "))?;
    writeln!(out, "Total hooks installed: {total_installed}")?;
    writeln!(out, "Bitloops agent initialization complete.")?;
    Ok(())
}

#[cfg(test)]
mod tests;
