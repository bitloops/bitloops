use anyhow::Result;
use std::path::Path;

use super::super::Agent;
use super::super::adapters::{
    AgentAdapterCapability, AgentAdapterCompatibility, AgentAdapterDescriptor,
    AgentAdapterPackageDescriptor, AgentAdapterRegistration, AgentAdapterRuntime,
    AgentAdapterRuntimeCompatibility, AgentConfigField, AgentConfigSchema, AgentConfigValueKind,
    AgentHookInstallOptions, AgentProtocolFamilyDescriptor, AgentTargetProfileDescriptor,
};

pub(super) const NO_ALIASES: &[&str] = &[];
pub(super) const ALIAS_ALPHA: &[&str] = &["alpha-cli"];
pub(super) const ALIAS_BETA: &[&str] = &["beta-cli"];
pub(super) const SHARED_ALIAS: &[&str] = &["shared-cli"];
pub(super) const BASE_CAPABILITIES: &[AgentAdapterCapability] =
    &[AgentAdapterCapability::HookInstallation];

pub(super) const PROFILE_ALPHA_ALIASES: &[&str] = &["alpha-profile"];
pub(super) const PROFILE_BETA_ALIASES: &[&str] = &["beta-profile"];

pub(super) const EMPTY_SCHEMA: AgentConfigSchema = AgentConfigSchema::empty("test.none");
pub(super) const ALPHA_REQUIRED_FIELDS: &[AgentConfigField] = &[AgentConfigField {
    key: "api_key",
    value_kind: AgentConfigValueKind::String,
    required: true,
}];
pub(super) const ALPHA_REQUIRED_SCHEMA: AgentConfigSchema = AgentConfigSchema {
    namespace: "test.profile.alpha",
    fields: ALPHA_REQUIRED_FIELDS,
    mutually_exclusive: &[],
};

pub(super) const LOCAL_RUNTIME: AgentAdapterRuntimeCompatibility =
    AgentAdapterRuntimeCompatibility {
        supported_runtimes: &[AgentAdapterRuntime::LocalCli],
    };
pub(super) const REMOTE_ONLY_RUNTIME: AgentAdapterRuntimeCompatibility =
    AgentAdapterRuntimeCompatibility {
        supported_runtimes: &[AgentAdapterRuntime::RemoteRuntime],
    };

struct TestAgent {
    name: &'static str,
    agent_type: &'static str,
    dirs: &'static [&'static str],
}

impl Agent for TestAgent {
    fn name(&self) -> String {
        self.name.to_string()
    }

    fn agent_type(&self) -> String {
        self.agent_type.to_string()
    }

    fn protected_dirs(&self) -> Vec<String> {
        self.dirs.iter().map(|dir| (*dir).to_string()).collect()
    }
}

fn create_alpha_agent() -> Box<dyn Agent + Send + Sync> {
    Box::new(TestAgent {
        name: "alpha",
        agent_type: "alpha-type",
        dirs: &[".alpha", ".shared"],
    })
}

fn create_beta_agent() -> Box<dyn Agent + Send + Sync> {
    Box::new(TestAgent {
        name: "beta",
        agent_type: "beta-type",
        dirs: &[".beta", ".shared"],
    })
}

fn detect_true(_: &Path) -> bool {
    true
}

fn detect_false(_: &Path) -> bool {
    false
}

fn hooks_true(_: &Path) -> bool {
    true
}

fn hooks_false(_: &Path) -> bool {
    false
}

fn install_noop(_: &Path, _: bool, _: bool, _: AgentHookInstallOptions) -> Result<usize> {
    Ok(0)
}

fn uninstall_noop(_: &Path) -> Result<()> {
    Ok(())
}

fn install_prompt_surface_noop(_: &Path) -> Result<bool> {
    Ok(false)
}

fn resume_alpha(session_id: &str) -> String {
    if session_id.trim().is_empty() {
        "alpha".to_string()
    } else {
        format!("alpha --resume {session_id}")
    }
}

