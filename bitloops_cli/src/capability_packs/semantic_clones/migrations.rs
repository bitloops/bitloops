use anyhow::Result;

use crate::engine::devql::capability_host::{
    CapabilityMigration, CapabilityMigrationContext, MigrationRunner,
};

use super::schema::semantic_clones_sqlite_schema_sql;

fn run_semantic_clones_initial_schema(ctx: &mut dyn CapabilityMigrationContext) -> Result<()> {
    ctx.apply_devql_sqlite_ddl(semantic_clones_sqlite_schema_sql())
}

pub static SEMANTIC_CLONES_MIGRATIONS: &[CapabilityMigration] = &[CapabilityMigration {
    capability_id: super::types::SEMANTIC_CLONES_CAPABILITY_ID,
    version: "0.1.0",
    description: "symbol_clone_edges on DevQL SQLite relational",
    run: MigrationRunner::Core(run_semantic_clones_initial_schema),
}];
