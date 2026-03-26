#[derive(Debug, Clone, Copy)]
pub(crate) struct LanguageAdapterHealthCheck {
    pub(crate) name: &'static str,
    pub(crate) run: fn(&LanguageAdapterHealthContext) -> LanguageAdapterHealthResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LanguageAdapterHealthResult {
    pub(crate) healthy: bool,
    pub(crate) message: String,
    pub(crate) details: Option<String>,
}

impl LanguageAdapterHealthResult {
    #[cfg(test)]
    pub(crate) fn ok(message: impl Into<String>) -> Self {
        Self {
            healthy: true,
            message: message.into(),
            details: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn failed(message: impl Into<String>, details: impl Into<String>) -> Self {
        Self {
            healthy: false,
            message: message.into(),
            details: Some(details.into()),
        }
    }

    pub(crate) fn is_healthy(&self) -> bool {
        self.healthy
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LanguageAdapterHealthContext {
    pub(crate) language_adapter_pack_id: String,
    pub(crate) runtime: String,
    pub(crate) registered: bool,
    pub(crate) migrated: bool,
    pub(crate) pending_migration_count: usize,
}

impl LanguageAdapterHealthContext {
    pub(crate) fn new(
        language_adapter_pack_id: impl Into<String>,
        runtime: impl Into<String>,
        registered: bool,
        migrated: bool,
        pending_migration_count: usize,
    ) -> Self {
        Self {
            language_adapter_pack_id: language_adapter_pack_id.into(),
            runtime: runtime.into(),
            registered,
            migrated,
            pending_migration_count,
        }
    }

    #[cfg(test)]
    pub(crate) fn has_pending_migrations(&self) -> bool {
        self.pending_migration_count > 0 && !self.migrated
    }
}
