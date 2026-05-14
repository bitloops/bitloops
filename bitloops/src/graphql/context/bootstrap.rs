use super::{BLOB_HEALTHCHECK_KEY, DevqlGraphqlContext, DevqlSchemaMode};
use crate::api::{BackendHealth, BackendHealthKind, DashboardDbPools};
use crate::config::{StoreBackendConfig, resolve_store_backend_config_for_repo};
use crate::devql_transport::{index_repo_path_registry, load_repo_path_registry};
use crate::graphql::loaders::LoaderMetrics;
use crate::graphql::scope::SelectedRepository;
use crate::graphql::types::{
    HealthBackendStatus, HealthStatus, Repository, StorageAuthorityStatus,
};
use crate::host::devql::{
    DevqlConfig, RepoIdentity, build_capability_host, deterministic_uuid, resolve_repo_identity,
};
use crate::storage::blob::{BlobStore, create_blob_store_with_backend_for_repo};
use anyhow::{Result, bail};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::graphql::subscriptions::SubscriptionHub;

impl DevqlGraphqlContext {
    pub(crate) fn new(repo_root: std::path::PathBuf, db: DashboardDbPools) -> Self {
        Self::build(
            DevqlSchemaMode::Global,
            repo_root.clone(),
            None,
            repo_root,
            None,
            None,
            false,
            true,
            db,
        )
    }

    pub(crate) fn for_global_request(
        config_root: std::path::PathBuf,
        repo_root: std::path::PathBuf,
        repo_registry_path: Option<std::path::PathBuf>,
        db: DashboardDbPools,
    ) -> Self {
        Self::build(
            DevqlSchemaMode::Global,
            config_root,
            repo_registry_path,
            repo_root,
            None,
            None,
            false,
            false,
            db,
        )
    }

