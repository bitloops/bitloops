//! Local capability runtime: wiring repo-backed stores, gateways, and context trait implementations.

mod capability_config;
mod language_services;
mod local_gateways;
mod local_resources;
mod local_runtime;

#[cfg(test)]
mod tests;

pub use language_services::BuiltinLanguageServicesGateway;
#[cfg(test)]
pub(crate) use language_services::builtin_language_services;
pub use local_gateways::{
    DefaultProvenanceBuilder, LocalCanonicalGraphGateway, LocalGitHistoryGateway,
    LocalStoreHealthGateway,
};
pub use local_resources::LocalCapabilityRuntimeResources;
pub use local_runtime::LocalCapabilityRuntime;
