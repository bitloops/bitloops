use anyhow::Result;

use crate::engine::devql::capability_host::{
    CapabilityMigration, KnowledgeMigrationContext, MigrationRunner,
};

fn run_initial_knowledge_migration(ctx: &mut dyn KnowledgeMigrationContext) -> Result<()> {
    ctx.relational().initialise_schema()?;
    ctx.documents().initialise_schema()?;
    Ok(())
}

pub static KNOWLEDGE_MIGRATIONS: &[CapabilityMigration] = &[CapabilityMigration {
    capability_id: "knowledge",
    version: "0.1.0",
    description: "Initial knowledge schema",
    run: MigrationRunner::Knowledge(run_initial_knowledge_migration),
}];
