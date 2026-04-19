use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;

use anyhow::{Result, bail};

use crate::adapters::agents::{
    AGENT_NAME_CLAUDE_CODE, AGENT_NAME_CODEX, AGENT_NAME_COPILOT, AGENT_NAME_CURSOR,
    AGENT_NAME_GEMINI, AGENT_NAME_OPEN_CODE, AgentAdapterRegistry, AgentHookInstallOptions,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReconcileProjectAgentSurfacesOptions {
    pub install_bitloops_skill: bool,
}

impl Default for ReconcileProjectAgentSurfacesOptions {
    fn default() -> Self {
        Self {
            install_bitloops_skill: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentIntegrationState {
    Installed,
    AlreadyInstalled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentIntegrationReport {
    pub agent: String,
    pub label: &'static str,
    pub hook_count: usize,
    pub newly_installed_hook_count: usize,
    pub state: AgentIntegrationState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentSurfaceReconcileReport {
    pub integrations: Vec<AgentIntegrationReport>,
}

pub(crate) fn configured_agents_or_bail(start: &Path) -> Result<Vec<String>> {
    let agents = crate::config::settings::supported_agents(start)?;
    if agents.is_empty() {
        bail!(
            "No supported agents are configured for this Bitloops project. Run `bitloops init` to select agents before enabling Bitloops."
        );
    }
    Ok(agents)
}

pub(crate) fn reconcile_project_agent_surfaces(
    project_root: &Path,
    selected_agents: &[String],
    local_dev: bool,
    force: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let report = reconcile_project_agent_surfaces_with_options(
        project_root,
        selected_agents,
        local_dev,
        force,
        ReconcileProjectAgentSurfacesOptions::default(),
        out,
    )?;

    for integration in report.integrations {
        if integration.state == AgentIntegrationState::Installed {
            writeln!(
                out,
                "Installed {} {} hooks and prompt surfaces.",
                integration.newly_installed_hook_count, integration.label
            )?;
        } else {
            writeln!(
                out,
                "{} hooks and prompt surfaces are already initialised.",
                integration.label
            )?;
        }
    }

    Ok(())
}

pub(crate) fn reconcile_project_agent_surfaces_with_options(
    project_root: &Path,
    selected_agents: &[String],
    local_dev: bool,
    force: bool,
    options: ReconcileProjectAgentSurfacesOptions,
    out: &mut dyn Write,
) -> Result<AgentSurfaceReconcileReport> {
    let registry = AgentAdapterRegistry::builtin();
    let selected = selected_agents.iter().cloned().collect::<BTreeSet<_>>();
    let mut integrations = Vec::new();

    for agent in registry.installed_agents(project_root) {
        if selected.contains(&agent) {
            continue;
        }
        let label = registry.uninstall_agent_hooks(project_root, &agent)?;
        writeln!(
            out,
            "Ensured {label} hooks and prompt surfaces are removed."
        )?;
    }

    for agent in selected_agents {
        let hooks_already_installed = registry.are_agent_hooks_installed(project_root, agent)?;
        let (label, installed) = registry.install_agent_hooks(
            project_root,
            agent,
            local_dev,
            force,
            AgentHookInstallOptions {
                install_bitloops_skill: options.install_bitloops_skill,
            },
        )?;
        integrations.push(AgentIntegrationReport {
            agent: agent.clone(),
            label,
            hook_count: managed_hook_count(agent),
            newly_installed_hook_count: installed,
            state: if installed == 0 && hooks_already_installed {
                AgentIntegrationState::AlreadyInstalled
            } else {
                AgentIntegrationState::Installed
            },
        });
    }

    Ok(AgentSurfaceReconcileReport { integrations })
}

/// Attempts cleanup for configured and already-detected agents and returns the
/// number of agent families considered for cleanup.
///
/// This is not an exact "files removed" count; adapter uninstall routines are
/// currently best-effort and do not report no-op vs changed.
pub(crate) fn cleanup_project_agent_surfaces(
    project_root: &Path,
    configured_agents: &[String],
    out: &mut dyn Write,
) -> Result<usize> {
    let registry = AgentAdapterRegistry::builtin();
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();

    for agent in configured_agents
        .iter()
        .cloned()
        .chain(registry.installed_agents(project_root))
    {
        if seen.insert(agent.clone()) {
            candidates.push(agent);
        }
    }

    for agent in &candidates {
        let label = registry.uninstall_agent_hooks(project_root, agent)?;
        writeln!(
            out,
            "Ensured {label} hooks and prompt surfaces are removed."
        )?;
    }

    Ok(candidates.len())
}

fn managed_hook_count(agent: &str) -> usize {
    match agent {
        AGENT_NAME_CLAUDE_CODE => 7,
        AGENT_NAME_COPILOT => 8,
        AGENT_NAME_CODEX => 5,
        AGENT_NAME_CURSOR => 9,
        AGENT_NAME_GEMINI => 12,
        AGENT_NAME_OPEN_CODE => 5,
        _ => 0,
    }
}
