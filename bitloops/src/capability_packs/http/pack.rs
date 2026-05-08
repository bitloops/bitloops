use anyhow::Result;

use crate::host::capability_host::{
    CapabilityDescriptor, CapabilityHealthCheck, CapabilityMigration, CapabilityPack,
    CapabilityRegistrar,
};

use super::descriptor::HTTP_DESCRIPTOR;
use super::health::HTTP_HEALTH_CHECKS;
use super::migrations::HTTP_MIGRATIONS;
use super::register::register_http_pack;

pub struct HttpPack;

impl HttpPack {
    pub fn new() -> Self {
        Self
    }
}

impl Default for HttpPack {
    fn default() -> Self {
        Self::new()
    }
}

impl CapabilityPack for HttpPack {
    fn descriptor(&self) -> &'static CapabilityDescriptor {
        &HTTP_DESCRIPTOR
    }

    fn register(&self, registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
        register_http_pack(registrar)
    }

    fn migrations(&self) -> &'static [CapabilityMigration] {
        HTTP_MIGRATIONS
    }

    fn health_checks(&self) -> &'static [CapabilityHealthCheck] {
        HTTP_HEALTH_CHECKS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_pack_exposes_descriptor_migration_and_health_checks() {
        let pack = HttpPack::new();

        assert_eq!(pack.descriptor().id, "http");
        assert_eq!(pack.descriptor().display_name, "HTTP");
        assert_eq!(pack.migrations().len(), 1);
        assert_eq!(pack.migrations()[0].capability_id, "http");
        assert_eq!(pack.health_checks().len(), 2);
    }
}
