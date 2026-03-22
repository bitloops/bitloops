use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::capability_packs::test_harness::storage::{
    BitloopsTestHarnessRepository, open_repository_for_repo,
};
use crate::host::capability_host::{
    CapabilityDescriptor, CapabilityHealthCheck, CapabilityMigration, CapabilityPack,
    CapabilityRegistrar,
};

use super::descriptor::TEST_HARNESS_DESCRIPTOR;
use super::health::TEST_HARNESS_HEALTH_CHECKS;
use super::migrations::TEST_HARNESS_MIGRATIONS;
use super::register::register_test_harness_pack;

pub struct TestHarnessPack {
    test_harness: Option<Arc<Mutex<BitloopsTestHarnessRepository>>>,
}

impl TestHarnessPack {
    pub fn new(repo_root: &Path) -> Self {
        Self {
            test_harness: open_repository_for_repo(repo_root)
                .ok()
                .map(|r| Arc::new(Mutex::new(r))),
        }
    }
}

impl CapabilityPack for TestHarnessPack {
    fn descriptor(&self) -> &'static CapabilityDescriptor {
        &TEST_HARNESS_DESCRIPTOR
    }

    fn register(&self, registrar: &mut dyn CapabilityRegistrar) -> Result<()> {
        register_test_harness_pack(registrar, self.test_harness.clone())
    }

    fn migrations(&self) -> &'static [CapabilityMigration] {
        TEST_HARNESS_MIGRATIONS
    }

    fn health_checks(&self) -> &'static [CapabilityHealthCheck] {
        TEST_HARNESS_HEALTH_CHECKS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_harness_pack_exposes_descriptor_migrations_and_health_checks() -> Result<()> {
        let pack = TestHarnessPack::new(Path::new("."));

        assert_eq!(pack.descriptor().id, "test_harness");
        assert_eq!(pack.descriptor().display_name, "Test Harness");
        assert_eq!(pack.migrations().len(), 1);
        assert_eq!(pack.migrations()[0].capability_id, "test_harness");
        assert_eq!(pack.health_checks().len(), 3);
        assert_eq!(pack.health_checks()[0].name, "test_harness.config");
        assert_eq!(pack.health_checks()[1].name, "test_harness.storage");
        assert_eq!(pack.health_checks()[2].name, "test_harness.dependencies");
        Ok(())
    }
}
