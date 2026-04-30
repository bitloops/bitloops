use anyhow::Result;

use crate::host::capability_host::{
    CapabilityMigration, CapabilityMigrationContext, MigrationRunner,
};

use super::super::descriptor::CONTEXT_GUIDANCE_CAPABILITY_ID;
use super::super::storage_schema::context_guidance_initial_sqlite_schema_sql;

fn run_context_guidance_initial_schema(ctx: &mut dyn CapabilityMigrationContext) -> Result<()> {
    ctx.apply_devql_sqlite_ddl(context_guidance_initial_sqlite_schema_sql())
}

pub static CONTEXT_GUIDANCE_INITIAL_MIGRATION: CapabilityMigration = CapabilityMigration {
    capability_id: CONTEXT_GUIDANCE_CAPABILITY_ID,
    version: env!("CARGO_PKG_VERSION"),
    description: "Initial context guidance distillation, fact, source, and target tables",
    run: MigrationRunner::Core(run_context_guidance_initial_schema),
};
