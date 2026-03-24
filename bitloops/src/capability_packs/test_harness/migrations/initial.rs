use anyhow::Result;

use crate::host::capability_host::{
    CapabilityMigration, CapabilityMigrationContext, MigrationRunner,
};

fn run_test_harness_domain_schema(ctx: &mut dyn CapabilityMigrationContext) -> Result<()> {
    ctx.apply_devql_sqlite_ddl(
        crate::capability_packs::test_harness::storage::schema::sqlite_test_domain_schema_sql(),
    )
}

pub static TEST_HARNESS_MIGRATIONS: &[CapabilityMigration] = &[CapabilityMigration {
    capability_id: "test_harness",
    version: "0.2.0",
    description: "Ensure test-domain tables for discovery, coverage, and classifications on DevQL SQLite relational",
    run: MigrationRunner::Core(run_test_harness_domain_schema),
}];
