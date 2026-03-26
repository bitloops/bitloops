use std::env;
use std::io::{self, Write};
use std::path::Path;

use anyhow::Result;
use clap::Args;

use crate::adapters::agents::{AgentAdapterRegistry, AgentReadinessStatus};
use crate::cli::enable::find_repo_root;
use crate::config::settings;
use crate::host::devql::{DevqlConfig, resolve_repo_identity, run_init_for_bitloops};

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

    /// Target a specific agent setup (claude-code|copilot|cursor|gemini|opencode)
    #[arg(long)]
    pub agent: Option<String>,

    /// Enable anonymous usage analytics
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub telemetry: bool,

    /// Skip initial baseline ingestion of tracked source files.
    #[arg(long, default_value_t = false)]
    pub skip_baseline: bool,
}

pub async fn run(args: InitArgs) -> Result<()> {
    let cwd = env::current_dir()?;
    let repo_root = find_repo_root(&cwd)?;
    store_backends::initialise_store_backends(&repo_root)?;

    let repo = resolve_repo_identity(&repo_root)?;
    let cfg = DevqlConfig::from_env(repo_root.clone(), repo)?;
    run_init_for_bitloops(&cfg, args.skip_baseline).await?;

    let mut out = io::stdout().lock();
    run_with_writer_for_repo(args, &repo_root, &mut out, None, true)
}

#[cfg(test)]
fn run_with_writer(
    args: InitArgs,
    out: &mut dyn Write,
    select_fn: Option<&AgentSelector>,
) -> Result<()> {
    let cwd = env::current_dir()?;
    let repo_root = find_repo_root(&cwd)?;
    run_with_writer_for_repo(args, &repo_root, out, select_fn, false)
}

fn run_with_writer_for_repo(
    args: InitArgs,
    repo_root: &Path,
    out: &mut dyn Write,
    select_fn: Option<&AgentSelector>,
    stores_initialised: bool,
) -> Result<()> {
    if !stores_initialised {
        store_backends::initialise_store_backends(repo_root)?;
    }

    let local_dev = settings::load_settings(repo_root)
        .unwrap_or_default()
        .local_dev;

    let selected_agents = if let Some(agent) = args.agent.as_deref() {
        vec![agent_hooks::normalize_agent_name(agent)?]
    } else {
        detect_or_select_agent(repo_root, out, select_fn)?
    };

    let mut total_installed = 0usize;
    let mut selected_labels = Vec::new();

    for agent in &selected_agents {
        let (label, count) =
            agent_hooks::install_agent_hooks(repo_root, agent, local_dev, args.force)?;
        selected_labels.push(label.clone());
        total_installed += count;
        if count > 0 {
            writeln!(out, "Installed {count} {label} hook(s).")?;
        } else {
            writeln!(out, "{label} hooks are already initialized.")?;
        }
    }

    let readiness = AgentAdapterRegistry::builtin().collect_readiness(repo_root);
    for selected in &selected_agents {
        if let Some(report) = readiness.iter().find(|entry| entry.id == *selected)
            && report.status == AgentReadinessStatus::NotReady
        {
            writeln!(out, "Readiness: {} is not ready.", report.display_name)?;
            for failure in &report.failures {
                writeln!(out, "  - {}: {}", failure.code, failure.message)?;
            }
            writeln!(out)?;
        }
    }

    telemetry::maybe_capture_telemetry_consent(
        repo_root,
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
