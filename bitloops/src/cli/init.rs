use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::Args;

mod agent_hooks;
mod agent_selection;
#[cfg(test)]
mod telemetry;
use crate::adapters::agents::AgentAdapterRegistry;
use crate::adapters::agents::claude_code::git_hooks;
use crate::config::settings::{DEFAULT_STRATEGY, load_settings, write_project_bootstrap_settings};
use crate::config::{REPO_POLICY_LOCAL_FILE_NAME, default_daemon_config_path};
use crate::devql_transport::discover_slim_cli_repo_scope;

pub use agent_selection::detect_or_select_agent;

pub type AgentSelector = dyn Fn(&[String]) -> std::result::Result<Vec<String>, String>;

#[derive(Args)]
pub struct InitArgs {
    /// Remove and reinstall existing hooks for selected agents.
    #[arg(long, short = 'f')]
    pub force: bool,

    /// Target a specific agent setup (claude-code|copilot|cursor|gemini|opencode).
    #[arg(long)]
    pub agent: Option<String>,

    /// Enable anonymous usage analytics
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub telemetry: bool,

    /// Skip the initial baseline sync into DevQL current state.
    #[arg(long, default_value_t = false)]
    pub skip_baseline: bool,
}

pub async fn run(args: InitArgs) -> Result<()> {
    let mut out = io::stdout().lock();
    run_with_writer_async(args, &mut out, None).await
}

#[cfg(test)]
fn run_with_writer(
    args: InitArgs,
    out: &mut dyn Write,
    select_fn: Option<&AgentSelector>,
) -> Result<()> {
    let runtime = tokio::runtime::Runtime::new().context("creating runtime for `bitloops init`")?;
    runtime.block_on(run_with_writer_async(args, out, select_fn))
}

async fn run_with_writer_async(
    args: InitArgs,
    out: &mut dyn Write,
    select_fn: Option<&AgentSelector>,
) -> Result<()> {
    let project_root = std::env::current_dir().context("getting current directory")?;
    let git_root = crate::cli::enable::find_repo_root(&project_root)?;

    ensure_daemon_running().await?;
    ensure_repo_local_policy_excluded(&git_root, &project_root)?;

    let selected_agents = if let Some(agent) = args.agent.as_deref() {
        vec![AgentAdapterRegistry::builtin().normalise_agent_name(agent)?]
    } else {
        detect_or_select_agent(&project_root, out, select_fn)?
    };
    let strategy = load_settings(&project_root)
        .map(|settings| settings.strategy)
        .unwrap_or_else(|_| DEFAULT_STRATEGY.to_string());
    let local_policy_path = project_root.join(REPO_POLICY_LOCAL_FILE_NAME);
    write_project_bootstrap_settings(&local_policy_path, &strategy, &selected_agents)?;

    let settings = load_settings(&project_root).unwrap_or_default();
    let git_count = git_hooks::install_git_hooks(&git_root, settings.local_dev)?;
    if git_count > 0 {
        writeln!(out, "Installed {git_count} git hook(s).")?;
    }

    reconcile_agent_hooks(
        &project_root,
        &selected_agents,
        settings.local_dev,
        args.force,
        out,
    )?;

    let scope = discover_slim_cli_repo_scope(Some(&project_root))?;
    crate::cli::devql::graphql::run_project_bootstrap_via_graphql(&scope, args.skip_baseline)
        .await?;

    writeln!(out)?;
    writeln!(out, "Project config: {}", local_policy_path.display())?;
    writeln!(out, "Initialised agents: {}", selected_agents.join(", "))?;
    writeln!(out, "Bitloops project bootstrap is ready.")?;
    Ok(())
}

async fn ensure_daemon_running() -> Result<()> {
    #[cfg(test)]
    if std::env::var("BITLOOPS_TEST_ASSUME_DAEMON_RUNNING")
        .ok()
        .is_some_and(|value| !value.trim().is_empty() && value.trim() != "0")
    {
        return Ok(());
    }

    let status = crate::daemon::status().await?;
    if status.runtime.is_some() {
        return Ok(());
    }

    let config_path = default_daemon_config_path()?;
    bail!(
        "Bitloops daemon is not running. Start it with `bitloops start`{}.",
        if config_path.exists() {
            String::new()
        } else {
            format!(
                " to auto-create the default daemon config at {}",
                config_path.display()
            )
        }
    )
}

fn ensure_repo_local_policy_excluded(git_root: &Path, project_root: &Path) -> Result<()> {
    let exclude_path = git_root.join(".git").join("info").join("exclude");
    if let Some(parent) = exclude_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating git exclude directory {}", parent.display()))?;
    }

    let mut content = std::fs::read_to_string(&exclude_path).unwrap_or_default();
    let relative_local_policy = project_root
        .strip_prefix(git_root)
        .unwrap_or(project_root)
        .join(REPO_POLICY_LOCAL_FILE_NAME);
    let relative_local_policy = relative_local_policy.to_string_lossy().replace('\\', "/");

    for entry in [relative_local_policy.as_str(), ".bitloops/"] {
        if content.lines().any(|line| line.trim() == entry) {
            continue;
        }
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(entry);
        content.push('\n');
    }

    std::fs::write(&exclude_path, content)
        .with_context(|| format!("writing {}", exclude_path.display()))?;
    Ok(())
}

fn reconcile_agent_hooks(
    project_root: &Path,
    selected_agents: &[String],
    local_dev: bool,
    force: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let registry = AgentAdapterRegistry::builtin();
    let selected = selected_agents
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();

    for agent in registry.installed_agents(project_root) {
        if selected.contains(&agent) {
            continue;
        }
        let label = registry.uninstall_agent_hooks(project_root, &agent)?;
        writeln!(out, "Removed {label} hooks.")?;
    }

    for agent in selected_agents {
        let (label, installed) =
            registry.install_agent_hooks(project_root, agent, local_dev, force)?;
        if installed > 0 {
            writeln!(out, "Installed {installed} {label} hook(s).")?;
        } else {
            writeln!(out, "{label} hooks are already initialised.")?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;
