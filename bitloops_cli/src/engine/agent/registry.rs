use super::Agent;
use super::DEFAULT_AGENT_NAME;
use anyhow::{Result, anyhow};
use std::collections::{HashMap, HashSet};

use crate::engine::agent::claude_code::agent::ClaudeCodeAgent;
use crate::engine::agent::copilot_cli::agent::CopilotCliAgent;
use crate::engine::agent::cursor::agent::CursorAgent;
use crate::engine::agent::gemini_cli::agent::GeminiCliAgent;
use crate::engine::agent::open_code::agent::OpenCodeAgent;

/// Immutable registry of known agents, constructed once at startup.
pub struct AgentRegistry {
    agents: HashMap<String, Box<dyn Agent + Send + Sync>>,
}

impl AgentRegistry {
    /// Build a registry from an arbitrary list of agents.
    pub fn new(agents: Vec<Box<dyn Agent + Send + Sync>>) -> Self {
        let mut map = HashMap::new();
        for agent in agents {
            map.insert(agent.name(), agent);
        }
        Self { agents: map }
    }

    /// Build the default registry containing all built-in agents.
    pub fn builtin() -> Self {
        Self::new(vec![
            Box::new(ClaudeCodeAgent),
            Box::new(CopilotCliAgent),
            Box::new(CursorAgent),
            Box::new(GeminiCliAgent),
            Box::new(OpenCodeAgent),
        ])
    }

    pub fn get(&self, name: &str) -> Result<&(dyn Agent + Send + Sync)> {
        self.agents
            .get(name)
            .map(|a| a.as_ref())
            .ok_or_else(|| anyhow!("unknown agent: {name} (available: {:?})", self.list()))
    }

    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = self.agents.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn detect_all(&self) -> Vec<&(dyn Agent + Send + Sync)> {
        let names = self.list(); // already sorted
        let mut detected = Vec::new();

        for name in &names {
            if let Some(agent) = self.agents.get(name)
                && matches!(agent.detect_presence(), Ok(true))
            {
                detected.push(agent.as_ref());
            }
        }

        detected
    }

    pub fn detect(&self) -> Result<&(dyn Agent + Send + Sync)> {
        let detected = self.detect_all();
        if detected.is_empty() {
            return Err(anyhow!("no agent detected (available: {:?})", self.list()));
        }
        Ok(detected[0])
    }

    pub fn get_by_agent_type(&self, agent_type: &str) -> Result<&(dyn Agent + Send + Sync)> {
        for agent in self.agents.values() {
            if agent.agent_type() == agent_type {
                return Ok(agent.as_ref());
            }
        }
        Err(anyhow!("unknown agent type: {agent_type}"))
    }

    pub fn all_protected_dirs(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut dirs = Vec::new();

        for agent in self.agents.values() {
            for dir in agent.protected_dirs() {
                if seen.insert(dir.clone()) {
                    dirs.push(dir);
                }
            }
        }

        dirs.sort();
        dirs
    }

    pub fn default_agent(&self) -> Option<&(dyn Agent + Send + Sync)> {
        self.get(DEFAULT_AGENT_NAME).ok()
    }
}
