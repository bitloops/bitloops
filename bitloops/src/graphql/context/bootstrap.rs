use super::{BLOB_HEALTHCHECK_KEY, DevqlGraphqlContext, DevqlSchemaMode};
use crate::api::{BackendHealth, BackendHealthKind, DashboardDbPools};
use crate::config::{StoreBackendConfig, resolve_store_backend_config_for_repo};
use crate::devql_transport::{index_repo_path_registry, load_repo_path_registry};
use crate::graphql::loaders::LoaderMetrics;
use crate::graphql::scope::SelectedRepository;
use crate::graphql::types::{HealthBackendStatus, HealthStatus, Repository};
use crate::host::devql::{
    DevqlConfig, RepoIdentity, build_capability_host, deterministic_uuid, resolve_repo_identity,
};
use anyhow::{Result, bail};
use crate::storage::blob::{BlobStore, create_blob_store_with_backend_for_repo};
use serde_json::Value;
use std::path::Path;
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
            db,
        )
    }

    fn build(
        schema_mode: DevqlSchemaMode,
        config_root: std::path::PathBuf,
        repo_registry_path: Option<std::path::PathBuf>,
        repo_root: std::path::PathBuf,
        branch_override: Option<String>,
        project_scope_override: Option<String>,
        request_scope_present: bool,
        db: DashboardDbPools,
    ) -> Self {
        let backend_config = resolve_store_backend_config_for_repo(&config_root).ok();
        let repo_identity =
            resolve_repo_identity(&repo_root).unwrap_or_else(|_| fallback_repo_identity(repo_root.as_path()));
        let default_repository = SelectedRepository::new(
            repo_identity.repo_id.clone(),
            repo_identity.provider.clone(),
            repo_identity.organization.clone(),
            repo_identity.name.clone(),
            repo_identity.identity.clone(),
            Some(super::git_history::git_default_branch_name(repo_root.as_path())),
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
        }
    }

    pub(crate) async fn repository_for_name(&self, name: &str) -> Result<Repository> {
        let selection = self.resolve_repository_selection(name).await?;
        let repository = Repository::new(
            selection.name(),
            selection.provider(),
            selection.organization(),
        )
        .with_scope(crate::graphql::ResolverScope::default().with_repository(selection));

        Ok(repository)
    }

    pub(crate) async fn resolve_repository_selection(
        &self,
        name: &str,
    ) -> Result<SelectedRepository> {
        let requested_name = name.trim();
        if requested_name.is_empty() {
            return Ok(self.default_repository.clone());
        }

        let repositories = self.load_known_repositories().await?;
        if repositories.is_empty() {
            if requested_name == self.default_repository.name()
                || requested_name == self.default_repository.identity()
                || requested_name == self.default_repository.repo_id()
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
            Err(err) if is_missing_repositories_table_error(&err) => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };
        let registry = match self.repo_registry_path() {
            Some(path) => index_repo_path_registry(&load_repo_path_registry(path)?),
            None => Default::default(),
        };

        let mut repositories = Vec::with_capacity(rows.len());
        for row in rows {
            let repo_id = required_row_string(&row, "repo_id")?;
            let repo_root = registry
                .get(&repo_id)
                .map(|entry| entry.repo_root.clone())
                .or_else(|| {
                    (repo_id == self.default_repository.repo_id())
                        .then(|| self.default_repository.repo_root().cloned())
                        .flatten()
                });
            let provider = required_row_string(&row, "provider")?;
            let organization = required_row_string(&row, "organization")?;
            let name = required_row_string(&row, "name")?;
            repositories.push(SelectedRepository::new(
                repo_id,
                provider.clone(),
                organization.clone(),
                name.clone(),
                format!("{provider}://{organization}/{name}"),
                optional_row_string(&row, "default_branch"),
                repo_root,
            ));
        }
        Ok(repositories)
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
    row.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing `{key}` in repositories row"))
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
