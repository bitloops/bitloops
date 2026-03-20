use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguagePackContext {
    pub repo_root: PathBuf,
    pub repo_id: String,
    pub commit_sha: Option<String>,
}

impl LanguagePackContext {
    pub fn new(repo_root: PathBuf, repo_id: impl Into<String>, commit_sha: Option<String>) -> Self {
        Self {
            repo_root,
            repo_id: repo_id.into(),
            commit_sha,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityExecutionContext {
    pub repo_root: PathBuf,
    pub repo_id: String,
    pub commit_sha: Option<String>,
    pub capability_pack_id: String,
    pub stage_id: String,
}

impl CapabilityExecutionContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo_root: PathBuf,
        repo_id: impl Into<String>,
        commit_sha: Option<String>,
        capability_pack_id: impl Into<String>,
        stage_id: impl Into<String>,
    ) -> Self {
        Self {
            repo_root,
            repo_id: repo_id.into(),
            commit_sha,
            capability_pack_id: capability_pack_id.into(),
            stage_id: stage_id.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityIngestContext {
    pub repo_root: PathBuf,
    pub repo_id: String,
    pub commit_sha: Option<String>,
    pub capability_pack_id: String,
    pub ingester_id: String,
}

impl CapabilityIngestContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo_root: PathBuf,
        repo_id: impl Into<String>,
        commit_sha: Option<String>,
        capability_pack_id: impl Into<String>,
        ingester_id: impl Into<String>,
    ) -> Self {
        Self {
            repo_root,
            repo_id: repo_id.into(),
            commit_sha,
            capability_pack_id: capability_pack_id.into(),
            ingester_id: ingester_id.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityMigrationContext {
    pub capability_pack_id: String,
    pub migration_id: String,
    pub order: u32,
    pub description: String,
}

impl CapabilityMigrationContext {
    pub fn new(
        capability_pack_id: impl Into<String>,
        migration_id: impl Into<String>,
        order: u32,
        description: impl Into<String>,
    ) -> Self {
        Self {
            capability_pack_id: capability_pack_id.into(),
            migration_id: migration_id.into(),
            order,
            description: description.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityHealthContext {
    pub capability_pack_id: String,
    pub runtime: String,
    pub registered: bool,
    pub migrated: bool,
    pub pending_migration_count: usize,
}

impl CapabilityHealthContext {
    pub fn new(
        capability_pack_id: impl Into<String>,
        runtime: impl Into<String>,
        registered: bool,
        migrated: bool,
        pending_migration_count: usize,
    ) -> Self {
        Self {
            capability_pack_id: capability_pack_id.into(),
            runtime: runtime.into(),
            registered,
            migrated,
            pending_migration_count,
        }
    }

    pub fn has_pending_migrations(&self) -> bool {
        self.pending_migration_count > 0 && !self.migrated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_contexts_are_constructible_and_preserve_identity() {
        let repo_root = PathBuf::from("/tmp/repo");

        let language =
            LanguagePackContext::new(repo_root.clone(), "repo-id", Some("commit-sha".to_string()));
        assert_eq!(language.repo_id, "repo-id");
        assert_eq!(language.commit_sha.as_deref(), Some("commit-sha"));

        let execution = CapabilityExecutionContext::new(
            repo_root.clone(),
            "repo-id",
            Some("commit-sha".to_string()),
            "semantic-clones-pack",
            "semantic-clones",
        );
        assert_eq!(execution.capability_pack_id, "semantic-clones-pack");
        assert_eq!(execution.stage_id, "semantic-clones");

        let ingest = CapabilityIngestContext::new(
            repo_root,
            "repo-id",
            Some("commit-sha".to_string()),
            "knowledge-pack",
            "knowledge-ingester",
        );
        assert_eq!(ingest.capability_pack_id, "knowledge-pack");
        assert_eq!(ingest.ingester_id, "knowledge-ingester");

        let migration = CapabilityMigrationContext::new(
            "knowledge-pack",
            "001",
            1,
            "initial capability schema setup",
        );
        assert_eq!(migration.capability_pack_id, "knowledge-pack");
        assert_eq!(migration.migration_id, "001");
        assert_eq!(migration.order, 1);

        let health = CapabilityHealthContext::new("knowledge-pack", "local-cli", true, false, 1);
        assert_eq!(health.capability_pack_id, "knowledge-pack");
        assert_eq!(health.runtime, "local-cli");
        assert!(health.has_pending_migrations());
    }
}
