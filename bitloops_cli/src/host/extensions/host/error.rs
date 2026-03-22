use std::error::Error;
use std::fmt::{self, Display, Formatter};

use crate::host::extensions::capability::CapabilityPackRegistryError;
use crate::host::extensions::language::LanguagePackRegistryError;
use crate::host::extensions::lifecycle::ExtensionCompatibilityError;

#[derive(Debug)]
pub enum CoreExtensionHostError {
    Language(LanguagePackRegistryError),
    Capability(CapabilityPackRegistryError),
    Compatibility(ExtensionCompatibilityError),
    Migration(String),
    CapabilityStageNotRegistered(String),
    CapabilityIngesterNotRegistered(String),
    CapabilityNotReady {
        capability_pack_id: String,
        reason: String,
    },
}

impl Display for CoreExtensionHostError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Language(error) => {
                write!(
                    f,
                    "[core_extension_host:language_pack] registration failed: {error}"
                )
            }
            Self::Capability(error) => {
                write!(
                    f,
                    "[core_extension_host:capability_pack] registration failed: {error}"
                )
            }
            Self::Compatibility(error) => {
                write!(
                    f,
                    "[core_extension_host:compatibility] check failed: {error}"
                )
            }
            Self::Migration(message) => {
                write!(f, "[core_extension_host:migration] failed: {message}")
            }
            Self::CapabilityStageNotRegistered(stage_id) => {
                write!(
                    f,
                    "[core_extension_host:resolve_stage] [stage:{stage_id}] not owned by any registered capability pack"
                )
            }
            Self::CapabilityIngesterNotRegistered(ingester_id) => {
                write!(
                    f,
                    "[core_extension_host:resolve_ingester] [ingester:{ingester_id}] not owned by any registered capability pack"
                )
            }
            Self::CapabilityNotReady {
                capability_pack_id,
                reason,
            } => {
                write!(
                    f,
                    "[core_extension_host:readiness] [capability_pack:{capability_pack_id}] not ready: {reason}"
                )
            }
        }
    }
}

impl Error for CoreExtensionHostError {}

impl From<LanguagePackRegistryError> for CoreExtensionHostError {
    fn from(value: LanguagePackRegistryError) -> Self {
        Self::Language(value)
    }
}

impl From<CapabilityPackRegistryError> for CoreExtensionHostError {
    fn from(value: CapabilityPackRegistryError) -> Self {
        Self::Capability(value)
    }
}

impl From<ExtensionCompatibilityError> for CoreExtensionHostError {
    fn from(value: ExtensionCompatibilityError) -> Self {
        Self::Compatibility(value)
    }
}
