use std::path::Path;

use crate::adapters::agents::{AGENT_NAME_CLAUDE_CODE, AgentAdapterRegistry};

pub(crate) const AGENT_CLAUDE_CODE: &str = AGENT_NAME_CLAUDE_CODE;
pub(crate) const DEFAULT_AGENT: &str = AGENT_CLAUDE_CODE;

pub(crate) fn detect_agents(repo_root: &Path) -> Vec<String> {
    AgentAdapterRegistry::builtin().detect_project_agents(repo_root)
}

pub(crate) fn available_agents() -> Vec<String> {
    AgentAdapterRegistry::builtin().available_agents()
}

pub(crate) fn agent_display(agent: &str) -> &'static str {
    AgentAdapterRegistry::builtin()
        .agent_display(agent)
        .unwrap_or("Unknown")
}
