use super::{
    AgentAdapterCapability, AgentAdapterCompatibility, AgentAdapterPackageDescriptor,
    AgentAdapterRuntimeCompatibility, AgentConfigSchema,
};

#[derive(Debug, Clone)]
pub struct AgentProtocolFamilyDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub capabilities: &'static [AgentAdapterCapability],
    pub compatibility: AgentAdapterCompatibility,
    pub runtime: AgentAdapterRuntimeCompatibility,
    pub config_schema: AgentConfigSchema,
}

#[derive(Debug, Clone)]
pub struct AgentTargetProfileDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub family_id: &'static str,
    pub aliases: &'static [&'static str],
    pub capabilities: &'static [AgentAdapterCapability],
    pub compatibility: AgentAdapterCompatibility,
    pub runtime: AgentAdapterRuntimeCompatibility,
    pub config_schema: AgentConfigSchema,
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
    pub runtime: AgentAdapterRuntimeCompatibility,
    pub protocol_family: AgentProtocolFamilyDescriptor,
    pub target_profile: AgentTargetProfileDescriptor,
    pub package: AgentAdapterPackageDescriptor,
    pub config_schema: AgentConfigSchema,
}
