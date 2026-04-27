use super::Agent;
use super::adapters::AgentAdapterRegistry;
use anyhow::{Result, anyhow};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

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
    pub fn builtin() -> &'static Self {
        static BUILTIN: OnceLock<AgentRegistry> = OnceLock::new();
        BUILTIN
            .get_or_init(|| AgentRegistry::new(AgentAdapterRegistry::builtin().create_all_agents()))
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
        self.get(AgentAdapterRegistry::builtin().default_agent_name())
            .ok()
    }
}
