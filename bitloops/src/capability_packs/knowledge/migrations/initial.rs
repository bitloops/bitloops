use anyhow::Result;

use crate::host::capability_host::{
    CapabilityMigration, KnowledgeMigrationContext, MigrationRunner,
};

fn run_initial_knowledge_migration(ctx: &mut dyn KnowledgeMigrationContext) -> Result<()> {
    ctx.knowledge_relational().initialise_schema()?;
    Ok(())
}

pub static KNOWLEDGE_MIGRATIONS: &[CapabilityMigration] = &[CapabilityMigration {
    capability_id: "knowledge",
    version: "0.0.11",
    description: "Initial knowledge schema",
    run: MigrationRunner::Knowledge(run_initial_knowledge_migration),
}];
