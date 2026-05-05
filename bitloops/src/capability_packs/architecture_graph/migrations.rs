use anyhow::Result;

use crate::host::capability_host::{
    CapabilityMigration, CapabilityMigrationContext, MigrationRunner,
};

use super::schema::architecture_graph_sqlite_schema_sql;
use super::types::ARCHITECTURE_GRAPH_CAPABILITY_ID;

fn run_architecture_graph_schema(ctx: &mut dyn CapabilityMigrationContext) -> Result<()> {
    let schema = architecture_graph_sqlite_schema_sql();
    ctx.apply_devql_sqlite_ddl(&schema)
}

pub static ARCHITECTURE_GRAPH_MIGRATIONS: &[CapabilityMigration] = &[CapabilityMigration {
    capability_id: ARCHITECTURE_GRAPH_CAPABILITY_ID,
    version: "0.1.0",
    description: "Ensure architecture-graph fact, assertion, run-status, and role metadata tables on DevQL SQLite relational",
    run: MigrationRunner::Core(run_architecture_graph_schema),
}];
