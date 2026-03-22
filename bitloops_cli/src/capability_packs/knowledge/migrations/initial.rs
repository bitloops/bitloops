use anyhow::Result;

use crate::host::devql::capability_host::{
    CapabilityMigration, CapabilityMigrationContext, MigrationRunner,
};

fn run_initial_knowledge_migration(ctx: &mut dyn CapabilityMigrationContext) -> Result<()> {
    ctx.relational()
        .expect("knowledge pack requires relational gateway")
        .initialise_schema()?;
    ctx.documents()
        .expect("knowledge pack requires documents gateway")
        .initialise_schema()?;
    Ok(())
}

pub static KNOWLEDGE_MIGRATIONS: &[CapabilityMigration] = &[CapabilityMigration {
    capability_id: "knowledge",
    version: "0.1.0",
    description: "Initial knowledge schema",
    run: MigrationRunner::Core(run_initial_knowledge_migration),
}];
