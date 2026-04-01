use super::clickhouse::ClickHousePool;
use super::config::DashboardDbConfig;
use super::duckdb::DuckDbPool;
use super::health::{BackendHealth, DashboardDbHealth};
use super::postgres::PostgresPool;
use super::sqlite::SqlitePool;
use super::{DashboardDbInit, DashboardDbPools, POSTGRES_POOL_SIZE};

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
        match DuckDbPool::connect(duckdb_path).await {
            Ok(pool) => match pool.ping().await {
                Ok(value) => {
                    pools.duckdb = Some(pool);
                    BackendHealth::ok(format!("SELECT 1 => {value}"))
                }
                Err(err) => BackendHealth::fail(format!("{err:#}")),
            },
            Err(err) => BackendHealth::fail(format!("{err:#}")),
        }
    }
}
