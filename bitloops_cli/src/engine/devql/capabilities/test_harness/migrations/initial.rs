use anyhow::Result;

use crate::engine::devql::capability_host::{CapabilityMigration, CapabilityMigrationContext};

fn run_test_harness_domain_schema(ctx: &mut dyn CapabilityMigrationContext) -> Result<()> {
    ctx.apply_devql_sqlite_ddl(crate::db::test_domain_schema_sql())
}

pub static TEST_HARNESS_MIGRATIONS: &[CapabilityMigration] = &[CapabilityMigration {
    capability_id: "test_harness",
    version: "0.2.0",
    description: "Ensure test-domain tables (suites, scenarios, links, coverage, classifications) on DevQL SQLite relational",
    run: run_test_harness_domain_schema,
}];
