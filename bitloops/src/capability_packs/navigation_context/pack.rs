use anyhow::Result;

use crate::host::capability_host::{
    CapabilityDescriptor, CapabilityHealthCheck, CapabilityMigration, CapabilityPack,
    CapabilityRegistrar,
};

use super::descriptor::NAVIGATION_CONTEXT_DESCRIPTOR;
use super::health::NAVIGATION_CONTEXT_HEALTH_CHECKS;
use super::migrations::NAVIGATION_CONTEXT_MIGRATIONS;
use super::register::register_navigation_context_pack;

pub struct NavigationContextPack;

impl NavigationContextPack {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NavigationContextPack {
    fn default() -> Self {
        Self::new()
    }
}

impl CapabilityPack for NavigationContextPack {
    fn descriptor(&self) -> &'static CapabilityDescriptor {
        &NAVIGATION_CONTEXT_DESCRIPTOR
    }

    fn register(&self, registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
        register_navigation_context_pack(registrar)
    }

    fn migrations(&self) -> &'static [CapabilityMigration] {
        NAVIGATION_CONTEXT_MIGRATIONS
    }

    fn health_checks(&self) -> &'static [CapabilityHealthCheck] {
        NAVIGATION_CONTEXT_HEALTH_CHECKS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_context_pack_exposes_descriptor_migration_and_health_checks() {
        let pack = NavigationContextPack::new();

        assert_eq!(pack.descriptor().id, "navigation_context");
        assert_eq!(pack.descriptor().display_name, "Navigation Context");
        assert_eq!(pack.migrations().len(), 1);
        assert_eq!(pack.migrations()[0].capability_id, "navigation_context");
        assert_eq!(pack.health_checks().len(), 2);
    }
}
