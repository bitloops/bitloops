use anyhow::Result;
use serde_json::{Value, json};
use std::fmt;
use std::path::Path;
use std::time::Instant;
use tokio::time::timeout;

use super::HEALTH_CHECK_TIMEOUT;
use super::clickhouse::ClickHousePool;
use super::duckdb::DuckDbPool;
use super::health::{BackendHealth, DashboardDbHealth};
use super::postgres::PostgresPool;
use super::sqlite::{SqlitePool, sqlite_exec_path};

#[derive(Clone, Default)]
pub(crate) struct DashboardDbPools {
    pub(super) has_postgres: bool,
    pub(super) has_clickhouse: bool,
    pub(super) postgres: Option<PostgresPool>,
    pub(super) sqlite: Option<SqlitePool>,
    pub(super) clickhouse: Option<ClickHousePool>,
    pub(super) duckdb: Option<DuckDbPool>,
}

impl fmt::Debug for DashboardDbPools {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DashboardDbPools")
            .field("has_postgres", &self.has_postgres)
            .field("has_clickhouse", &self.has_clickhouse)
            .field("postgres_enabled", &self.postgres.is_some())
            .field("sqlite_enabled", &self.sqlite.is_some())
            .field("clickhouse_enabled", &self.clickhouse.is_some())
            .field("duckdb_enabled", &self.duckdb.is_some())
            .finish()
    }
}

impl DashboardDbPools {
    pub(crate) async fn health_check(&self) -> DashboardDbHealth {
        let postgres_pool = self.postgres.clone();
        let sqlite_pool = self.sqlite.clone();
        let clickhouse_pool = self.clickhouse.as_ref();
        let duckdb_pool = self.duckdb.as_ref();

        let relational_fut = async move {
            match (postgres_pool, sqlite_pool) {
                (Some(pool), _) => match timeout(HEALTH_CHECK_TIMEOUT, pool.ping()).await {
                    Ok(Ok(value)) => BackendHealth::ok(format!("SELECT 1 => {value}")),
                    Ok(Err(err)) => BackendHealth::fail(format!("{err:#}")),
                    Err(_) => BackendHealth::fail("health check timed out".to_string()),
                },
                (None, Some(pool)) => match timeout(HEALTH_CHECK_TIMEOUT, pool.ping()).await {
                    Ok(Ok(value)) => BackendHealth::ok(format!("SELECT 1 => {value}")),
                    Ok(Err(err)) => BackendHealth::fail(format!("{err:#}")),
                    Err(_) => BackendHealth::fail("health check timed out".to_string()),
                },
                (None, None) => BackendHealth::skip("not configured"),
            }
        };
        let clickhouse_fut = async move {
            match clickhouse_pool {
                Some(pool) => match timeout(HEALTH_CHECK_TIMEOUT, pool.ping()).await {
                    Ok(Ok(value)) => BackendHealth::ok(format!("SELECT 1 => {value}")),
                    Ok(Err(err)) => BackendHealth::fail(format!("{err:#}")),
                    Err(_) => BackendHealth::fail("health check timed out".to_string()),
                },
                None => BackendHealth::skip("not configured"),
            }
        };
        let duckdb_fut = async move {
            match duckdb_pool {
                Some(pool) => match timeout(HEALTH_CHECK_TIMEOUT, pool.ping()).await {
                    Ok(Ok(value)) => BackendHealth::ok(format!("SELECT 1 => {value}")),
                    Ok(Err(err)) => BackendHealth::fail(format!("{err:#}")),
                    Err(_) => BackendHealth::fail("health check timed out".to_string()),
                },
                None => BackendHealth::skip("not configured"),
            }
        };

        let (relational_backend, clickhouse, duckdb) =
            tokio::join!(relational_fut, clickhouse_fut, duckdb_fut);
        let events = if self.has_clickhouse {
            clickhouse.clone()
        } else {
            duckdb.clone()
        };

        DashboardDbHealth::with_compat_fields(
            relational_backend,
            events,
            self.has_postgres,
            self.has_clickhouse,
        )
    }

