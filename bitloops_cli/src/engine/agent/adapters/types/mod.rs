mod compatibility;
mod config;
mod descriptor;
mod package;
mod readiness;
mod util;

pub use compatibility::{
    AgentAdapterCapability, AgentAdapterCompatibility, AgentAdapterRuntime,
    AgentAdapterRuntimeCompatibility, HOST_ADAPTER_CONTRACT_VERSION, HOST_ADAPTER_RUNTIME_VERSION,
    HOST_PACKAGE_METADATA_VERSION,
};
pub use config::{
    AgentAdapterConfiguration, AgentConfigField, AgentConfigSchema, AgentConfigValidationIssue,
    AgentConfigValueKind,
};
pub use descriptor::{
    AgentAdapterDescriptor, AgentProtocolFamilyDescriptor, AgentTargetProfileDescriptor,
};
pub use package::{
    AgentAdapterPackageBoundary, AgentAdapterPackageCompatibility, AgentAdapterPackageDescriptor,
    AgentAdapterPackageDiagnostic, AgentAdapterPackageDiscovery,
    AgentAdapterPackageDiscoveryStatus, AgentAdapterPackageLifecycle,
    AgentAdapterPackageLifecyclePhase, AgentAdapterPackageMetadataVersion,
    AgentAdapterPackageResponsibility, AgentAdapterPackageSource, AgentAdapterPackageTrustModel,
};
pub(crate) use readiness::AliasResolutionSource;
pub use readiness::{
    AgentAdapterReadiness, AgentReadinessFailure, AgentReadinessStatus,
    AgentRegistrationObservation, AgentResolutionTrace, AgentResolvedRegistration,
};
pub(crate) use util::normalise_key;
