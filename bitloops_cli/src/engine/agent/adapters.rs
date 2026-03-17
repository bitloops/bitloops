use super::{
    AGENT_NAME_CLAUDE_CODE, AGENT_NAME_CODEX, AGENT_NAME_COPILOT, AGENT_NAME_CURSOR,
    AGENT_NAME_GEMINI, AGENT_NAME_OPEN_CODE, AGENT_TYPE_CLAUDE_CODE, AGENT_TYPE_CODEX,
    AGENT_TYPE_COPILOT, AGENT_TYPE_CURSOR, AGENT_TYPE_GEMINI, AGENT_TYPE_OPEN_CODE, Agent,
    HookSupport,
};
use anyhow::{Result, anyhow, bail};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

use crate::engine::agent::claude_code::agent::ClaudeCodeAgent;
use crate::engine::agent::claude_code::hooks as claude_hooks;
use crate::engine::agent::codex::agent::CodexAgent;
use crate::engine::agent::codex::hooks as codex_hooks;
use crate::engine::agent::copilot_cli::agent::CopilotCliAgent;
use crate::engine::agent::cursor::agent::CursorAgent;
use crate::engine::agent::gemini_cli::agent::GeminiCliAgent;
use crate::engine::agent::open_code::agent::OpenCodeAgent;

