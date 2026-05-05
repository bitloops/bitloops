use anyhow::Result;

use crate::host::capability_host::{
    CapabilityMigration, CapabilityMigrationContext, MigrationRunner,
};

use super::storage::codecity_sqlite_schema_sql;
use super::types::CODECITY_CAPABILITY_ID;

fn run_codecity_health_schema(ctx: &mut dyn CapabilityMigrationContext) -> Result<()> {
    ctx.apply_devql_sqlite_ddl(codecity_sqlite_schema_sql())
}

pub static CODECITY_MIGRATIONS: &[CapabilityMigration] = &[
    CapabilityMigration {
        capability_id: CODECITY_CAPABILITY_ID,
        version: "0.3.0",
        description: "CodeCity health snapshot tables",
        run: MigrationRunner::Core(run_codecity_health_schema),
    },
    CapabilityMigration {
        capability_id: CODECITY_CAPABILITY_ID,
        version: "0.4.0",
        description: "CodeCity architecture diagnostics and render arcs",
        run: MigrationRunner::Core(run_codecity_health_schema),
    },
];
