//! Configuration: store backend parsing, path resolution, and project settings.
//! Used by both the CLI and the dashboard server so supported keys and defaults stay in sync.

mod constants;
mod daemon_config;
mod file_config;
mod repo_policy;
mod resolve;
pub mod settings;
mod store_config_utils;
mod types;
pub mod unified_config;

pub use constants::BITLOOPS_CONFIG_RELATIVE_PATH;
pub use daemon_config::{
    DaemonCliSettings, LoadedDaemonSettings, bootstrap_default_daemon_environment,
    default_daemon_config_exists, default_daemon_config_path, ensure_daemon_config_exists,
    load_daemon_settings, persist_daemon_cli_settings, persist_dashboard_tls_hint,
};
pub use repo_policy::{
    ImportedKnowledgeConfig, REPO_POLICY_FILE_NAME, REPO_POLICY_LOCAL_FILE_NAME,
    RepoPolicySnapshot, discover_repo_policy, discover_repo_policy_optional,
};
pub use resolve::{
    resolve_blob_local_path, resolve_blob_local_path_for_repo, resolve_dashboard_config,
    resolve_dashboard_config_for_repo, resolve_duckdb_db_path_for_repo, resolve_provider_config,
    resolve_provider_config_for_repo, resolve_sqlite_db_path, resolve_sqlite_db_path_for_repo,
    resolve_store_backend_config, resolve_store_backend_config_for_repo,
    resolve_store_embedding_config, resolve_store_embedding_config_for_repo,
    resolve_store_semantic_config, resolve_store_semantic_config_for_repo,
    resolve_watch_runtime_config_for_repo,
};
pub use types::{
    AtlassianProviderConfig, BlobStorageConfig, DashboardFileConfig, DashboardLocalDashboardConfig,
    EventsBackendConfig, GithubProviderConfig, ProviderConfig, RelationalBackendConfig,
    StoreBackendConfig, StoreEmbeddingConfig, StoreFileConfig, StoreSemanticConfig,
    WatchFileConfig, WatchRuntimeConfig,
};

#[cfg(test)]
pub(crate) use constants::{
    ENV_EMBEDDING_API_KEY, ENV_EMBEDDING_MODEL, ENV_EMBEDDING_PROVIDER, ENV_SEMANTIC_API_KEY,
    ENV_SEMANTIC_BASE_URL, ENV_SEMANTIC_MODEL, ENV_SEMANTIC_PROVIDER, ENV_WATCH_DEBOUNCE_MS,
    ENV_WATCH_POLL_FALLBACK_MS,
};
#[cfg(test)]
pub(crate) use resolve::{
    resolve_provider_config_for_tests, resolve_store_backend_config_for_tests,
    resolve_store_embedding_config_for_tests, resolve_store_semantic_config_for_tests,
    resolve_watch_runtime_config_for_tests,
};

#[cfg(test)]
mod store_config_tests;
#[cfg(test)]
mod unified_config_tests;
#[cfg(test)]
mod unified_consumer_tests;