    pub(crate) fn for_slim_request(
        config_root: std::path::PathBuf,
        repo_root: std::path::PathBuf,
        branch_override: Option<String>,
        project_scope_override: Option<String>,
        repo_registry_path: Option<std::path::PathBuf>,
        request_scope_present: bool,
        db: DashboardDbPools,
    ) -> Self {
        Self::build(
            DevqlSchemaMode::Slim,
            config_root,
            repo_registry_path,
            repo_root,
            branch_override,
            project_scope_override,
            request_scope_present,
            false,
            db,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        schema_mode: DevqlSchemaMode,
        config_root: std::path::PathBuf,
        repo_registry_path: Option<std::path::PathBuf>,
        repo_root: std::path::PathBuf,
        branch_override: Option<String>,
        project_scope_override: Option<String>,
        request_scope_present: bool,
        allow_default_repository_selection_without_catalog: bool,
        db: DashboardDbPools,
    ) -> Self {
        let backend_config = resolve_store_backend_config_for_repo(&config_root).ok();
        let repo_identity = resolve_repo_identity(&repo_root)
            .unwrap_or_else(|_| fallback_repo_identity(repo_root.as_path()));
        let default_repository = SelectedRepository::new(
            repo_identity.repo_id.clone(),
            repo_identity.provider.clone(),
            repo_identity.organization.clone(),
            repo_identity.name.clone(),
            repo_identity.identity.clone(),
            Some(super::git_history::git_default_branch_name(
                repo_root.as_path(),
            )),
            Some(repo_root.clone()),
        );
        let (config, config_error) = match DevqlConfig::from_roots(
            config_root.clone(),
            repo_root.clone(),
            repo_identity.clone(),
        ) {
            Ok(config) => (Some(config), None),
            Err(err) => (None, Some(format!("{err:#}"))),
        };
        let blob_backend = backend_config
            .as_ref()
            .map(configured_blob_backend)
            .unwrap_or("unknown")
            .to_string();
        let (blob_store, blob_bootstrap_error) = match backend_config.as_ref() {
            Some(cfg) => match create_blob_store_with_backend_for_repo(&cfg.blobs, &config_root) {
                Ok(resolved) => {
                    let store: Arc<dyn BlobStore> = Arc::from(resolved.store);
                    (Some(store), None)
                }
                Err(err) => (None, Some(format!("{err:#}"))),
            },
            None => (
                None,
                Some("store backend configuration unavailable".to_string()),
            ),
        };
        let (capability_host, capability_host_bootstrap_error) =
            match build_capability_host(&repo_root, repo_identity.clone()) {
                Ok(host) => (Some(Arc::new(host)), None),
                Err(err) => (None, Some(format!("{err:#}"))),
            };

        Self {
            schema_mode,
            config_root,
            repo_registry_path,
            repo_root,
            branch_override,
            project_scope_override,
            request_scope_present,
            db,
            backend_config,
            config,
            config_error,
            default_repository,
            allow_default_repository_selection_without_catalog,
            repo_identity,
            blob_store,
            blob_backend,
            blob_bootstrap_error,
            capability_host,
            capability_host_bootstrap_error,
            loader_metrics: LoaderMetrics::default(),
            subscriptions: SubscriptionHub::new_arc(),
        }
    }

    pub(crate) async fn health_status(&self) -> HealthStatus {
        let health = self.db.health_check().await;
        let storage_authorities = self
            .backend_config
            .as_ref()
            .map(|cfg| {
                crate::host::db_status::collect_storage_authority_rows(
                    &self.config_root,
                    &self.repo_root,
                    cfg,
                )
                .into_iter()
                .map(|row| StorageAuthorityStatus::new(row.family, row.authority, row.backend))
                .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let blob = if let Some(store) = self.blob_store.as_ref() {
            let store = Arc::clone(store);
            let backend = self.blob_backend.clone();
            match tokio::task::spawn_blocking(move || store.exists(BLOB_HEALTHCHECK_KEY)).await {
                Ok(Ok(_)) => HealthBackendStatus::new(true, backend, "OK", "blob store reachable"),
                Ok(Err(err)) => {
                    HealthBackendStatus::new(false, backend, "FAIL", format!("{err:#}"))
                }
                Err(join_err) => HealthBackendStatus::new(
                    false,
                    backend,
                    "FAIL",
                    format!("blob health probe task failed: {join_err}"),
                ),
            }
        } else {
            HealthBackendStatus::new(
                false,
                self.blob_backend.clone(),
                "FAIL",
                self.blob_bootstrap_error
                    .clone()
                    .unwrap_or_else(|| "blob store unavailable".to_string()),
            )
        };
        HealthStatus {
            relational: map_backend_health(self.relational_backend_name(), health.relational),
            events: map_backend_health(self.events_backend_name(), health.events),
            blob,
            storage_authorities,
        }
    }

    pub(crate) async fn repository_for_name(&self, name: &str) -> Result<Repository> {
        let selection = self.resolve_repository_selection(name).await?;
        let repository = self.repository_from_selection(selection);

        Ok(repository)
    }

    pub(crate) async fn list_known_repositories(&self) -> Result<Vec<SelectedRepository>> {
        let mut repositories = self.load_known_repositories().await?;
        repositories.sort_by(|left, right| {
            left.name()
                .cmp(right.name())
                .then_with(|| left.provider().cmp(right.provider()))
                .then_with(|| left.organization().cmp(right.organization()))
        });

        Ok(repositories)
    }

    pub(crate) async fn resolve_repository_selection(
        &self,
        name: &str,
    ) -> Result<SelectedRepository> {
        let requested_name = name.trim();
        if requested_name.is_empty() {
            return Ok(self.default_repository.clone());
        }

        let repositories = self.list_known_repositories().await?;
        if repositories.is_empty() {
            if self.allow_default_repository_selection_without_catalog
                && matches_default_repository_selector(&self.default_repository, requested_name)
            {
                return Ok(self.default_repository.clone());
            }
            bail!("unknown repository `{requested_name}`");
        }

        if let Some(repository) = repositories
            .iter()
            .find(|repository| repository.repo_id() == requested_name)
        {
            return Ok(repository.clone());
        }
        if let Some(repository) = repositories
            .iter()
            .find(|repository| repository.identity() == requested_name)
        {
            return Ok(repository.clone());
        }

        let matches = repositories
            .iter()
            .filter(|repository| repository.name() == requested_name)
            .cloned()
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [repository] => Ok(repository.clone()),
            [] => bail!("unknown repository `{requested_name}`"),
            _ => bail!(
                "repository name `{requested_name}` is ambiguous; use the repo id or identity instead"
            ),
        }
    }

    async fn load_known_repositories(&self) -> Result<Vec<SelectedRepository>> {
        let sql = "SELECT repo_id, provider, organization, name, default_branch FROM repositories ORDER BY name ASC, provider ASC, organization ASC";
        let rows = match self.query_devql_sqlite_rows(sql).await {
            Ok(rows) => rows,
            Err(err) if is_missing_repositories_table_error(&err) => Vec::new(),
            Err(err) => return Err(err),
        };
        let repo_sync_roots = self.load_repo_sync_roots().await?;
        let registry = match self.repo_registry_path() {
            Some(path) => index_repo_path_registry(&load_repo_path_registry(path)?),
            None => Default::default(),
        };

        let mut repositories = BTreeMap::<String, SelectedRepository>::new();
        for row in rows {
            let repo_id = required_row_string(&row, "repo_id")?;
            let repo_root = registry
                .get(&repo_id)
                .map(|entry| entry.repo_root.clone())
                .or_else(|| repo_sync_roots.get(&repo_id).cloned())
                .or_else(|| {
                    (repo_id == self.default_repository.repo_id())
                        .then(|| self.default_repository.repo_root().cloned())
                        .flatten()
                });
            let provider = required_row_string(&row, "provider")?;
            let organization = required_row_string(&row, "organization")?;
            let name = required_row_string(&row, "name")?;
            repositories.insert(
                repo_id.clone(),
                SelectedRepository::new(
                    repo_id,
                    provider.clone(),
                    organization.clone(),
                    name.clone(),
                    format!("{provider}://{organization}/{name}"),
                    optional_row_string(&row, "default_branch"),
                    repo_root,
                ),
            );
        }

        for entry in registry.into_values() {
            repositories
                .entry(entry.repo_id.clone())
                .or_insert_with(|| {
                    SelectedRepository::new(
                        entry.repo_id,
                        entry.provider,
                        entry.organisation,
                        entry.name,
                        entry.identity,
                        entry.last_branch,
                        Some(entry.repo_root),
                    )
                });
        }

        Ok(repositories.into_values().collect())
    }

    async fn load_repo_sync_roots(&self) -> Result<BTreeMap<String, PathBuf>> {
        let sql = "SELECT repo_id, repo_root FROM repo_sync_state";
        let rows = match self.query_devql_sqlite_rows(sql).await {
            Ok(rows) => rows,
            Err(err) if is_missing_repo_sync_state_table_error(&err) => return Ok(BTreeMap::new()),
            Err(err) => return Err(err),
        };

        let mut repo_sync_roots = BTreeMap::new();
        for row in rows {
            let repo_id = required_row_string_in_table(&row, "repo_id", "repo_sync_state")?;
            let repo_root = required_row_string_in_table(&row, "repo_root", "repo_sync_state")?;
            repo_sync_roots.insert(repo_id, PathBuf::from(repo_root));
        }

        Ok(repo_sync_roots)
    }

    fn repository_from_selection(&self, selection: SelectedRepository) -> Repository {
        Repository::new(
            selection.name(),
            selection.provider(),
            selection.organization(),
        )
        .with_scope(crate::graphql::ResolverScope::default().with_repository(selection))
    }

    fn relational_backend_name(&self) -> &'static str {
        match self.backend_config.as_ref() {
            Some(cfg) if cfg.relational.has_postgres() => "postgres",
            Some(_) => "sqlite",
            None => "unknown",
        }
    }

    fn events_backend_name(&self) -> &'static str {
        match self.backend_config.as_ref() {
            Some(cfg) if cfg.events.has_clickhouse() => "clickhouse",
            Some(_) => "duckdb",
            None => "unknown",
        }
    }
}

