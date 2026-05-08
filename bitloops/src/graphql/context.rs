mod architecture_graph;
mod bootstrap;
mod commit_checkpoints;
mod enrichment;
mod events;
mod git_history;
mod historical_context;
mod http;
mod interactions;
mod knowledge;
mod navigation_context;
mod repository_graph;
mod temporal_scope;

use crate::api::DashboardDbPools;
use crate::config::StoreBackendConfig;
use crate::config::resolve_store_backend_config_for_repo;
use crate::host::capability_host::DevqlCapabilityHost;
use crate::host::devql::{DevqlConfig, RelationalStorage, RepoIdentity};
use crate::storage::blob::BlobStore;
use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::loaders::LoaderMetrics;
use super::scope::SelectedRepository;
use super::subscriptions::SubscriptionHub;

#[allow(unused_imports)]
pub(crate) use historical_context::HistoricalContextSelectionInput;

const BLOB_HEALTHCHECK_KEY: &str = "__bitloops/graphql/healthcheck";
const GIT_FIELD_SEPARATOR: char = '\u{1f}';
const GIT_RECORD_SEPARATOR: char = '\u{1e}';
const GRAPHQL_GIT_SCAN_LIMIT: usize = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DevqlSchemaMode {
    Slim,
    Global,
}

#[allow(dead_code)]
#[derive(Clone)]
pub(crate) struct DevqlGraphqlContext {
    schema_mode: DevqlSchemaMode,
    config_root: PathBuf,
    repo_registry_path: Option<PathBuf>,
    repo_root: PathBuf,
    branch_override: Option<String>,
    project_scope_override: Option<String>,
    request_scope_present: bool,
    db: DashboardDbPools,
    backend_config: Option<StoreBackendConfig>,
    config: Option<DevqlConfig>,
    config_error: Option<String>,
    default_repository: SelectedRepository,
    allow_default_repository_selection_without_catalog: bool,
    repo_identity: RepoIdentity,
    blob_store: Option<Arc<dyn BlobStore>>,
    blob_backend: String,
    blob_bootstrap_error: Option<String>,
    capability_host: Option<Arc<DevqlCapabilityHost>>,
    capability_host_bootstrap_error: Option<String>,
    loader_metrics: LoaderMetrics,
    subscriptions: Arc<SubscriptionHub>,
}

impl fmt::Debug for DevqlGraphqlContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DevqlGraphqlContext")
            .field("schema_mode", &self.schema_mode)
            .field("config_root", &self.config_root)
            .field("repo_registry_path", &self.repo_registry_path)
            .field("repo_root", &self.repo_root)
            .field("branch_override", &self.branch_override)
            .field("project_scope_override", &self.project_scope_override)
            .field("request_scope_present", &self.request_scope_present)
            .field("db", &self.db)
            .field("backend_config", &self.backend_config)
            .field("config_ready", &self.config.is_some())
            .field("config_error", &self.config_error)
            .field("default_repository", &self.default_repository)
            .field(
                "allow_default_repository_selection_without_catalog",
                &self.allow_default_repository_selection_without_catalog,
            )
            .field("repo_identity", &self.repo_identity)
            .field("blob_backend", &self.blob_backend)
            .field("blob_ready", &self.blob_store.is_some())
            .field("blob_bootstrap_error", &self.blob_bootstrap_error)
            .field("capability_host_ready", &self.capability_host.is_some())
            .field(
                "capability_host_bootstrap_error",
                &self.capability_host_bootstrap_error,
            )
            .field("loader_metrics", &self.loader_metrics)
            .finish()
    }
}

impl DevqlGraphqlContext {
    pub(crate) async fn query_sqlite_rows_at_path(
        &self,
        path: &Path,
        sql: &str,
    ) -> Result<Vec<Value>> {
        self.db.query_sqlite_rows(path, sql).await
    }

    pub(crate) async fn query_devql_sqlite_rows(&self, sql: &str) -> Result<Vec<Value>> {
        let sqlite_path = self.devql_sqlite_path()?;
        self.query_sqlite_rows_at_path(&sqlite_path, sql).await
    }

    pub(crate) async fn execute_sqlite_batch_at_path(&self, path: &Path, sql: &str) -> Result<()> {
        self.db.execute_sqlite_batch(path, sql).await
    }

    pub(crate) async fn query_duckdb_rows_at_path(
        &self,
        path: &Path,
        sql: &str,
    ) -> Result<Vec<Value>> {
        self.db.query_duckdb_rows(path, sql).await
    }

    pub(crate) async fn query_clickhouse_data(&self, sql: &str) -> Result<Value> {
        let cfg = self.devql_config()?;
        self.db.query_clickhouse_data(&cfg, sql).await
    }

