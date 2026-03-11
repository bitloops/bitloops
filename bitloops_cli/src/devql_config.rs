//! Shared DevQL config file parsing and path.
//! Used by both the CLI (devql commands) and the dashboard server so supported keys and defaults stay in sync.

use serde_json::{Map, Value};
use std::env;
use std::fs;
use std::path::PathBuf;

/// Relative path from user home to the DevQL config file (e.g. `~/.bitloops/config.json`).
pub const DEVQL_CONFIG_RELATIVE_PATH: &str = ".bitloops/config.json";

#[derive(Debug, Clone, Default)]
pub struct DevqlFileConfig {
    pub(crate) pg_dsn: Option<String>,
    pub(crate) clickhouse_url: Option<String>,
    pub(crate) clickhouse_user: Option<String>,
    pub(crate) clickhouse_password: Option<String>,
    pub(crate) clickhouse_database: Option<String>,
    pub(crate) semantic_provider: Option<String>,
    pub(crate) semantic_model: Option<String>,
    pub(crate) semantic_api_key: Option<String>,
    pub(crate) semantic_base_url: Option<String>,
}

impl DevqlFileConfig {
    /// Load config from `$HOME/.bitloops/config.json` (or `$USERPROFILE` on Windows).
    /// Returns default if the file is missing or invalid.
    pub fn load() -> Self {
        let Some(path) = user_home_config_path() else {
            return Self::default();
        };

        let data = match fs::read(&path) {
            Ok(data) => data,
            Err(_) => return Self::default(),
        };
        let value: Value = match serde_json::from_slice(&data) {
            Ok(value) => value,
            Err(_) => return Self::default(),
        };
        Self::from_json_value(&value)
    }

    /// Parse config from a JSON value (e.g. from file or tests).
    /// Looks for a `"devql"` object first, then falls back to the root object.
    pub fn from_json_value(value: &Value) -> Self {
        let root_opt = value
            .get("devql")
            .and_then(Value::as_object)
            .or_else(|| value.as_object());
        let Some(root) = root_opt else {
            return Self::default();
        };

        Self {
            pg_dsn: read_any_string(root, &["postgres_dsn", "pg_dsn", "BITLOOPS_DEVQL_PG_DSN"]),
            clickhouse_url: read_any_string(
                root,
                &["clickhouse_url", "ch_url", "BITLOOPS_DEVQL_CH_URL"],
            ),
            clickhouse_user: read_any_string(
                root,
                &["clickhouse_user", "ch_user", "BITLOOPS_DEVQL_CH_USER"],
            ),
            clickhouse_password: read_any_string(
                root,
                &[
                    "clickhouse_password",
                    "ch_password",
                    "BITLOOPS_DEVQL_CH_PASSWORD",
                ],
            ),
            clickhouse_database: read_any_string(
                root,
                &[
                    "clickhouse_database",
                    "ch_database",
                    "BITLOOPS_DEVQL_CH_DATABASE",
                ],
            ),
            semantic_provider: read_any_string(
                root,
                &["semantic_provider", "BITLOOPS_DEVQL_SEMANTIC_PROVIDER"],
            ),
            semantic_model: read_any_string(
                root,
                &["semantic_model", "BITLOOPS_DEVQL_SEMANTIC_MODEL"],
            ),
            semantic_api_key: read_any_string(
                root,
                &["semantic_api_key", "BITLOOPS_DEVQL_SEMANTIC_API_KEY"],
            ),
            semantic_base_url: read_any_string(
                root,
                &["semantic_base_url", "BITLOOPS_DEVQL_SEMANTIC_BASE_URL"],
            ),
        }
    }
}

fn user_home_config_path() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .map(|home| home.join(DEVQL_CONFIG_RELATIVE_PATH))
}

fn read_any_string(root: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = root.get(*key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}
