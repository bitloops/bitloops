use anyhow::Result;

use crate::host::capability_host::{
    CapabilityMigration, CapabilityMigrationContext, MigrationRunner,
};

use super::schema::navigation_context_sqlite_schema_sql;
use super::types::NAVIGATION_CONTEXT_CAPABILITY_ID;

fn run_navigation_context_schema(ctx: &mut dyn CapabilityMigrationContext) -> Result<()> {
    ctx.apply_devql_sqlite_ddl(navigation_context_sqlite_schema_sql())
}

pub static NAVIGATION_CONTEXT_MIGRATIONS: &[CapabilityMigration] = &[CapabilityMigration {
    capability_id: NAVIGATION_CONTEXT_CAPABILITY_ID,
    version: "0.1.0",
    description: "Ensure navigation-context primitive, edge, view, and view-dependency tables on DevQL SQLite relational",
    run: MigrationRunner::Core(run_navigation_context_schema),
}];
