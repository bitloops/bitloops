use anyhow::Result;

use crate::host::capability_host::{
    CapabilityDescriptor, CapabilityHealthCheck, CapabilityMigration, CapabilityPack,
    CapabilityRegistrar,
};

use super::descriptor::ARCHITECTURE_GRAPH_DESCRIPTOR;
use super::health::ARCHITECTURE_GRAPH_HEALTH_CHECKS;
use super::migrations::ARCHITECTURE_GRAPH_MIGRATIONS;
use super::register::register_architecture_graph_pack;

pub struct ArchitectureGraphPack;

impl ArchitectureGraphPack {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ArchitectureGraphPack {
    fn default() -> Self {
        Self::new()
    }
}

impl CapabilityPack for ArchitectureGraphPack {
    fn descriptor(&self) -> &'static CapabilityDescriptor {
        &ARCHITECTURE_GRAPH_DESCRIPTOR
    }

    fn register(&self, registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
        register_architecture_graph_pack(registrar)
    }

    fn migrations(&self) -> &'static [CapabilityMigration] {
        ARCHITECTURE_GRAPH_MIGRATIONS
    }

    fn health_checks(&self) -> &'static [CapabilityHealthCheck] {
        ARCHITECTURE_GRAPH_HEALTH_CHECKS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn architecture_graph_pack_exposes_descriptor_migration_and_health_checks() {
        let pack = ArchitectureGraphPack::new();

        assert_eq!(pack.descriptor().id, "architecture_graph");
        assert_eq!(pack.descriptor().display_name, "Architecture Graph");
        assert_eq!(pack.migrations().len(), 1);
        assert_eq!(pack.migrations()[0].capability_id, "architecture_graph");
        assert_eq!(pack.health_checks().len(), 2);
    }
}
