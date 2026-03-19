use std::sync::Arc;

use anyhow::Result;

use crate::engine::devql::capability_host::{
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