fn required_row_string(row: &Value, key: &str) -> Result<String> {
    required_row_string_in_table(row, key, "repositories")
}

fn required_row_string_in_table(row: &Value, key: &str, table: &str) -> Result<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing `{key}` in {table} row"))
}

fn optional_row_string(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn is_missing_repositories_table_error(err: &anyhow::Error) -> bool {
    let message = format!("{err:#}");
    message.contains("no such table: repositories")
        || message.contains("relation \"repositories\" does not exist")
}

fn is_missing_repo_sync_state_table_error(err: &anyhow::Error) -> bool {
    let message = format!("{err:#}");
    message.contains("no such table: repo_sync_state")
        || message.contains("relation \"repo_sync_state\" does not exist")
}

fn map_backend_health(backend: &str, health: BackendHealth) -> HealthBackendStatus {
    HealthBackendStatus::new(
        health.kind == BackendHealthKind::Ok,
        backend,
        health.status_label(),
        health.detail,
    )
}

fn configured_blob_backend(cfg: &StoreBackendConfig) -> &'static str {
    if cfg.blobs.s3_bucket.is_some() && cfg.blobs.gcs_bucket.is_some() {
        "invalid"
    } else if cfg.blobs.s3_bucket.is_some() {
        "s3"
    } else if cfg.blobs.gcs_bucket.is_some() {
        "gcs"
    } else {
        "local"
    }
}

