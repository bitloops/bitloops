use anyhow::Result;

use crate::engine::devql::capability_host::{CapabilityMigration, CapabilityMigrationContext};

fn run_initial_knowledge_migration(ctx: &mut dyn CapabilityMigrationContext) -> Result<()> {
    ctx.knowledge_relational().initialise_schema()?;
    ctx.knowledge_documents().initialise_schema()?;
    Ok(())
}

pub static KNOWLEDGE_MIGRATIONS: &[CapabilityMigration] = &[CapabilityMigration {
    capability_id: "knowledge",
    version: "0.1.0",
    description: "Initial knowledge schema",
    run: run_initial_knowledge_migration,
}];
