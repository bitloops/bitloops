use std::sync::Arc;

use anyhow::Result;

use crate::host::devql::capability_host::{
    CapabilityDescriptor, CapabilityHealthCheck, CapabilityMigration, CapabilityPack,
    CapabilityRegistrar,
};

use super::descriptor::KNOWLEDGE_DESCRIPTOR;
use super::health::KNOWLEDGE_HEALTH_CHECKS;
use super::migrations::KNOWLEDGE_MIGRATIONS;
use super::register::register_knowledge_pack;
use super::services::KnowledgeServices;

pub struct KnowledgePack {
    services: Arc<KnowledgeServices>,
}

impl KnowledgePack {
    pub fn new() -> Result<Self> {
        Ok(Self {
            services: Arc::new(KnowledgeServices::new()),
        })
    }
}

impl CapabilityPack for KnowledgePack {
    fn descriptor(&self) -> &'static CapabilityDescriptor {
        &KNOWLEDGE_DESCRIPTOR
    }

    fn register(&self, registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
        register_knowledge_pack(self.services.clone(), registrar)
    }

    fn migrations(&self) -> &'static [CapabilityMigration] {
        KNOWLEDGE_MIGRATIONS
    }

    fn health_checks(&self) -> &'static [CapabilityHealthCheck] {
        KNOWLEDGE_HEALTH_CHECKS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knowledge_pack_exposes_descriptor_migrations_and_health_checks() -> Result<()> {
        let pack = KnowledgePack::new()?;

        assert_eq!(pack.descriptor().id, "knowledge");
        assert_eq!(pack.descriptor().display_name, "Knowledge");
        assert!(!pack.migrations().is_empty());
        assert_eq!(pack.migrations()[0].capability_id, "knowledge");
        assert_eq!(pack.health_checks().len(), 3);
        assert_eq!(pack.health_checks()[0].name, "knowledge.config");
        assert_eq!(pack.health_checks()[1].name, "knowledge.storage");
        assert_eq!(pack.health_checks()[2].name, "knowledge.connectors");

        Ok(())
    }
}
