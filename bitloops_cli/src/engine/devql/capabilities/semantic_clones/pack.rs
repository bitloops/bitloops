use anyhow::Result;

use crate::engine::devql::capability_host::{
    CapabilityDescriptor, CapabilityHealthCheck, CapabilityMigration, CapabilityPack,
    CapabilityRegistrar,
};

use super::descriptor::SEMANTIC_CLONES_DESCRIPTOR;
use super::health::SEMANTIC_CLONES_HEALTH_CHECKS;
use super::migrations::SEMANTIC_CLONES_MIGRATIONS;
use super::register::register_semantic_clones_pack;

pub struct SemanticClonesPack;

impl SemanticClonesPack {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }
}

impl CapabilityPack for SemanticClonesPack {
    fn descriptor(&self) -> &'static CapabilityDescriptor {
        &SEMANTIC_CLONES_DESCRIPTOR
    }

    fn register(&self, registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
        register_semantic_clones_pack(registrar)
    }

    fn migrations(&self) -> &'static [CapabilityMigration] {
        SEMANTIC_CLONES_MIGRATIONS
    }

    fn health_checks(&self) -> &'static [CapabilityHealthCheck] {
        SEMANTIC_CLONES_HEALTH_CHECKS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_clones_pack_descriptor_and_migrations() -> Result<()> {
        let pack = SemanticClonesPack::new()?;
        assert_eq!(pack.descriptor().id, "semantic_clones");
        assert_eq!(pack.migrations()[0].capability_id, "semantic_clones");
        assert_eq!(pack.health_checks().len(), 1);
        Ok(())
    }
}