fn resume_beta(session_id: &str) -> String {
    if session_id.trim().is_empty() {
        "beta".to_string()
    } else {
        format!("beta --resume {session_id}")
    }
}

#[derive(Clone, Copy)]
pub(super) struct AdapterCallbacks {
    create_agent: fn() -> Box<dyn Agent + Send + Sync>,
    detect_project_presence: fn(&Path) -> bool,
    hooks_installed: fn(&Path) -> bool,
    format_resume_command: fn(&str) -> String,
}

pub(super) const ALPHA_CALLBACKS: AdapterCallbacks = AdapterCallbacks {
    create_agent: create_alpha_agent,
    detect_project_presence: detect_true,
    hooks_installed: hooks_true,
    format_resume_command: resume_alpha,
};

pub(super) const BETA_CALLBACKS: AdapterCallbacks = AdapterCallbacks {
    create_agent: create_beta_agent,
    detect_project_presence: detect_false,
    hooks_installed: hooks_false,
    format_resume_command: resume_beta,
};

pub(super) fn test_family(
    family_id: &'static str,
    namespace: &'static str,
    runtime: AgentAdapterRuntimeCompatibility,
) -> AgentProtocolFamilyDescriptor {
    AgentProtocolFamilyDescriptor {
        id: family_id,
        display_name: "Test Family",
        capabilities: BASE_CAPABILITIES,
        compatibility: AgentAdapterCompatibility::phase1(),
        runtime,
        config_schema: AgentConfigSchema::empty(namespace),
    }
}

pub(super) fn test_profile(
    profile_id: &'static str,
    family_id: &'static str,
    aliases: &'static [&'static str],
    schema: AgentConfigSchema,
    runtime: AgentAdapterRuntimeCompatibility,
) -> AgentTargetProfileDescriptor {
    AgentTargetProfileDescriptor {
        id: profile_id,
        display_name: "Test Profile",
        family_id,
        aliases,
        capabilities: BASE_CAPABILITIES,
        compatibility: AgentAdapterCompatibility::phase1(),
        runtime,
        config_schema: schema,
    }
}

pub(super) fn test_package(
    package_id: &'static str,
    display_name: &'static str,
) -> AgentAdapterPackageDescriptor {
    AgentAdapterPackageDescriptor::first_party_linked(package_id, display_name)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn make_registration_with_package(
    id: &'static str,
    display_name: &'static str,
    agent_type: &'static str,
    aliases: &'static [&'static str],
    is_default: bool,
    callbacks: AdapterCallbacks,
    family: AgentProtocolFamilyDescriptor,
    profile: AgentTargetProfileDescriptor,
    runtime: AgentAdapterRuntimeCompatibility,
    package: AgentAdapterPackageDescriptor,
) -> AgentAdapterRegistration {
    AgentAdapterRegistration::new(
        AgentAdapterDescriptor {
            id,
            display_name,
            agent_type,
            aliases,
            is_default,
            capabilities: BASE_CAPABILITIES,
            compatibility: AgentAdapterCompatibility::phase1(),
            runtime,
            config_schema: EMPTY_SCHEMA,
            protocol_family: family,
            target_profile: profile,
            package,
        },
        callbacks.create_agent,
        callbacks.detect_project_presence,
        callbacks.hooks_installed,
        install_noop,
        uninstall_noop,
        callbacks.hooks_installed,
        install_prompt_surface_noop,
        uninstall_noop,
        callbacks.format_resume_command,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn make_registration(
    id: &'static str,
    display_name: &'static str,
    agent_type: &'static str,
    aliases: &'static [&'static str],
    is_default: bool,
    callbacks: AdapterCallbacks,
    family: AgentProtocolFamilyDescriptor,
    profile: AgentTargetProfileDescriptor,
    runtime: AgentAdapterRuntimeCompatibility,
) -> AgentAdapterRegistration {
    make_registration_with_package(
        id,
        display_name,
        agent_type,
        aliases,
        is_default,
        callbacks,
        family,
        profile,
        runtime,
        test_package(id, display_name),
    )
}