pub const HOST_ADAPTER_CONTRACT_VERSION: u16 = 1;
pub const HOST_ADAPTER_RUNTIME_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentAdapterCapability {
    PresenceDetection,
    ProjectDetection,
    HookInstallation,
    SessionIo,
    TranscriptIo,
    TranscriptAnalysis,
    TokenCalculation,
    LifecycleRouting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentAdapterCompatibility {
    pub contract_version: u16,
    pub min_host_version: u16,
    pub max_host_version: u16,
}

impl AgentAdapterCompatibility {
    pub fn phase1() -> Self {
        Self {
            contract_version: HOST_ADAPTER_CONTRACT_VERSION,
            min_host_version: HOST_ADAPTER_RUNTIME_VERSION,
            max_host_version: HOST_ADAPTER_RUNTIME_VERSION,
        }
    }

    fn validate(&self, id: &str) -> Result<()> {
        if self.contract_version != HOST_ADAPTER_CONTRACT_VERSION {
            bail!(
                "adapter {id} has unsupported contract version {} (expected {})",
                self.contract_version,
                HOST_ADAPTER_CONTRACT_VERSION
            );
        }
        if HOST_ADAPTER_RUNTIME_VERSION < self.min_host_version
            || HOST_ADAPTER_RUNTIME_VERSION > self.max_host_version
        {
            bail!(
                "adapter {id} is incompatible with host runtime version {} (supported {}-{})",
                HOST_ADAPTER_RUNTIME_VERSION,
                self.min_host_version,
                self.max_host_version
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct AgentAdapterDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub agent_type: &'static str,
    pub aliases: &'static [&'static str],
    pub is_default: bool,
    pub capabilities: &'static [AgentAdapterCapability],
    pub compatibility: AgentAdapterCompatibility,
}

pub struct AgentAdapterRegistration {
    descriptor: AgentAdapterDescriptor,
    create_agent: fn() -> Box<dyn Agent + Send + Sync>,
    detect_project_presence: fn(&Path) -> bool,
    hooks_installed: fn(&Path) -> bool,
    install_hooks: fn(&Path, bool, bool) -> Result<usize>,
    uninstall_hooks: fn(&Path) -> Result<()>,
    format_resume_command: fn(&str) -> String,
}

impl AgentAdapterRegistration {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        descriptor: AgentAdapterDescriptor,
        create_agent: fn() -> Box<dyn Agent + Send + Sync>,
        detect_project_presence: fn(&Path) -> bool,
        hooks_installed: fn(&Path) -> bool,
        install_hooks: fn(&Path, bool, bool) -> Result<usize>,
        uninstall_hooks: fn(&Path) -> Result<()>,
        format_resume_command: fn(&str) -> String,
    ) -> Self {
        Self {
            descriptor,
            create_agent,
            detect_project_presence,
            hooks_installed,
            install_hooks,
            uninstall_hooks,
            format_resume_command,
        }
    }

    pub fn descriptor(&self) -> &AgentAdapterDescriptor {
        &self.descriptor
    }

    pub fn create_agent(&self) -> Box<dyn Agent + Send + Sync> {
        (self.create_agent)()
    }

    pub fn is_project_detected(&self, repo_root: &Path) -> bool {
        (self.detect_project_presence)(repo_root)
    }

    pub fn are_hooks_installed(&self, repo_root: &Path) -> bool {
        (self.hooks_installed)(repo_root)
    }

    pub fn install_hooks(&self, repo_root: &Path, local_dev: bool, force: bool) -> Result<usize> {
        (self.install_hooks)(repo_root, local_dev, force)
    }

    pub fn uninstall_hooks(&self, repo_root: &Path) -> Result<()> {
        (self.uninstall_hooks)(repo_root)
    }

    pub fn format_resume_command(&self, session_id: &str) -> String {
        (self.format_resume_command)(session_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAdapterReadiness {
    pub id: String,
    pub display_name: String,
    pub project_detected: bool,
    pub hooks_installed: bool,
    pub compatibility_ok: bool,
}

pub struct AgentAdapterRegistry {
    registrations: HashMap<String, AgentAdapterRegistration>,
    aliases: HashMap<String, String>,
    ordered_ids: Vec<String>,
    default_id: String,
}

impl AgentAdapterRegistry {
    pub fn new(registrations: Vec<AgentAdapterRegistration>) -> Result<Self> {
        if registrations.is_empty() {
            bail!("at least one adapter registration is required");
        }

        let mut map: HashMap<String, AgentAdapterRegistration> = HashMap::new();
        let mut aliases: HashMap<String, String> = HashMap::new();
        let mut ordered_ids = Vec::new();
        let mut default_id: Option<String> = None;
        let mut used_agent_types: HashSet<String> = HashSet::new();

        for registration in registrations {
            let id = normalise_key(registration.descriptor.id)?;
            registration.descriptor.compatibility.validate(&id)?;

            let agent_type = normalise_key(registration.descriptor.agent_type)?;
            if !used_agent_types.insert(agent_type.clone()) {
                bail!("duplicate adapter agent type: {agent_type}");
            }

            if map.contains_key(&id) {
                bail!("duplicate adapter id: {id}");
            }

            if registration.descriptor.is_default {
                if default_id.is_some() {
                    bail!("multiple default adapters configured");
                }
                default_id = Some(id.clone());
            }

            register_alias(&mut aliases, &id, registration.descriptor.id)?;
            for alias in registration.descriptor.aliases {
                register_alias(&mut aliases, &id, alias)?;
            }

            ordered_ids.push(id.clone());
            map.insert(id, registration);
        }

        let Some(default_id) = default_id else {
            bail!("no default adapter configured");
        };

        Ok(Self {
            registrations: map,
            aliases,
            ordered_ids,
            default_id,
        })
    }

    pub fn builtin() -> &'static Self {
        static BUILTIN: OnceLock<AgentAdapterRegistry> = OnceLock::new();
        BUILTIN.get_or_init(|| {
            AgentAdapterRegistry::new(builtin_registrations())
                .expect("builtin adapter registrations must be valid")
        })
    }

    pub fn available_agents(&self) -> Vec<String> {
        self.ordered_ids
            .iter()
            .map(|id| {
                self.registrations
                    .get(id)
                    .expect("adapter id missing from registry")
                    .descriptor
                    .id
                    .to_string()
            })
            .collect()
    }

    pub fn default_agent_name(&self) -> &str {
        self.registrations
            .get(&self.default_id)
            .expect("default adapter id missing")
            .descriptor
            .id
    }

    pub fn normalise_agent_name(&self, value: &str) -> Result<String> {
        let key = normalise_key(value)?;
        let id = self
            .aliases
            .get(&key)
            .ok_or_else(|| anyhow!("unknown agent name: {}", value.trim()))?;
        Ok(self
            .registrations
            .get(id)
            .expect("resolved adapter id missing")
            .descriptor
            .id
            .to_string())
    }

    pub fn resolve(&self, value: &str) -> Result<&AgentAdapterRegistration> {
        let key = normalise_key(value)?;
        let id = self
            .aliases
            .get(&key)
            .ok_or_else(|| anyhow!("unknown agent name: {}", value.trim()))?;
        self.registrations
            .get(id)
            .ok_or_else(|| anyhow!("adapter registration missing: {id}"))
    }

    pub fn agent_display(&self, value: &str) -> Option<&'static str> {
        self.resolve(value)
            .ok()
            .map(|registration| registration.descriptor.display_name)
    }

    pub fn detect_project_agents(&self, repo_root: &Path) -> Vec<String> {
        self.ordered_ids
            .iter()
            .filter_map(|id| {
                let registration = self.registrations.get(id)?;
                if registration.is_project_detected(repo_root) {
                    Some(registration.descriptor.id.to_string())
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn installed_agents(&self, repo_root: &Path) -> Vec<String> {
        self.ordered_ids
            .iter()
            .filter_map(|id| {
                let registration = self.registrations.get(id)?;
                if registration.are_hooks_installed(repo_root) {
                    Some(registration.descriptor.id.to_string())
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn install_agent_hooks(
        &self,
        repo_root: &Path,
        value: &str,
        local_dev: bool,
        force: bool,
    ) -> Result<(&'static str, usize)> {
        let registration = self.resolve(value)?;
        let count = registration.install_hooks(repo_root, local_dev, force)?;
        Ok((registration.descriptor.display_name, count))
    }

    pub fn uninstall_agent_hooks(&self, repo_root: &Path, value: &str) -> Result<&'static str> {
        let registration = self.resolve(value)?;
        registration.uninstall_hooks(repo_root)?;
        Ok(registration.descriptor.display_name)
    }

    pub fn are_agent_hooks_installed(&self, repo_root: &Path, value: &str) -> Result<bool> {
        let registration = self.resolve(value)?;
        Ok(registration.are_hooks_installed(repo_root))
    }

    pub fn format_resume_command(&self, value: &str, session_id: &str) -> Result<String> {
        let registration = self.resolve(value)?;
        Ok(registration.format_resume_command(session_id))
    }

    pub fn create_all_agents(&self) -> Vec<Box<dyn Agent + Send + Sync>> {
        self.ordered_ids
            .iter()
            .map(|id| {
                self.registrations
                    .get(id)
                    .expect("adapter id missing from registry")
                    .create_agent()
            })
            .collect()
    }

    pub fn all_protected_dirs(&self) -> Vec<String> {
        let mut dirs = Vec::new();
        let mut seen = HashSet::new();

        for agent in self.create_all_agents() {
            for dir in agent.protected_dirs() {
                if seen.insert(dir.clone()) {
                    dirs.push(dir);
                }
            }
        }

        dirs.sort();
        dirs
    }

    pub fn collect_readiness(&self, repo_root: &Path) -> Vec<AgentAdapterReadiness> {
        self.ordered_ids
            .iter()
            .map(|id| {
                let registration = self
                    .registrations
                    .get(id)
                    .expect("adapter id missing from registry");
                AgentAdapterReadiness {
                    id: registration.descriptor.id.to_string(),
                    display_name: registration.descriptor.display_name.to_string(),
                    project_detected: registration.is_project_detected(repo_root),
                    hooks_installed: registration.are_hooks_installed(repo_root),
                    compatibility_ok: registration
                        .descriptor
                        .compatibility
                        .validate(registration.descriptor.id)
                        .is_ok(),
                }
            })
            .collect()
    }
}

fn normalise_key(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("missing agent name");
    }
    Ok(trimmed.to_ascii_lowercase())
}

fn register_alias(aliases: &mut HashMap<String, String>, id: &str, alias: &str) -> Result<()> {
    let alias_key = normalise_key(alias)?;
    if let Some(existing) = aliases.get(&alias_key)
        && existing != id
    {
        bail!("alias collision for {alias_key}: {existing} vs {id}");
    }
    aliases.insert(alias_key, id.to_string());
    Ok(())
}

const BASE_CAPABILITIES: &[AgentAdapterCapability] = &[
    AgentAdapterCapability::PresenceDetection,
    AgentAdapterCapability::ProjectDetection,
    AgentAdapterCapability::HookInstallation,
    AgentAdapterCapability::SessionIo,
    AgentAdapterCapability::TranscriptIo,
];

const ANALYTICS_CAPABILITIES: &[AgentAdapterCapability] = &[
    AgentAdapterCapability::PresenceDetection,
    AgentAdapterCapability::ProjectDetection,
    AgentAdapterCapability::HookInstallation,
    AgentAdapterCapability::SessionIo,
    AgentAdapterCapability::TranscriptIo,
    AgentAdapterCapability::TranscriptAnalysis,
    AgentAdapterCapability::TokenCalculation,
];

fn builtin_registrations() -> Vec<AgentAdapterRegistration> {
    vec![
        AgentAdapterRegistration::new(
            AgentAdapterDescriptor {
                id: AGENT_NAME_CLAUDE_CODE,
                display_name: "Claude Code",
                agent_type: AGENT_TYPE_CLAUDE_CODE,
                aliases: &[],
                is_default: true,
                capabilities: BASE_CAPABILITIES,
                compatibility: AgentAdapterCompatibility::phase1(),
            },
            || Box::new(ClaudeCodeAgent),
            |repo_root| repo_root.join(".claude").is_dir(),
            claude_hooks::are_hooks_installed,
            |repo_root, _local_dev, force| claude_hooks::install_hooks(repo_root, force),
            claude_hooks::uninstall_hooks,
            |_session_id| "claude".to_string(),
        ),
        AgentAdapterRegistration::new(
            AgentAdapterDescriptor {
                id: AGENT_NAME_COPILOT,
                display_name: "Copilot",
                agent_type: AGENT_TYPE_COPILOT,
                aliases: &["copilot-cli", "github-copilot"],
                is_default: false,
                capabilities: ANALYTICS_CAPABILITIES,
                compatibility: AgentAdapterCompatibility::phase1(),
            },
            || Box::new(CopilotCliAgent),
            |_repo_root| HookSupport::are_hooks_installed(&CopilotCliAgent),
            |_repo_root| HookSupport::are_hooks_installed(&CopilotCliAgent),
            |_repo_root, local_dev, force| {
                HookSupport::install_hooks(&CopilotCliAgent, local_dev, force)
            },
            |_repo_root| HookSupport::uninstall_hooks(&CopilotCliAgent),
            |session_id| {
                if session_id.trim().is_empty() {
                    "copilot".to_string()
                } else {
                    format!("copilot --resume {session_id}")
                }
            },
        ),
        AgentAdapterRegistration::new(
            AgentAdapterDescriptor {
                id: AGENT_NAME_CODEX,
                display_name: "Codex CLI",
                agent_type: AGENT_TYPE_CODEX,
                aliases: &[],
                is_default: false,
                capabilities: BASE_CAPABILITIES,
                compatibility: AgentAdapterCompatibility::phase1(),
            },
            || Box::new(CodexAgent),
            |repo_root| repo_root.join(".codex").is_dir(),
            codex_hooks::are_hooks_installed_at,
            codex_hooks::install_hooks_at,
            codex_hooks::uninstall_hooks_at,
            |session_id| {
                if session_id.trim().is_empty() {
                    "codex".to_string()
                } else {
                    format!("codex --resume {session_id}")
                }
            },
        ),
        AgentAdapterRegistration::new(
            AgentAdapterDescriptor {
                id: AGENT_NAME_CURSOR,
                display_name: "Cursor",
                agent_type: AGENT_TYPE_CURSOR,
                aliases: &[],
                is_default: false,
                capabilities: BASE_CAPABILITIES,
                compatibility: AgentAdapterCompatibility::phase1(),
            },
            || Box::new(CursorAgent),
            |repo_root| repo_root.join(".cursor").is_dir(),
            |_repo_root| HookSupport::are_hooks_installed(&CursorAgent),
            |_repo_root, local_dev, force| {
                HookSupport::install_hooks(&CursorAgent, local_dev, force)
            },
            |_repo_root| HookSupport::uninstall_hooks(&CursorAgent),
            |_session_id| "Open this project in Cursor to continue the session.".to_string(),
        ),
        AgentAdapterRegistration::new(
            AgentAdapterDescriptor {
                id: AGENT_TYPE_GEMINI,
                display_name: "Gemini CLI",
                agent_type: AGENT_TYPE_GEMINI,
                aliases: &[AGENT_NAME_GEMINI],
                is_default: false,
                capabilities: ANALYTICS_CAPABILITIES,
                compatibility: AgentAdapterCompatibility::phase1(),
            },
            || Box::new(GeminiCliAgent),
            |repo_root| repo_root.join(".gemini").is_dir(),
            |_repo_root| HookSupport::are_hooks_installed(&GeminiCliAgent),
            |_repo_root, local_dev, force| {
                HookSupport::install_hooks(&GeminiCliAgent, local_dev, force)
            },
            |_repo_root| HookSupport::uninstall_hooks(&GeminiCliAgent),
            |_session_id| "gemini".to_string(),
        ),
        AgentAdapterRegistration::new(
            AgentAdapterDescriptor {
                id: AGENT_NAME_OPEN_CODE,
                display_name: "OpenCode",
                agent_type: AGENT_TYPE_OPEN_CODE,
                aliases: &["open-code"],
                is_default: false,
                capabilities: BASE_CAPABILITIES,
                compatibility: AgentAdapterCompatibility::phase1(),
            },
            || Box::new(OpenCodeAgent),
            |repo_root| repo_root.join(".opencode").is_dir(),
            |_repo_root| HookSupport::are_hooks_installed(&OpenCodeAgent),
            |_repo_root, local_dev, force| {
                HookSupport::install_hooks(&OpenCodeAgent, local_dev, force)
            },
            |_repo_root| HookSupport::uninstall_hooks(&OpenCodeAgent),
            |session_id| {
                if session_id.trim().is_empty() {
                    "opencode".to_string()
                } else {
                    format!("opencode -s {session_id}")
                }
            },
        ),
    ]
}
