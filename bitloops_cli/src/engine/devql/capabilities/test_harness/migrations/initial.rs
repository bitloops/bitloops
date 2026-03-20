use anyhow::Result;

use crate::engine::devql::capability_host::{CapabilityMigration, CapabilityMigrationContext};

fn run_initial_test_harness_scaffold_migration(
    _ctx: &mut dyn CapabilityMigrationContext,
) -> Result<()> {
    Ok(())
}

pub static TEST_HARNESS_MIGRATIONS: &[CapabilityMigration] = &[CapabilityMigration {
    capability_id: "test_harness",
    version: "0.1.0",
    description: "Scaffold migration placeholder awaiting dedicated test-harness storage gateways",
    run: run_initial_test_harness_scaffold_migration,
}];
