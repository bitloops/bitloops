mod bootstrap;
mod commit_checkpoints;
mod enrichment;
mod events;
mod git_history;
mod knowledge;
mod repository_graph;
mod temporal_scope;

use crate::api::DashboardDbPools;
use crate::config::StoreBackendConfig;
use crate::host::capability_host::DevqlCapabilityHost;
use crate::host::devql::{DevqlConfig, RepoIdentity};
use crate::storage::blob::BlobStore;
use anyhow::{Result, anyhow};
use serde_json::Value;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::loaders::LoaderMetrics;
use super::subscriptions::SubscriptionHub;

const BLOB_HEALTHCHECK_KEY: &str = "__bitloops/graphql/healthcheck";
const GIT_FIELD_SEPARATOR: char = '\u{1f}';
const GIT_RECORD_SEPARATOR: char = '\u{1e}';
const GRAPHQL_GIT_SCAN_LIMIT: usize = 5_000;

#[allow(dead_code)]
#[derive(Clone)]
pub(crate) struct DevqlGraphqlContext {
    repo_root: PathBuf,
    db: DashboardDbPools,
    backend_config: Option<StoreBackendConfig>,
    config: Option<DevqlConfig>,
    config_error: Option<String>,
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
            .field("repo_root", &self.repo_root)
            .field("db", &self.db)
            .field("backend_config", &self.backend_config)
            .field("config_ready", &self.config.is_some())
            .field("config_error", &self.config_error)
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

    pub(crate) fn repo_id(&self) -> &str {
        self.repo_identity.repo_id.as_str()
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

    pub(crate) fn repo_name(&self) -> &str {
        self.repo_identity.name.as_str()
    }

    pub(crate) fn subscriptions(&self) -> Arc<SubscriptionHub> {
        Arc::clone(&self.subscriptions)
    }

    #[cfg(test)]
    pub(crate) fn loader_metrics_snapshot(&self) -> super::loaders::LoaderMetricsSnapshot {
        self.loader_metrics.snapshot()
    }
}
