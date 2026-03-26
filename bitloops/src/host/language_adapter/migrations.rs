use anyhow::Result;

use super::LanguageAdapterContext;

pub(crate) type LanguageAdapterMigrationRunner =
    fn(&LanguageAdapterMigrationContext) -> Result<()>;

#[derive(Debug, Clone, Copy)]
pub(crate) struct LanguageAdapterMigrationDescriptor {
    pub(crate) id: &'static str,
    pub(crate) order: u32,
    pub(crate) description: &'static str,
    pub(crate) run: LanguageAdapterMigrationRunner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LanguageAdapterMigrationContext {
    pub(crate) language_adapter_pack_id: String,
    pub(crate) migration_id: String,
    pub(crate) order: u32,
    pub(crate) description: String,
    pub(crate) adapter: LanguageAdapterContext,
}

impl LanguageAdapterMigrationContext {
    pub(crate) fn new(
        language_adapter_pack_id: impl Into<String>,
        migration_id: impl Into<String>,
        order: u32,
        description: impl Into<String>,
        adapter: LanguageAdapterContext,
    ) -> Self {
        Self {
            language_adapter_pack_id: language_adapter_pack_id.into(),
            migration_id: migration_id.into(),
            order,
            description: description.into(),
            adapter,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LanguageAdapterMigrationStep {
    pub(crate) pack_id: String,
    pub(crate) migration_id: String,
    pub(crate) order: u32,
    pub(crate) description: String,
}

impl LanguageAdapterMigrationStep {
    pub(crate) fn from_descriptor(
        pack_id: &str,
        descriptor: &LanguageAdapterMigrationDescriptor,
    ) -> Self {
        Self {
            pack_id: pack_id.to_string(),
            migration_id: descriptor.id.to_string(),
            order: descriptor.order,
            description: descriptor.description.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LanguageAdapterMigrationExecution {
    pub(crate) pack_id: String,
    pub(crate) migration_id: String,
    pub(crate) order: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LanguageAdapterMigrationFailure {
    pub(crate) pack_id: String,
    pub(crate) migration_id: String,
    pub(crate) order: u32,
    pub(crate) reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LanguageAdapterMigrationStatus {
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LanguageAdapterMigrationRunReport {
    pub(crate) status: LanguageAdapterMigrationStatus,
    pub(crate) executed: Vec<LanguageAdapterMigrationExecution>,
    pub(crate) failure: Option<LanguageAdapterMigrationFailure>,
}
