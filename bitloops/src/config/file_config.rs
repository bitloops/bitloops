use serde_json::Value;
use std::fs;
use std::path::Path;

use super::constants::*;
use super::store_config_utils::{
    current_repo_root_or_cwd, load_repo_config_value, read_any_bool, read_any_string,
    read_any_string_opt, read_any_u64, read_any_u64_opt,
};
use super::types::{DashboardFileConfig, StoreFileConfig, WatchFileConfig};

impl StoreFileConfig {
    /// Load config from `<repo>/.bitloops/config.json`.
    /// Returns default if the file is missing or invalid.
    pub fn load() -> Self {
        let repo_root = current_repo_root_or_cwd();
        Self::load_for_repo(&repo_root)
    }

    /// Load config from `<repo_root>/.bitloops/config.json`.
    pub fn load_for_repo(repo_root: &Path) -> Self {
        load_repo_config_value(repo_root)
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
            relational_provider: read_any_string_opt(relational, &["provider"])
                .or_else(|| read_any_string(root, &["relational_provider"])),
            sqlite_path: read_any_string_opt(relational, &["sqlite_path", "path"])
                .or_else(|| read_any_string(root, &["sqlite_path"])),
            pg_dsn: read_any_string_opt(relational, &["postgres_dsn", "pg_dsn"])
                .or_else(|| read_any_string(root, &["postgres_dsn", "pg_dsn"])),
            events_provider: read_any_string_opt(events, &["provider"])
                .or_else(|| read_any_string(root, &["events_provider", "event_provider"])),
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
            embedding_provider: read_any_string(
                root,
                &["embedding_provider", ENV_EMBEDDING_PROVIDER],
            ),
            embedding_model: read_any_string(root, &["embedding_model", ENV_EMBEDDING_MODEL]),
            embedding_api_key: read_any_string(root, &["embedding_api_key", ENV_EMBEDDING_API_KEY]),
            blob_provider: read_any_string_opt(blobs, &["provider"])
                .or_else(|| read_any_string(root, &["blob_provider"])),
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
    /// Load dashboard config from `<repo>/.bitloops/config.json`.
    /// Returns default if the file is missing or invalid.
    pub fn load() -> Self {
        let repo_root = current_repo_root_or_cwd();
        load_repo_config_value(&repo_root)
            .map(|value| Self::from_json_value(&value))
            .unwrap_or_default()
    }

    /// Parse dashboard config from a JSON value.
    /// Reads the nested `dashboard` object.
    pub fn from_json_value(value: &Value) -> Self {
        let Some(root) = value.get(DASHBOARD_CONFIG_KEY).and_then(Value::as_object) else {
            return Self::default();
        };

        Self {
            use_bitloops_local: read_any_bool(root, &[DASHBOARD_USE_BITLOOPS_LOCAL_KEY]),
        }
    }
}

impl WatchFileConfig {
    pub fn load_for_repo(repo_root: &Path) -> Self {
        let mut merged = Self::default();

        let json_path = repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
        if let Ok(data) = fs::read(&json_path)
            && let Ok(value) = serde_json::from_slice::<Value>(&data)
        {
            merged = Self::from_json_value(&value);
        }

        let toml_path = repo_root.join(BITLOOPS_CONFIG_TOML_RELATIVE_PATH);
        if let Ok(data) = fs::read_to_string(&toml_path) {
            merged.merge(Self::from_toml_str(&data));
        }

        merged
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

    pub fn from_toml_str(input: &str) -> Self {
        let mut cfg = Self::default();
        let mut section: Vec<String> = Vec::new();

        for raw_line in input.lines() {
            let line = raw_line.split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }

            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1]
                    .split('.')
                    .map(|part| part.trim().trim_matches('"').to_string())
                    .filter(|part| !part.is_empty())
                    .collect();
                continue;
            }

            let Some((raw_key, raw_value)) = line.split_once('=') else {
                continue;
            };

            let key = raw_key.trim().trim_matches('"');
            let value = raw_value.trim().trim_matches('"').trim_matches('\'');
            let in_watch_scope = section.is_empty()
                || section.as_slice() == [WATCH_CONFIG_KEY]
                || section.as_slice() == [DEVQL_CONFIG_KEY]
                || section.as_slice() == [DEVQL_CONFIG_KEY, WATCH_CONFIG_KEY];
            if !in_watch_scope {
                continue;
            }

            match key {
                WATCH_DEBOUNCE_MS_KEY => cfg.watch_debounce_ms = value.parse::<u64>().ok(),
                WATCH_POLL_FALLBACK_MS_KEY => {
                    cfg.watch_poll_fallback_ms = value.parse::<u64>().ok()
                }
                _ => {}
            }
        }

        cfg
    }

    fn merge(&mut self, other: Self) {
        if other.watch_debounce_ms.is_some() {
            self.watch_debounce_ms = other.watch_debounce_ms;
        }
        if other.watch_poll_fallback_ms.is_some() {
            self.watch_poll_fallback_ms = other.watch_poll_fallback_ms;
        }
    }
}
