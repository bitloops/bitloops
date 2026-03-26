use super::super::registration::AgentAdapterRegistration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentReadinessStatus {
    Ready,
    NotReady,
}

impl AgentReadinessStatus {
    pub(crate) fn from_failures(has_failures: bool) -> Self {
        if has_failures {
            Self::NotReady
        } else {
            Self::Ready
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentReadinessFailure {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAdapterReadiness {
    pub id: String,
    pub display_name: String,
    pub package_id: String,
    pub package_metadata_version: u16,
    pub package_version: String,
    pub package_source: String,
    pub package_trust_model: String,
    pub protocol_family: String,
    pub target_profile: String,
    pub runtime: String,
    pub project_detected: bool,
    pub hooks_installed: bool,
    pub compatibility_ok: bool,
    pub config_valid: bool,
    pub status: AgentReadinessStatus,
    pub failures: Vec<AgentReadinessFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRegistrationObservation {
    pub id: String,
    pub adapter_id: String,
    pub package_id: String,
    pub package_metadata_version: u16,
    pub package_version: String,
    pub package_source: String,
    pub package_trust_model: String,
    pub protocol_family: String,
    pub target_profile: String,
    pub runtime: String,
    pub is_default: bool,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AliasResolutionSource {
    LegacyTarget,
    TargetProfile,
}

impl AliasResolutionSource {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::LegacyTarget => "legacy-target-compat",
            Self::TargetProfile => "target-profile",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentResolutionTrace {
    pub correlation_id: String,
    pub requested: String,
    pub resolved_adapter_id: String,
    pub package_id: String,
    pub package_metadata_version: u16,
    pub package_version: String,
    pub package_source: String,
    pub package_trust_model: String,
    pub protocol_family: String,
    pub target_profile: String,
    pub runtime: String,
    pub used_alias: bool,
    pub resolution_path: String,
    pub diagnostics: Vec<String>,
}

pub struct AgentResolvedRegistration<'a> {
    pub registration: &'a AgentAdapterRegistration,
    pub trace: AgentResolutionTrace,
}