    pub(crate) fn loader_metrics(&self) -> &LoaderMetrics {
        &self.loader_metrics
    }

    pub(crate) fn devql_config(&self) -> Result<DevqlConfig> {
        self.config.clone().ok_or_else(|| {
            anyhow!(
                "{}",
                self.config_error
                    .clone()
                    .unwrap_or_else(|| "DevQL configuration unavailable".to_string())
            )
        })
    }

    pub(crate) async fn open_relational_storage(&self, command: &str) -> Result<RelationalStorage> {
        let cfg = self.devql_config()?;
        let backends = resolve_store_backend_config_for_repo(&cfg.repo_root)
            .with_context(|| format!("resolving backend config for `{command}`"))?;
        RelationalStorage::connect(&cfg, &backends.relational, command).await
    }

    pub(crate) fn daemon_config_path(&self) -> PathBuf {
        self.config_root
            .join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH)
    }

    pub(crate) fn repo_id(&self) -> &str {
        self.repo_identity.repo_id.as_str()
    }

    pub(crate) fn slim_root_scope(&self) -> super::ResolverScope {
        let mut scope = super::ResolverScope::default();
        if self.request_scope_present {
            scope = scope.with_repository(self.default_repository.clone());
        }
        if let Some(branch_name) = self.branch_override.clone() {
            scope = scope.with_branch_name(branch_name);
        }
        if let Some(project_path) = self.project_scope_override.clone() {
            scope = scope.with_project_path(project_path);
        }
        scope
    }

    pub(crate) fn capability_host_arc(&self) -> Result<Arc<DevqlCapabilityHost>> {
        let Some(capability_host) = self.capability_host.as_ref() else {
            return Err(anyhow!(
                "{}",
                self.capability_host_bootstrap_error
                    .clone()
                    .unwrap_or_else(|| "capability host unavailable".to_string())
            ));
        };
        Ok(Arc::clone(capability_host))
    }

    pub(crate) fn require_slim_request_scope(&self) -> Result<()> {
        if self.schema_mode == DevqlSchemaMode::Slim && !self.request_scope_present {
            return Err(anyhow!(
                "the slim DevQL endpoint requires CLI repository scope; use `bitloops devql ...` or connect to `/devql/global`"
            ));
        }
        Ok(())
    }

    pub(crate) fn require_repo_write_scope(&self) -> Result<()> {
        if self.schema_mode == DevqlSchemaMode::Slim && self.request_scope_present {
            return Ok(());
        }
        Err(anyhow!(
            "repo-scoped DevQL mutations require CLI repository scope; use `bitloops devql ...` against `/devql`"
        ))
    }

    pub(crate) fn require_global_write_scope(&self) -> Result<()> {
        if self.schema_mode == DevqlSchemaMode::Global {
            return Ok(());
        }
        Err(anyhow!(
            "global DevQL mutations require the global daemon endpoint; use `/devql/global`"
        ))
    }

    pub(crate) fn repository_selection_for_scope(
        &self,
        scope: &super::ResolverScope,
    ) -> Result<SelectedRepository> {
        if let Some(repository) = scope.repository() {
            return Ok(repository.clone());
        }
        if self.schema_mode == DevqlSchemaMode::Slim {
            self.require_slim_request_scope()?;
        }
        Ok(self.default_repository.clone())
    }

    pub(crate) fn repo_id_for_scope(&self, scope: &super::ResolverScope) -> Result<String> {
        Ok(self
            .repository_selection_for_scope(scope)?
            .repo_id()
            .to_string())
    }

    pub(crate) fn repo_root_for_scope(&self, scope: &super::ResolverScope) -> Result<PathBuf> {
        let repository = self.repository_selection_for_scope(scope)?;
        repository.repo_root().cloned().ok_or_else(|| {
            anyhow!(
                "repo checkout unknown for `{}`; re-run a slim CLI query or ingest from that checkout to register its path",
                repository.name()
            )
        })
    }

    pub(crate) fn repo_name_for_scope(&self, scope: &super::ResolverScope) -> Result<String> {
        Ok(self
            .repository_selection_for_scope(scope)?
            .name()
            .to_string())
    }

    pub(crate) fn repo_registry_path(&self) -> Option<&Path> {
        self.repo_registry_path.as_deref()
    }

    pub(crate) fn subscriptions(&self) -> Arc<SubscriptionHub> {
        Arc::clone(&self.subscriptions)
    }

    pub(crate) fn with_subscription_hub(mut self, subscriptions: Arc<SubscriptionHub>) -> Self {
        self.subscriptions = subscriptions;
        self
    }

    #[cfg(test)]
    pub(crate) fn loader_metrics_snapshot(&self) -> super::loaders::LoaderMetricsSnapshot {
        self.loader_metrics.snapshot()
    }
}
