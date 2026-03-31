use serde_json::Value;
use std::path::Path;

use super::constants::*;
use super::daemon_config::load_daemon_settings;
use super::repo_policy::discover_repo_policy_optional;
use super::store_config_utils::{
    current_repo_root_or_cwd, read_any_bool, read_any_string, read_any_string_opt, read_any_u64,
    read_any_u64_opt,
};
use super::types::{
    DashboardFileConfig, DashboardLocalDashboardConfig, StoreFileConfig, WatchFileConfig,
};

impl StoreFileConfig {
    /// Load store config from the active daemon configuration.
    pub fn load() -> Self {
        Self::load_for_repo(&current_repo_root_or_cwd())
    }

    /// Load global daemon store config.
    pub fn load_for_repo(_repo_root: &Path) -> Self {
        load_daemon_settings(None)
            .ok()
            .and_then(|loaded| loaded.settings.stores)
            .map(|value| Self::from_json_value(&value))
            .unwrap_or_default()
    }

    /// Parse config from a JSON value (e.g. from file or tests).
    /// Reads the `stores` object when present, otherwise falls back to root.
    pub fn from_json_value(value: &Value) -> Self {
        let root_opt = value
            .get(STORES_CONFIG_KEY)
            .and_then(Value::as_object)
            .or_else(|| value.as_object());
        let Some(root) = root_opt else {
            return Self::default();
        };

        let relational = root.get(RELATIONAL_CONFIG_KEY).and_then(Value::as_object);
        let events = root
            .get(EVENT_CONFIG_KEY)
            .or_else(|| root.get(EVENTS_CONFIG_KEY))
            .and_then(Value::as_object);
        let blobs = root
            .get(BLOB_CONFIG_KEY)
            .or_else(|| root.get(BLOBS_CONFIG_KEY))
            .and_then(Value::as_object);
        let semantic = root.get(SEMANTIC_CONFIG_KEY).and_then(Value::as_object);

        Self {
            sqlite_path: read_any_string_opt(relational, &["sqlite_path", "path"])
                .or_else(|| read_any_string(root, &["sqlite_path"])),
            pg_dsn: read_any_string_opt(relational, &["postgres_dsn", "pg_dsn"])
                .or_else(|| read_any_string(root, &["postgres_dsn", "pg_dsn"])),
            duckdb_path: read_any_string_opt(events, &["duckdb_path", "path"])
                .or_else(|| read_any_string(root, &["duckdb_path"])),
            clickhouse_url: read_any_string_opt(events, &["clickhouse_url"])
                .or_else(|| read_any_string(root, &["clickhouse_url"])),
            clickhouse_user: read_any_string_opt(events, &["clickhouse_user"])
                .or_else(|| read_any_string(root, &["clickhouse_user"])),
            clickhouse_password: read_any_string_opt(events, &["clickhouse_password"])
                .or_else(|| read_any_string(root, &["clickhouse_password"])),
            clickhouse_database: read_any_string_opt(events, &["clickhouse_database"])
                .or_else(|| read_any_string(root, &["clickhouse_database"])),
            semantic_provider: read_any_string_opt(semantic, &["provider", "semantic_provider"])
                .or_else(|| read_any_string(root, &["semantic_provider"])),
            semantic_model: read_any_string_opt(semantic, &["model", "semantic_model"])
                .or_else(|| read_any_string(root, &["semantic_model"])),
            semantic_api_key: read_any_string_opt(semantic, &["api_key", "semantic_api_key"])
                .or_else(|| read_any_string(root, &["semantic_api_key"])),
            semantic_base_url: read_any_string_opt(semantic, &["base_url", "semantic_base_url"])
                .or_else(|| read_any_string(root, &["semantic_base_url"])),
<<<<<<< Updated upstream
            embedding_provider: None,
            embedding_model: None,
            embedding_api_key: None,
            embedding_cache_dir: None,
=======
<<<<<<< Updated upstream
            embedding_provider: read_any_string(
                root,
                &["embedding_provider", ENV_EMBEDDING_PROVIDER],
            ),
            embedding_model: read_any_string(root, &["embedding_model", ENV_EMBEDDING_MODEL]),
            embedding_api_key: read_any_string(root, &["embedding_api_key", ENV_EMBEDDING_API_KEY]),
            embedding_cache_dir: read_any_string(root, &["embedding_cache_dir"]),
=======
>>>>>>> Stashed changes
>>>>>>> Stashed changes
            blob_local_path: read_any_string_opt(blobs, &["local_path"])
                .or_else(|| read_any_string(root, &["blob_local_path"])),
            blob_s3_bucket: read_any_string_opt(blobs, &["s3_bucket"])
                .or_else(|| read_any_string(root, &["blob_s3_bucket"])),
            blob_s3_region: read_any_string_opt(blobs, &["s3_region"])
                .or_else(|| read_any_string(root, &["blob_s3_region"])),
            blob_s3_access_key_id: read_any_string_opt(blobs, &["s3_access_key_id"])
                .or_else(|| read_any_string(root, &["blob_s3_access_key_id"])),
            blob_s3_secret_access_key: read_any_string_opt(blobs, &["s3_secret_access_key"])
                .or_else(|| read_any_string(root, &["blob_s3_secret_access_key"])),
            blob_gcs_bucket: read_any_string_opt(blobs, &["gcs_bucket"])
                .or_else(|| read_any_string(root, &["blob_gcs_bucket"])),
            blob_gcs_credentials_path: read_any_string_opt(blobs, &["gcs_credentials_path"])
                .or_else(|| read_any_string(root, &["blob_gcs_credentials_path"])),
        }
    }
}

