use std::error::Error;
use std::fmt::{self, Display, Formatter};

use super::capability::{
    CapabilityPackDescriptor, CapabilityPackRegistry, CapabilityPackRegistryError,
};
use super::language::{LanguagePackDescriptor, LanguagePackRegistry, LanguagePackRegistryError};

const RUST_LANGUAGE_PACK: LanguagePackDescriptor = LanguagePackDescriptor {
    id: "rust-language-pack",
    display_name: "Rust Language Pack",
    aliases: &["rust-pack"],
    supported_languages: &["rust"],
};

const TS_JS_LANGUAGE_PACK: LanguagePackDescriptor = LanguagePackDescriptor {
    id: "ts-js-language-pack",
    display_name: "TypeScript/JavaScript Language Pack",
    aliases: &["typescript-pack", "javascript-pack"],
    supported_languages: &["typescript", "javascript", "tsx", "jsx"],
};

const SEMANTIC_CLONES_CAPABILITY_PACK: CapabilityPackDescriptor = CapabilityPackDescriptor {
    id: "semantic-clones-capability-pack",
    display_name: "Semantic Clones Capability Pack",
    aliases: &["semantic-clones-pack"],
    stage_contributions: &["semantic-clones"],
    ingester_contributions: &["semantic-clones-ingester"],
};

const KNOWLEDGE_CAPABILITY_PACK: CapabilityPackDescriptor = CapabilityPackDescriptor {
    id: "knowledge-capability-pack",
    display_name: "Knowledge Capability Pack",
    aliases: &["knowledge-pack"],
    stage_contributions: &["knowledge"],
    ingester_contributions: &["knowledge-ingester"],
};

const TEST_HARNESS_CAPABILITY_PACK: CapabilityPackDescriptor = CapabilityPackDescriptor {
    id: "test-harness-capability-pack",
    display_name: "Test Harness Capability Pack",
    aliases: &["test-harness-pack"],
    stage_contributions: &["test-harness"],
    ingester_contributions: &["test-harness-ingester"],
};

#[derive(Debug)]
pub enum CoreExtensionHostError {
    Language(LanguagePackRegistryError),
    Capability(CapabilityPackRegistryError),
}

impl Display for CoreExtensionHostError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Language(error) => write!(f, "language pack registration failed: {error}"),
            Self::Capability(error) => {
                write!(f, "capability pack registration failed: {error}")
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

#[derive(Debug, Clone, Default)]
pub struct CoreExtensionHost {
    language_packs: LanguagePackRegistry,
    capability_packs: CapabilityPackRegistry,
}

impl CoreExtensionHost {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_builtins() -> Result<Self, CoreExtensionHostError> {
        let mut host = Self::new();
        host.bootstrap_builtins()?;
        Ok(host)
    }

    pub fn bootstrap_builtins(&mut self) -> Result<(), CoreExtensionHostError> {
        self.language_packs.register(RUST_LANGUAGE_PACK)?;
        self.language_packs.register(TS_JS_LANGUAGE_PACK)?;

        self.capability_packs
            .register(SEMANTIC_CLONES_CAPABILITY_PACK)?;
        self.capability_packs.register(KNOWLEDGE_CAPABILITY_PACK)?;
        self.capability_packs
            .register(TEST_HARNESS_CAPABILITY_PACK)?;
        Ok(())
    }

    pub fn language_packs(&self) -> &LanguagePackRegistry {
        &self.language_packs
    }

    pub fn language_packs_mut(&mut self) -> &mut LanguagePackRegistry {
        &mut self.language_packs
    }

    pub fn capability_packs(&self) -> &CapabilityPackRegistry {
        &self.capability_packs
    }

    pub fn capability_packs_mut(&mut self) -> &mut CapabilityPackRegistry {
        &mut self.capability_packs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_extension_host_bootstraps_language_and_capability_builtins() {
        let host = CoreExtensionHost::with_builtins().expect("bootstrap builtins");

        assert!(
            host.language_packs().resolve_for_language("rust").is_some(),
            "rust language pack should be resolvable"
        );
        assert!(
            host.language_packs()
                .resolve_for_language("typescript")
                .is_some(),
            "typescript language pack should be resolvable"
        );
        assert_eq!(
            host.capability_packs()
                .resolve_stage_owner("semantic-clones"),
            Some("semantic-clones-capability-pack")
        );
        assert_eq!(
            host.capability_packs()
                .resolve_ingester_owner("test-harness-ingester"),
            Some("test-harness-capability-pack")
        );
    }
}
