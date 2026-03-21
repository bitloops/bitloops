use anyhow::Result;

use super::contexts::{CapabilityMigrationContext, KnowledgeMigrationContext};

#[derive(Debug, Clone, Copy)]
pub enum MigrationRunner {
    /// DevQL relational SQLite DDL / repo-only work (semantic_clones, test_harness, …).
    Core(fn(&mut dyn CapabilityMigrationContext) -> Result<()>),
    /// Knowledge relational + document schema (`capabilities/knowledge/storage`).
    Knowledge(fn(&mut dyn KnowledgeMigrationContext) -> Result<()>),
}

#[derive(Debug, Clone, Copy)]
pub struct CapabilityMigration {
    pub capability_id: &'static str,
    pub version: &'static str,
    pub description: &'static str,
    pub run: MigrationRunner,
}
