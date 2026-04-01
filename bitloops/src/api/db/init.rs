use super::clickhouse::ClickHousePool;
use super::config::DashboardDbConfig;
use super::duckdb::DuckDbPool;
use super::health::{BackendHealth, DashboardDbHealth};
use super::postgres::PostgresPool;
use super::sqlite::SqlitePool;
use super::{DashboardDbInit, DashboardDbPools, HEALTH_CHECK_TIMEOUT, POSTGRES_POOL_SIZE};

pub(super) async fn init_dashboard_db() -> DashboardDbInit {
    let cfg = match DashboardDbConfig::from_env() {
        Ok(cfg) => cfg,
        Err(err) => {
            let failure = BackendHealth::fail(format!("{err:#}"));
            return DashboardDbInit {
                pools: DashboardDbPools::default(),
                startup_health: DashboardDbHealth::with_compat_fields(
                    failure.clone(),
                    failure,
                    false,
                    false,
                ),
            };
        }
    };

    let has_postgres = cfg.backends.relational.has_postgres();
    let has_clickhouse = cfg.backends.events.has_clickhouse();

    let mut pools = DashboardDbPools {
        has_postgres,
        has_clickhouse,
        postgres: None,
        sqlite: None,
        clickhouse: None,
        duckdb: None,
        duckdb_path: None,
    };

    let relational_health = init_relational_backend(&cfg, &mut pools).await;
    let events_health = init_events_backend(&cfg, &mut pools, has_clickhouse).await;

    DashboardDbInit {
        startup_health: DashboardDbHealth::with_compat_fields(
            relational_health,
            events_health,
            has_postgres,
            has_clickhouse,
        ),
        pools,
    }
}

async fn init_relational_backend(
    cfg: &DashboardDbConfig,
    pools: &mut DashboardDbPools,
) -> BackendHealth {
    if let Some(dsn) = cfg.backends.relational.postgres_dsn.clone() {
        match PostgresPool::connect(&dsn, POSTGRES_POOL_SIZE).await {
            Ok(pool) => match pool.ping().await {
                Ok(value) => {
                    pools.postgres = Some(pool);
                    BackendHealth::ok(format!("SELECT 1 => {value}"))
                }
                Err(err) => BackendHealth::fail(format!("{err:#}")),
            },
            Err(err) => BackendHealth::fail(format!("{err:#}")),
        }
    } else {
        match cfg.sqlite_db_path() {
            Ok(db_path) => {
                let db_label = db_path.display().to_string();
                match SqlitePool::connect(db_path).await {
                    Ok(pool) => match pool.ping().await {
                        Ok(value) => {
                            pools.sqlite = Some(pool);
                            BackendHealth::ok(format!("SELECT 1 => {value} ({db_label})"))
                        }
                        Err(err) => BackendHealth::fail(format!("{err:#}")),
                    },
                    Err(err) => BackendHealth::fail(format!("{err:#}")),
                }
            }
            Err(err) => BackendHealth::fail(format!("{err:#}")),
        }
    }
}

async fn init_events_backend(
    cfg: &DashboardDbConfig,
    pools: &mut DashboardDbPools,
    has_clickhouse: bool,
) -> BackendHealth {
    if has_clickhouse {
        let ch_cfg = cfg.clickhouse_config();
        match ClickHousePool::build(&ch_cfg) {
            Ok(pool) => match pool.ping().await {
                Ok(value) => {
                    pools.clickhouse = Some(pool);
                    BackendHealth::ok(format!("SELECT 1 => {value}"))
                }
                Err(err) => BackendHealth::fail(format!("{err:#}")),
            },
            Err(err) => BackendHealth::fail(format!("{err:#}")),
        }
    } else {
        let duckdb_path = cfg.duckdb_path();
        pools.duckdb_path = Some(duckdb_path.clone());
        match tokio::time::timeout(HEALTH_CHECK_TIMEOUT, DuckDbPool::connect(duckdb_path)).await {
            Ok(Ok(pool)) => match tokio::time::timeout(HEALTH_CHECK_TIMEOUT, pool.ping()).await {
                Ok(Ok(value)) => BackendHealth::ok(format!("SELECT 1 => {value}")),
                Ok(Err(err)) => BackendHealth::fail(format!("{err:#}")),
                Err(_) => BackendHealth::fail("DuckDB health query timed out".to_string()),
            },
            Ok(Err(err)) => BackendHealth::fail(format!("{err:#}")),
            Err(_) => BackendHealth::fail("DuckDB connect timed out".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::db::BackendHealthKind;
    use crate::config::{
        BlobStorageConfig, EventsBackendConfig, RelationalBackendConfig, StoreBackendConfig,
    };
    use tempfile::tempdir;

    #[tokio::test]
    async fn init_events_backend_keeps_duckdb_path_without_holding_pool_open() {
        let dir = tempdir().expect("temp dir");
        let duckdb_path = dir.path().join("events.duckdb");
        let conn = duckdb::Connection::open(&duckdb_path).expect("create duckdb file");
        conn.execute_batch("SELECT 1;")
            .expect("seed duckdb for startup health");
        drop(conn);

        let cfg = DashboardDbConfig {
            backends: StoreBackendConfig {
                relational: RelationalBackendConfig {
                    sqlite_path: Some(dir.path().join("relational.db").to_string_lossy().into()),
                    postgres_dsn: None,
                },
                events: EventsBackendConfig {
                    duckdb_path: Some(duckdb_path.to_string_lossy().into()),
                    clickhouse_url: None,
                    clickhouse_user: None,
                    clickhouse_password: None,
                    clickhouse_database: None,
                },
                blobs: BlobStorageConfig {
                    local_path: Some(dir.path().join("blob").to_string_lossy().into()),
                    s3_bucket: None,
                    s3_region: None,
                    s3_access_key_id: None,
                    s3_secret_access_key: None,
                    gcs_bucket: None,
                    gcs_credentials_path: None,
                },
            },
        };
        let mut pools = DashboardDbPools::default();

        let health = init_events_backend(&cfg, &mut pools, false).await;

        assert_eq!(health.kind, BackendHealthKind::Ok);
        assert!(
            pools.duckdb.is_none(),
            "daemon startup should not retain a DuckDB pool"
        );
        assert_eq!(pools.duckdb_path.as_deref(), Some(duckdb_path.as_path()));
    }
}
