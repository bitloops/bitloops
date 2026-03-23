//! Configuration: store backend parsing, path resolution, and project settings.
//! Used by both the CLI and the dashboard server so supported keys and defaults stay in sync.

mod constants;
mod file_config;
mod resolve;
pub mod settings;
mod store_config_utils;
mod types;
pub mod unified_config;

pub use constants::BITLOOPS_CONFIG_RELATIVE_PATH;
pub use resolve::{
    dashboard_use_bitloops_local, resolve_blob_local_path, resolve_blob_local_path_for_repo,
    resolve_duckdb_db_path_for_repo, resolve_provider_config, resolve_provider_config_for_repo,
    resolve_sqlite_db_path, resolve_sqlite_db_path_for_repo, resolve_store_backend_config,
    resolve_store_backend_config_for_repo, resolve_store_embedding_config,
    resolve_store_semantic_config, resolve_watch_runtime_config_for_repo,
};
pub use types::{
    AtlassianProviderConfig, BlobStorageConfig, BlobStorageProvider, DashboardFileConfig,
    EventsBackendConfig, EventsProvider, GithubProviderConfig, ProviderConfig,
    RelationalBackendConfig, RelationalProvider, StoreBackendConfig, StoreEmbeddingConfig,
    StoreFileConfig, StoreSemanticConfig, WatchFileConfig, WatchRuntimeConfig,
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
