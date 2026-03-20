use anyhow::Result;

use super::contexts::CapabilityMigrationContext;

#[derive(Debug, Clone, Copy)]
pub struct CapabilityMigration {
    pub capability_id: &'static str,
    pub version: &'static str,
    pub description: &'static str,
    pub run: fn(&mut dyn CapabilityMigrationContext) -> Result<()>,
}
