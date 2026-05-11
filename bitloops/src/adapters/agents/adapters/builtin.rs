use super::super::{
    AGENT_NAME_CLAUDE_CODE, AGENT_NAME_CODEX, AGENT_NAME_COPILOT, AGENT_NAME_CURSOR,
    AGENT_NAME_GEMINI, AGENT_NAME_OPEN_CODE, AGENT_TYPE_CLAUDE_CODE, AGENT_TYPE_CODEX,
    AGENT_TYPE_COPILOT, AGENT_TYPE_CURSOR, AGENT_TYPE_GEMINI, AGENT_TYPE_OPEN_CODE,
};
use super::registration::AgentAdapterRegistration;
use super::types::{
    AgentAdapterCapability, AgentAdapterCompatibility, AgentAdapterDescriptor,
    AgentAdapterPackageDescriptor, AgentAdapterRuntimeCompatibility, AgentConfigSchema,
    AgentProtocolFamilyDescriptor, AgentTargetProfileDescriptor,
};
use crate::adapters::agents::claude_code::agent::ClaudeCodeAgent;
use crate::adapters::agents::claude_code::hooks as claude_hooks;
use crate::adapters::agents::claude_code::skills as claude_skills;
use crate::adapters::agents::codex::agent::CodexAgent;
use crate::adapters::agents::codex::hooks as codex_hooks;
use crate::adapters::agents::codex::skills as codex_skills;
use crate::adapters::agents::copilot::agent::CopilotCliAgent;
use crate::adapters::agents::copilot::hooks as copilot_hooks;
use crate::adapters::agents::copilot::skills as copilot_skills;
use crate::adapters::agents::cursor::agent::CursorAgent;
use crate::adapters::agents::cursor::hooks as cursor_hooks;
use crate::adapters::agents::cursor::rules as cursor_rules;
use crate::adapters::agents::gemini::agent::GeminiCliAgent;
use crate::adapters::agents::gemini::skills as gemini_skills;
use crate::adapters::agents::open_code::agent::OpenCodeAgent;
use crate::adapters::agents::open_code::skills as open_code_skills;

const PROTOCOL_FAMILY_JSONL_CLI: &str = "jsonl-cli";
const PROTOCOL_FAMILY_JSON_EVENT: &str = "json-event";

const BASE_CAPABILITIES: &[AgentAdapterCapability] = &[
    AgentAdapterCapability::PresenceDetection,
    AgentAdapterCapability::ProjectDetection,
    AgentAdapterCapability::HookInstallation,
    AgentAdapterCapability::SessionIo,
    AgentAdapterCapability::TranscriptIo,
    AgentAdapterCapability::LifecycleRouting,
];

const ANALYTICS_CAPABILITIES: &[AgentAdapterCapability] = &[
    AgentAdapterCapability::PresenceDetection,
    AgentAdapterCapability::ProjectDetection,
    AgentAdapterCapability::HookInstallation,
    AgentAdapterCapability::SessionIo,
    AgentAdapterCapability::TranscriptIo,
    AgentAdapterCapability::TranscriptAnalysis,
    AgentAdapterCapability::TokenCalculation,
    AgentAdapterCapability::LifecycleRouting,
];

fn protocol_family_jsonl() -> AgentProtocolFamilyDescriptor {
    AgentProtocolFamilyDescriptor {
        id: PROTOCOL_FAMILY_JSONL_CLI,
        display_name: "JSONL CLI Hooks",
        capabilities: BASE_CAPABILITIES,
        compatibility: AgentAdapterCompatibility::phase1(),
        runtime: AgentAdapterRuntimeCompatibility::local_cli(),
        config_schema: AgentConfigSchema::empty("family.jsonl-cli"),
    }
}

fn protocol_family_json_event() -> AgentProtocolFamilyDescriptor {
    AgentProtocolFamilyDescriptor {
        id: PROTOCOL_FAMILY_JSON_EVENT,
        display_name: "JSON Event Hooks",
        capabilities: ANALYTICS_CAPABILITIES,
        compatibility: AgentAdapterCompatibility::phase1(),
        runtime: AgentAdapterRuntimeCompatibility::local_cli(),
        config_schema: AgentConfigSchema::empty("family.json-event"),
    }
}