fn fallback_repo_identity(repo_root: &Path) -> RepoIdentity {
    let name = repo_root
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("repo")
        .to_string();
    let identity = format!("local://local/{name}");
    RepoIdentity {
        provider: "local".to_string(),
        organization: "local".to_string(),
        name,
        identity: identity.clone(),
        repo_id: deterministic_uuid(&identity),
    }
}

fn matches_default_repository_selector(
    repository: &SelectedRepository,
    requested_name: &str,
) -> bool {
    repository.repo_id() == requested_name
        || repository.identity() == requested_name
        || repository.name() == requested_name
}

#[cfg(test)]
mod tests {
    use super::DevqlGraphqlContext;
    use crate::api::DashboardDbPools;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn health_status_exposes_storage_authorities_by_data_family() {
        let temp = tempdir().expect("temp dir");
        let config_root = temp.path().join("daemon");
        let repo_root = temp.path().join("repo");
        fs::create_dir_all(&config_root).expect("create config root");
        fs::create_dir_all(&repo_root).expect("create repo root");
        fs::write(
            config_root.join("config.toml"),
            r#"
[stores.relational]
postgres_dsn = "postgres://bitloops:secret@postgres.internal:5432/bitloops"

[stores.events]
clickhouse_url = "http://clickhouse.internal:8123"
clickhouse_database = "analytics"

[stores.blob]
s3_bucket = "bitloops-shared"
s3_region = "eu-central-1"
"#,
        )
        .expect("write daemon config");

        let ctx = DevqlGraphqlContext::for_global_request(
            config_root,
            repo_root,
            None,
            DashboardDbPools::default(),
        );

        let health = ctx.health_status().await;

        assert_eq!(health.storage_authorities.len(), 6);
        let shared_relational = health
            .storage_authorities
            .iter()
            .find(|row| row.family == "relational shared")
            .expect("shared relational authority");
        assert_eq!(shared_relational.authority, "shared");
        assert_eq!(shared_relational.backend, "postgres");

        let current_projection = health
            .storage_authorities
            .iter()
            .find(|row| row.family == "relational current")
            .expect("current projection authority");
        assert_eq!(current_projection.authority, "workspace-local");
        assert_eq!(current_projection.backend, "sqlite");

        let project_blobs = health
            .storage_authorities
            .iter()
            .find(|row| row.family == "blob project/knowledge")
            .expect("project blob authority");
        assert_eq!(project_blobs.authority, "shared");
        assert_eq!(project_blobs.backend, "s3");
    }
}
