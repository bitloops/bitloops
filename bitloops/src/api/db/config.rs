use anyhow::Result;
use reqwest::Url;
use std::path::PathBuf;

use crate::config::{StoreBackendConfig, resolve_store_backend_config};

#[derive(Debug, Clone)]
pub(super) struct ClickHouseConfig {
    pub(super) url: String,
    pub(super) database: String,
    pub(super) user: Option<String>,
    pub(super) password: Option<String>,
}

impl ClickHouseConfig {
    pub(super) fn endpoint(&self) -> String {
        let base = self.url.trim_end_matches('/');
        let Ok(mut url) = Url::parse(base) else {
            return format!("{base}/?database={}", self.database);
        };
        url.query_pairs_mut()
            .append_pair("database", &self.database);
        url.to_string()
    }
}

#[derive(Debug, Clone)]
pub(super) struct DashboardDbConfig {
    pub(super) backends: StoreBackendConfig,
}

impl DashboardDbConfig {
    pub(super) fn from_env() -> Result<Self> {
        Ok(Self {
            backends: resolve_store_backend_config()?,
        })
    }

    pub(super) fn clickhouse_config(&self) -> ClickHouseConfig {
        ClickHouseConfig {
            url: self
                .backends
                .events
                .clickhouse_url
                .clone()
                .unwrap_or_else(|| "http://localhost:8123".to_string()),
            database: self
                .backends
                .events
                .clickhouse_database
                .clone()
                .unwrap_or_else(|| "default".to_string()),
            user: self.backends.events.clickhouse_user.clone(),
            password: self.backends.events.clickhouse_password.clone(),
        }
    }

    pub(super) fn duckdb_path(&self) -> PathBuf {
        self.backends.events.duckdb_path_or_default()
    }

    pub(super) fn sqlite_db_path(&self) -> Result<PathBuf> {
        self.backends.relational.resolve_sqlite_db_path()
    }
}
