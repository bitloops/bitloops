use std::path::Path;

use anyhow::Result;

use crate::engine::agent::{AGENT_NAME_CLAUDE_CODE, AgentAdapterRegistry};
#[cfg(test)]
use crate::engine::agent::{AGENT_NAME_CODEX, AGENT_NAME_CURSOR, AGENT_TYPE_GEMINI};

pub(super) const AGENT_CLAUDE_CODE: &str = AGENT_NAME_CLAUDE_CODE;
#[cfg(test)]
pub(super) const AGENT_CODEX: &str = AGENT_NAME_CODEX;
#[cfg(test)]
pub(super) const AGENT_CURSOR: &str = AGENT_NAME_CURSOR;
#[cfg(test)]
pub(super) const AGENT_GEMINI_CLI: &str = AGENT_TYPE_GEMINI;
pub(super) const DEFAULT_AGENT: &str = AGENT_CLAUDE_CODE;

pub(super) fn install_agent_hooks(
    repo_root: &Path,
    agent_name: &str,
    local_dev: bool,
    force: bool,
) -> Result<(String, usize)> {
    let (label, installed) = AgentAdapterRegistry::builtin()
        .install_agent_hooks(repo_root, agent_name, local_dev, force)?;
    Ok((label.to_string(), installed))
}

pub(super) fn normalize_agent_name(value: &str) -> Result<String> {
    AgentAdapterRegistry::builtin().normalise_agent_name(value)
}

pub(super) fn detect_agents(repo_root: &Path) -> Vec<String> {
    AgentAdapterRegistry::builtin().detect_project_agents(repo_root)
}

pub(super) fn available_agents() -> Vec<String> {
    AgentAdapterRegistry::builtin().available_agents()
}

pub(super) fn agent_display(agent: &str) -> &'static str {
    AgentAdapterRegistry::builtin()
        .agent_display(agent)
        .unwrap_or("Unknown")
}
