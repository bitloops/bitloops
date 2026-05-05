use anyhow::Result;

use crate::host::capability_host::{
    CapabilityDescriptor, CapabilityHealthCheck, CapabilityMigration, CapabilityPack,
    CapabilityRegistrar,
};

use super::descriptor::CODECITY_DESCRIPTOR;
use super::health::CODECITY_HEALTH_CHECKS;
use super::migrations::CODECITY_MIGRATIONS;
use super::register::register_codecity_pack;

pub struct CodeCityPack;

impl CodeCityPack {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CodeCityPack {
    fn default() -> Self {
        Self::new()
    }
}

impl CapabilityPack for CodeCityPack {
    fn descriptor(&self) -> &'static CapabilityDescriptor {
        &CODECITY_DESCRIPTOR
    }

    fn register(&self, registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
        register_codecity_pack(registrar)
    }

    fn migrations(&self) -> &'static [CapabilityMigration] {
        CODECITY_MIGRATIONS
    }

    fn health_checks(&self) -> &'static [CapabilityHealthCheck] {
        CODECITY_HEALTH_CHECKS
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    #[test]
    fn codecity_pack_exposes_descriptor_and_health_checks() -> Result<()> {
        let pack = CodeCityPack::new();

        assert_eq!(pack.descriptor().id, "codecity");
        assert_eq!(pack.descriptor().display_name, "CodeCity");
        assert!(pack.descriptor().experimental);
        assert_eq!(pack.migrations().len(), 2);
        assert_eq!(pack.migrations()[0].version, "0.3.0");
        assert_eq!(pack.health_checks().len(), 2);
        assert_eq!(pack.health_checks()[0].name, "codecity.config");
        assert_eq!(pack.health_checks()[1].name, "codecity.source_data");
        Ok(())
    }
}
