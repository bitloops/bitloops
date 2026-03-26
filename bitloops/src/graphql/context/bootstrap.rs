use super::{BLOB_HEALTHCHECK_KEY, DevqlGraphqlContext};
use crate::api::{BackendHealth, BackendHealthKind, DashboardDbPools};
use crate::config::{StoreBackendConfig, resolve_store_backend_config_for_repo};
use crate::graphql::loaders::LoaderMetrics;
use crate::graphql::types::{HealthBackendStatus, HealthStatus, Repository};
use crate::host::devql::{
    DevqlConfig, RepoIdentity, build_capability_host, deterministic_uuid, resolve_repo_identity,
};
use crate::storage::blob::{BlobStore, create_blob_store_with_backend_for_repo};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::graphql::subscriptions::SubscriptionHub;

impl DevqlGraphqlContext {
    pub(crate) fn new(repo_root: std::path::PathBuf, db: DashboardDbPools) -> Self {
        let backend_config = resolve_store_backend_config_for_repo(&repo_root).ok();
        let repo_identity = resolve_repo_identity(&repo_root)
            .unwrap_or_else(|_| fallback_repo_identity(repo_root.as_path()));
        let (config, config_error) =
            match DevqlConfig::from_env(repo_root.clone(), repo_identity.clone()) {
                Ok(config) => (Some(config), None),
                Err(err) => (None, Some(format!("{err:#}"))),
            };
        let blob_backend = backend_config
            .as_ref()
            .map(configured_blob_backend)
            .unwrap_or("unknown")
            .to_string();
        let (blob_store, blob_bootstrap_error) = match backend_config.as_ref() {
            Some(cfg) => match create_blob_store_with_backend_for_repo(&cfg.blobs, &repo_root) {
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
                Ok(host) => (Some(Arc::new(Mutex::new(host))), None),
                Err(err) => (None, Some(format!("{err:#}"))),
            };

        Self {
            repo_root,
            db,
            backend_config,
            config,
            config_error,
            repo_identity,
            blob_store,
            blob_backend,
            blob_bootstrap_error,
            capability_host,
            capability_host_bootstrap_error,
            loader_metrics: LoaderMetrics::default(),
            subscriptions: SubscriptionHub::shared(),
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

    pub(crate) fn repository_for_name(&self, name: &str) -> Repository {
        let requested_name = name.trim();
        let name = if requested_name.is_empty() {
            self.repo_identity.name.as_str()
        } else {
            requested_name
        };

        Repository::new(
            name,
            self.repo_identity.provider.as_str(),
            self.repo_identity.organization.as_str(),
        )
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
