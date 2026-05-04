use anyhow::Result;

use crate::host::capability_host::{
    CapabilityDescriptor, CapabilityHealthCheck, CapabilityMigration, CapabilityPack,
    CapabilityRegistrar,
};

use super::descriptor::CONTEXT_GUIDANCE_DESCRIPTOR;
use super::health::CONTEXT_GUIDANCE_HEALTH_CHECKS;
use super::migrations::CONTEXT_GUIDANCE_MIGRATIONS;
use super::register::register_context_guidance_pack;

pub struct ContextGuidancePack;

impl ContextGuidancePack {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }
}

impl CapabilityPack for ContextGuidancePack {
    fn descriptor(&self) -> &'static CapabilityDescriptor {
        &CONTEXT_GUIDANCE_DESCRIPTOR
    }

    fn register(&self, registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
        register_context_guidance_pack(registrar)
    }

    fn migrations(&self) -> &'static [CapabilityMigration] {
        CONTEXT_GUIDANCE_MIGRATIONS
    }

    fn health_checks(&self) -> &'static [CapabilityHealthCheck] {
        CONTEXT_GUIDANCE_HEALTH_CHECKS
    }
}
