use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;

use anyhow::{Result, bail};

use crate::adapters::agents::AgentAdapterRegistry;

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
    let registry = AgentAdapterRegistry::builtin();
    let selected = selected_agents.iter().cloned().collect::<BTreeSet<_>>();

    for agent in registry.installed_agents(project_root) {
        if selected.contains(&agent) {
            continue;
        }
        let label = registry.uninstall_agent_hooks(project_root, &agent)?;
        writeln!(out, "Ensured {label} hooks and prompt surfaces are removed.")?;
    }

    for agent in selected_agents {
        let (label, installed) =
            registry.install_agent_hooks(project_root, agent, local_dev, force)?;
        if installed > 0 {
            writeln!(
                out,
                "Installed {installed} {label} hooks and prompt surfaces."
            )?;
        } else {
            writeln!(
                out,
                "{label} hooks and prompt surfaces are already initialised."
            )?;
        }
    }

    Ok(())
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
        .chain(registry.installed_agents(project_root).into_iter())
    {
        if seen.insert(agent.clone()) {
            candidates.push(agent);
        }
    }

    for agent in &candidates {
        let label = registry.uninstall_agent_hooks(project_root, agent)?;
        writeln!(out, "Ensured {label} hooks and prompt surfaces are removed.")?;
    }

    Ok(candidates.len())
}
