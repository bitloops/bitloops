use crate::api::DashboardDbPools;
use crate::config::{StoreBackendConfig, resolve_store_backend_config_for_repo};
use crate::host::devql::{RepoIdentity, resolve_repo_identity};
use std::path::PathBuf;

use super::types::Repository;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct DevqlGraphqlContext {
    repo_root: PathBuf,
    db: DashboardDbPools,
    backend_config: Option<StoreBackendConfig>,
    repo_identity: Option<RepoIdentity>,
}

impl DevqlGraphqlContext {
    pub(crate) fn new(repo_root: PathBuf, db: DashboardDbPools) -> Self {
        let backend_config = resolve_store_backend_config_for_repo(&repo_root).ok();
        let repo_identity = resolve_repo_identity(&repo_root).ok();
        Self {
            repo_root,
            db,
            backend_config,
            repo_identity,
        }
    }

    pub(crate) fn repository_for_name(&self, name: &str) -> Repository {
        let requested_name = name.trim();
        let name = if requested_name.is_empty() {
            self.repo_identity
                .as_ref()
                .map(|repo| repo.name.as_str())
                .unwrap_or("repo")
        } else {
            requested_name
        };

        let provider = self
            .repo_identity
            .as_ref()
            .map(|repo| repo.provider.as_str())
            .unwrap_or("local");
        let organization = self
            .repo_identity
            .as_ref()
            .map(|repo| repo.organization.as_str())
            .unwrap_or("local");

        Repository::new(name, provider, organization)
    }
}