fn profile_for(
    profile_id: &'static str,
    display_name: &'static str,
    family_id: &'static str,
    aliases: &'static [&'static str],
    capabilities: &'static [AgentAdapterCapability],
) -> AgentTargetProfileDescriptor {
    AgentTargetProfileDescriptor {
        id: profile_id,
        display_name,
        family_id,
        aliases,
        capabilities,
        compatibility: AgentAdapterCompatibility::phase1(),
        runtime: AgentAdapterRuntimeCompatibility::local_cli(),
        config_schema: AgentConfigSchema::empty("profile.default"),
    }
}

pub(super) fn builtin_registrations() -> Vec<AgentAdapterRegistration> {
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
                runtime: AgentAdapterRuntimeCompatibility::local_cli(),
                protocol_family: protocol_family_jsonl(),
                target_profile: profile_for(
                    AGENT_NAME_CLAUDE_CODE,
                    "Claude Code",
                    PROTOCOL_FAMILY_JSONL_CLI,
                    &[],
                    BASE_CAPABILITIES,
                ),
                package: AgentAdapterPackageDescriptor::first_party_linked(
                    AGENT_NAME_CLAUDE_CODE,
                    "Claude Code",
                ),
                config_schema: AgentConfigSchema::empty("adapter.claude-code"),
            },
            || Box::new(ClaudeCodeAgent),
            |repo_root| repo_root.join(".claude").is_dir(),
            claude_hooks::are_hooks_installed,
            |repo_root, _local_dev, force, options| {
                claude_hooks::install_hooks_with_bitloops_skill(
                    repo_root,
                    force,
                    options.install_bitloops_skill,
                )
            },
            claude_hooks::uninstall_hooks,
            |repo_root| claude_skills::repo_skill_path(repo_root).is_file(),
            claude_skills::install_repo_skill,
            claude_skills::uninstall_repo_skill,
            |_session_id| "claude".to_string(),
        ),
        AgentAdapterRegistration::new(
            AgentAdapterDescriptor {
                id: AGENT_NAME_COPILOT,
                display_name: "Copilot",
                agent_type: AGENT_TYPE_COPILOT,
                aliases: &["copilot", "copilot-cli", "github-copilot"],
                is_default: false,
                capabilities: ANALYTICS_CAPABILITIES,
                compatibility: AgentAdapterCompatibility::phase1(),
                runtime: AgentAdapterRuntimeCompatibility::local_cli(),
                protocol_family: protocol_family_json_event(),
                target_profile: profile_for(
                    AGENT_NAME_COPILOT,
                    "Copilot",
                    PROTOCOL_FAMILY_JSON_EVENT,
                    &["copilot", "copilot-cli", "github-copilot"],
                    ANALYTICS_CAPABILITIES,
                ),
                package: AgentAdapterPackageDescriptor::first_party_linked(
                    AGENT_NAME_COPILOT,
                    "Copilot",
                ),
                config_schema: AgentConfigSchema::empty("adapter.copilot"),
            },
            || Box::new(CopilotCliAgent),
            copilot_hooks::are_hooks_installed_at,
            copilot_hooks::are_hooks_installed_at,
            |repo_root, local_dev, force, options| {
                copilot_hooks::install_hooks_at_with_bitloops_skill(
                    repo_root,
                    local_dev,
                    force,
                    options.install_bitloops_skill,
                )
            },
            copilot_hooks::uninstall_hooks_at,
            |repo_root| copilot_skills::repo_skill_path(repo_root).is_file(),
            copilot_skills::install_repo_skill,
            copilot_skills::uninstall_repo_skill,
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
                display_name: "Codex",
                agent_type: AGENT_TYPE_CODEX,
                aliases: &[],
                is_default: false,
                capabilities: BASE_CAPABILITIES,
                compatibility: AgentAdapterCompatibility::phase1(),
                runtime: AgentAdapterRuntimeCompatibility::local_cli(),
                protocol_family: protocol_family_jsonl(),
                target_profile: profile_for(
                    AGENT_NAME_CODEX,
                    "Codex",
                    PROTOCOL_FAMILY_JSONL_CLI,
                    &[],
                    BASE_CAPABILITIES,
                ),
                package: AgentAdapterPackageDescriptor::first_party_linked(
                    AGENT_NAME_CODEX,
                    "Codex",
                ),
                config_schema: AgentConfigSchema::empty("adapter.codex"),
            },
            || Box::new(CodexAgent),
            |repo_root| repo_root.join(".codex").is_dir(),
            codex_hooks::are_hooks_installed_at,
            |repo_root, local_dev, force, options| {
                codex_hooks::install_hooks_at_with_bitloops_skill(
                    repo_root,
                    local_dev,
                    force,
                    options.install_bitloops_skill,
                )
            },
            codex_hooks::uninstall_hooks_at,
            |repo_root| codex_skills::repo_skill_path(repo_root).is_file(),
            codex_skills::install_repo_skill,
            codex_skills::uninstall_repo_skill,
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
                runtime: AgentAdapterRuntimeCompatibility::local_cli(),
                protocol_family: protocol_family_jsonl(),
                target_profile: profile_for(
                    AGENT_NAME_CURSOR,
                    "Cursor",
                    PROTOCOL_FAMILY_JSONL_CLI,
                    &[],
                    BASE_CAPABILITIES,
                ),
                package: AgentAdapterPackageDescriptor::first_party_linked(
                    AGENT_NAME_CURSOR,
                    "Cursor",
                ),
                config_schema: AgentConfigSchema::empty("adapter.cursor"),
            },
            || Box::new(CursorAgent),
            |repo_root| repo_root.join(".cursor").is_dir(),
            cursor_hooks::are_hooks_installed_at,
            |repo_root, local_dev, force, options| {
                cursor_hooks::install_hooks_at_with_bitloops_skill(
                    repo_root,
                    local_dev,
                    force,
                    options.install_bitloops_skill,
                )
            },
            cursor_hooks::uninstall_hooks_at,
            |repo_root| cursor_rules::repo_rule_path(repo_root).is_file(),
            cursor_rules::install_repo_rule,
            cursor_rules::uninstall_repo_rule,
            |_session_id| "Open this project in Cursor to continue the session.".to_string(),
        ),
        AgentAdapterRegistration::new(
            AgentAdapterDescriptor {
                id: AGENT_TYPE_GEMINI,
                display_name: "Gemini",
                agent_type: AGENT_TYPE_GEMINI,
                aliases: &[AGENT_NAME_GEMINI],
                is_default: false,
                capabilities: ANALYTICS_CAPABILITIES,
                compatibility: AgentAdapterCompatibility::phase1(),
                runtime: AgentAdapterRuntimeCompatibility::local_cli(),
                protocol_family: protocol_family_json_event(),
                target_profile: profile_for(
                    AGENT_TYPE_GEMINI,
                    "Gemini",
                    PROTOCOL_FAMILY_JSON_EVENT,
                    &[AGENT_NAME_GEMINI],
                    ANALYTICS_CAPABILITIES,
                ),
                package: AgentAdapterPackageDescriptor::first_party_linked(
                    AGENT_TYPE_GEMINI,
                    "Gemini",
                ),
                config_schema: AgentConfigSchema::empty("adapter.gemini"),
            },
            || Box::new(GeminiCliAgent),
            |repo_root| repo_root.join(".gemini").is_dir(),
            |repo_root| GeminiCliAgent.are_hooks_installed_at(repo_root),
            |repo_root, local_dev, force, options| {
                GeminiCliAgent.install_hooks_at_with_bitloops_skill(
                    repo_root,
                    local_dev,
                    force,
                    options.install_bitloops_skill,
                )
            },
            |repo_root| GeminiCliAgent.uninstall_hooks_at(repo_root),
            |repo_root| gemini_skills::repo_skill_path(repo_root).is_file(),
            gemini_skills::install_repo_skill,
            gemini_skills::uninstall_repo_skill,
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
                runtime: AgentAdapterRuntimeCompatibility::local_cli(),
                protocol_family: protocol_family_jsonl(),
                target_profile: profile_for(
                    AGENT_NAME_OPEN_CODE,
                    "OpenCode",
                    PROTOCOL_FAMILY_JSONL_CLI,
                    &["open-code"],
                    BASE_CAPABILITIES,
                ),
                package: AgentAdapterPackageDescriptor::first_party_linked(
                    AGENT_NAME_OPEN_CODE,
                    "OpenCode",
                ),
                config_schema: AgentConfigSchema::empty("adapter.opencode"),
            },
            || Box::new(OpenCodeAgent),
            |repo_root| repo_root.join(".opencode").is_dir(),
            |repo_root| OpenCodeAgent.are_hooks_installed_at(repo_root),
            |repo_root, local_dev, force, options| {
                OpenCodeAgent.install_hooks_at_with_bitloops_skill(
                    repo_root,
                    local_dev,
                    force,
                    options.install_bitloops_skill,
                )
            },
            |repo_root| OpenCodeAgent.uninstall_hooks_at(repo_root),
            |repo_root| open_code_skills::repo_skill_path(repo_root).is_file(),
            open_code_skills::install_repo_skill,
            open_code_skills::uninstall_repo_skill,
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
