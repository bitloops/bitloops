use std::error::Error;
use std::fmt::{self, Display, Formatter};

use crate::engine::extensions::capability::CapabilityPackRegistryError;
use crate::engine::extensions::language::LanguagePackRegistryError;
use crate::engine::extensions::lifecycle::ExtensionCompatibilityError;

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
            Self::Language(error) => write!(f, "language pack registration failed: {error}"),
            Self::Capability(error) => {
                write!(f, "capability pack registration failed: {error}")
            }
            Self::Compatibility(error) => {
                write!(f, "extension compatibility check failed: {error}")
            }
            Self::Migration(message) => write!(f, "capability migration failed: {message}"),
            Self::CapabilityStageNotRegistered(stage_id) => {
                write!(
                    f,
                    "capability stage `{stage_id}` is not owned by any registered capability pack"
                )
            }
            Self::CapabilityIngesterNotRegistered(ingester_id) => {
                write!(
                    f,
                    "capability ingester `{ingester_id}` is not owned by any registered capability pack"
                )
            }
            Self::CapabilityNotReady {
                capability_pack_id,
                reason,
            } => {
                write!(
                    f,
                    "capability pack `{capability_pack_id}` is not ready: {reason}"
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