    pub(crate) async fn query_sqlite_rows(&self, path: &Path, sql: &str) -> Result<Vec<Value>> {
        let started = Instant::now();
        let pooled = self.sqlite.as_ref().is_some_and(|pool| pool.path() == path);
        let result = if let Some(pool) = self.sqlite.as_ref()
            && pool.path() == path
        {
            pool.query_rows(sql).await
        } else {
            crate::host::devql::sqlite_query_rows_path(path, sql).await
        };
        record_backend_stage(
            "server.db.sqlite.query_rows",
            started.elapsed(),
            json!({
                "path": path.display().to_string(),
                "pooled": pooled,
                "rows": result.as_ref().map(|rows| rows.len()).ok(),
                "sqlBytes": sql.len(),
                "error": result.as_ref().err().map(|err| format!("{err:#}")),
            }),
        );
        result
    }

    pub(crate) async fn execute_sqlite_batch(&self, path: &Path, sql: &str) -> Result<()> {
        let started = Instant::now();
        let pooled = self.sqlite.as_ref().is_some_and(|pool| pool.path() == path);
        let result = if let Some(pool) = self.sqlite.as_ref()
            && pool.path() == path
        {
            pool.execute_batch(sql).await
        } else {
            sqlite_exec_path(path, sql).await
        };
        record_backend_stage(
            "server.db.sqlite.execute_batch",
            started.elapsed(),
            json!({
                "path": path.display().to_string(),
                "pooled": pooled,
                "sqlBytes": sql.len(),
                "error": result.as_ref().err().map(|err| format!("{err:#}")),
            }),
        );
        result
    }

    pub(crate) async fn query_duckdb_rows(&self, path: &Path, sql: &str) -> Result<Vec<Value>> {
        let started = Instant::now();
        let pooled = self.duckdb.as_ref().is_some_and(|pool| pool.path() == path);
        let result = if let Some(pool) = self.duckdb.as_ref()
            && pool.path() == path
        {
            pool.query_rows(sql).await
        } else {
            crate::host::devql::duckdb_query_rows_path(path, sql).await
        };
        record_backend_stage(
            "server.db.duckdb.query_rows",
            started.elapsed(),
            json!({
                "path": path.display().to_string(),
                "pooled": pooled,
                "rows": result.as_ref().map(|rows| rows.len()).ok(),
                "sqlBytes": sql.len(),
                "error": result.as_ref().err().map(|err| format!("{err:#}")),
            }),
        );
        result
    }

    pub(crate) async fn query_clickhouse_data(
        &self,
        cfg: &crate::host::devql::DevqlConfig,
        sql: &str,
    ) -> Result<Value> {
        let started = Instant::now();
        let pooled = self.clickhouse.is_some();
        let result = if let Some(pool) = self.clickhouse.as_ref() {
            pool.query_data(sql).await
        } else {
            crate::host::devql::clickhouse_query_data(cfg, sql).await
        };
        record_backend_stage(
            "server.db.clickhouse.query_data",
            started.elapsed(),
            json!({
                "pooled": pooled,
                "sqlBytes": sql.len(),
                "responseKind": result.as_ref().ok().map(json_value_kind),
                "rowCount": result.as_ref().ok().and_then(|value| value.as_array().map(|rows| rows.len())),
                "error": result.as_ref().err().map(|err| format!("{err:#}")),
            }),
        );
        result
    }

    #[cfg(test)]
    pub(super) fn for_test_backends(
        sqlite: Option<SqlitePool>,
        duckdb: Option<DuckDbPool>,
    ) -> Self {
        Self {
            has_postgres: false,
            has_clickhouse: false,
            postgres: None,
            sqlite,
            clickhouse: None,
            duckdb,
        }
    }
}

fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn record_backend_stage(stage: &str, elapsed: tokio::time::Duration, detail: Value) {
    crate::devql_timing::record_current_stage(stage, elapsed, detail);
}