impl DashboardFileConfig {
    /// Load dashboard config from the active daemon configuration.
    pub fn load() -> Self {
        load_daemon_settings(None)
            .ok()
            .and_then(|loaded| loaded.settings.dashboard)
            .map(|value| Self::from_json_value(&value))
            .unwrap_or_default()
    }

    /// Parse dashboard config from a JSON value.
    /// Accepts either the nested `dashboard` object or a root object containing it.
    pub fn from_json_value(value: &Value) -> Self {
        let Some(root) = value
            .get(DASHBOARD_CONFIG_KEY)
            .and_then(Value::as_object)
            .or_else(|| value.as_object())
        else {
            return Self::default();
        };

        let local_dashboard = root
            .get(DASHBOARD_LOCAL_DASHBOARD_KEY)
            .and_then(Value::as_object)
            .map(|local| DashboardLocalDashboardConfig {
                tls: read_any_bool(local, &[DASHBOARD_LOCAL_DASHBOARD_TLS_KEY]),
            })
            .filter(|local| local.tls.is_some());

        Self {
            local_dashboard,
            bundle_dir: read_any_string(root, &["bundle_dir"]).map(Into::into),
        }
    }
}

impl WatchFileConfig {
    pub fn load_for_repo(repo_root: &Path) -> Self {
        discover_repo_policy_optional(repo_root)
            .ok()
            .map(|policy| {
                let mut map = serde_json::Map::new();
                map.insert(WATCH_CONFIG_KEY.into(), policy.watch);
                Self::from_json_value(&Value::Object(map))
            })
            .unwrap_or_default()
    }

    pub fn from_json_value(value: &Value) -> Self {
        let root = value.as_object();
        let watch = root
            .and_then(|map| map.get(WATCH_CONFIG_KEY))
            .and_then(Value::as_object);
        let devql = root
            .and_then(|map| map.get(DEVQL_CONFIG_KEY))
            .and_then(Value::as_object);
        let devql_watch = devql
            .and_then(|map| map.get(WATCH_CONFIG_KEY))
            .and_then(Value::as_object);

        Self {
            watch_debounce_ms: read_any_u64_opt(watch, &[WATCH_DEBOUNCE_MS_KEY])
                .or_else(|| read_any_u64_opt(devql_watch, &[WATCH_DEBOUNCE_MS_KEY]))
                .or_else(|| read_any_u64_opt(devql, &[WATCH_DEBOUNCE_MS_KEY]))
                .or_else(|| root.and_then(|map| read_any_u64(map, &[WATCH_DEBOUNCE_MS_KEY]))),
            watch_poll_fallback_ms: read_any_u64_opt(watch, &[WATCH_POLL_FALLBACK_MS_KEY])
                .or_else(|| read_any_u64_opt(devql_watch, &[WATCH_POLL_FALLBACK_MS_KEY]))
                .or_else(|| read_any_u64_opt(devql, &[WATCH_POLL_FALLBACK_MS_KEY]))
                .or_else(|| root.and_then(|map| read_any_u64(map, &[WATCH_POLL_FALLBACK_MS_KEY]))),
        }
    }
}
