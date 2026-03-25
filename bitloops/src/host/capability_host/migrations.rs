use anyhow::Result;

use super::contexts::{CapabilityMigrationContext, KnowledgeMigrationContext};

#[derive(Debug, Clone, Copy)]
pub enum MigrationRunner {
    /// Pack migration that receives a core migration context.
    Core(fn(&mut dyn CapabilityMigrationContext) -> Result<()>),
    /// Pack migration that receives knowledge stores in addition to core migration context.
    Knowledge(fn(&mut dyn KnowledgeMigrationContext) -> Result<()>),
}

#[derive(Debug, Clone, Copy)]
pub struct CapabilityMigration {
    pub capability_id: &'static str,
    pub version: &'static str,
    pub description: &'static str,
    pub run: MigrationRunner,
}
