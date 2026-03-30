use std::path::Path;

use crate::adapters::agents::{AGENT_NAME_CLAUDE_CODE, AgentAdapterRegistry};
#[cfg(test)]
use crate::adapters::agents::{AGENT_NAME_CODEX, AGENT_NAME_CURSOR, AGENT_TYPE_GEMINI};

pub(super) const AGENT_CLAUDE_CODE: &str = AGENT_NAME_CLAUDE_CODE;
#[cfg(test)]
pub(super) const AGENT_CODEX: &str = AGENT_NAME_CODEX;
#[cfg(test)]
pub(super) const AGENT_CURSOR: &str = AGENT_NAME_CURSOR;
#[cfg(test)]
pub(super) const AGENT_GEMINI: &str = AGENT_TYPE_GEMINI;
pub(super) const DEFAULT_AGENT: &str = AGENT_CLAUDE_CODE;

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
