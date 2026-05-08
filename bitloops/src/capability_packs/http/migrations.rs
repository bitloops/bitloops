use anyhow::Result;

use crate::host::capability_host::{
    CapabilityMigration, CapabilityMigrationContext, MigrationRunner,
};

use super::schema::http_sqlite_schema_sql;
use super::types::HTTP_CAPABILITY_ID;

fn run_http_schema(ctx: &mut dyn CapabilityMigrationContext) -> Result<()> {
    ctx.apply_devql_sqlite_ddl(http_sqlite_schema_sql())
}

pub static HTTP_MIGRATIONS: &[CapabilityMigration] = &[CapabilityMigration {
    capability_id: HTTP_CAPABILITY_ID,
    version: "0.1.0",
    description: "Ensure HTTP primitive, bundle, evidence, and role-query projection tables on DevQL SQLite relational",
    run: MigrationRunner::Core(run_http_schema),
}];
