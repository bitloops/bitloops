use anyhow::Result;

use super::contexts::CapabilityMigrationContext;

#[derive(Debug, Clone, Copy)]
pub enum MigrationRunner {
    /// Pack migration that receives a `CapabilityMigrationContext` (with optional relational /
    /// document gateways via the `relational()` / `documents()` methods).
    Core(fn(&mut dyn CapabilityMigrationContext) -> Result<()>),
}

#[derive(Debug, Clone, Copy)]
pub struct CapabilityMigration {
    pub capability_id: &'static str,
    pub version: &'static str,
    pub description: &'static str,
    pub run: MigrationRunner,
}
