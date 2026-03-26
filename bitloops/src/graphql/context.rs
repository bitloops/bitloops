mod bootstrap;
mod commit_checkpoints;
mod events;
mod git_history;
mod repository_graph;
mod temporal_scope;

use crate::api::DashboardDbPools;
use crate::config::StoreBackendConfig;
use crate::host::capability_host::DevqlCapabilityHost;
use crate::host::devql::{DevqlConfig, RepoIdentity};
use crate::storage::blob::BlobStore;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::loaders::LoaderMetrics;

const BLOB_HEALTHCHECK_KEY: &str = "__bitloops/graphql/healthcheck";
const GIT_FIELD_SEPARATOR: char = '\u{1f}';
const GIT_RECORD_SEPARATOR: char = '\u{1e}';
const GRAPHQL_GIT_SCAN_LIMIT: usize = 5_000;
const GRAPHQL_DEVQL_SCAN_LIMIT: usize = i32::MAX as usize;

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
    capability_host: Option<Arc<Mutex<DevqlCapabilityHost>>>,
    capability_host_bootstrap_error: Option<String>,
    loader_metrics: LoaderMetrics,
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
    pub(crate) fn loader_metrics(&self) -> &LoaderMetrics {
        &self.loader_metrics
    }

    #[cfg(test)]
    pub(crate) fn loader_metrics_snapshot(&self) -> super::loaders::LoaderMetricsSnapshot {
        self.loader_metrics.snapshot()
    }
}
