use super::*;
use crate::config::default_daemon_config_exists;

pub(crate) const RELATIONAL_SQLITE_LABEL: &str = "Relational (SQLite)";
pub(crate) const RELATIONAL_POSTGRES_LABEL: &str = "Relational (Postgres)";
pub(crate) const EVENTS_DUCKDB_LABEL: &str = "Events (DuckDB)";
pub(crate) const EVENTS_CLICKHOUSE_LABEL: &str = "Events (ClickHouse)";

pub async fn run_connection_status() -> Result<()> {
    if !default_daemon_config_exists()? {
        let rows = vec![
            DatabaseStatusRow {
                db: RELATIONAL_SQLITE_LABEL,
                status: DatabaseConnectionStatus::NotConfigured,
            },
            DatabaseStatusRow {
                db: EVENTS_DUCKDB_LABEL,
                status: DatabaseConnectionStatus::NotConfigured,
            },
        ];
        print_db_status_table(&rows);
        bail!(
            "Bitloops daemon has not been bootstrapped yet. Run `bitloops start --create-default-config` or `bitloops init --install-default-daemon`."
        );
    }

    let cfg = resolve_store_backend_config()?;
    let rows = collect_connection_status_rows(&cfg).await;

    print_db_status_table(&rows);

    let failures = rows.iter().filter(|row| row.status.is_failure()).count();
    if failures > 0 {
        bail!("{failures} backend connection check(s) failed");
    }

    Ok(())
}

pub(crate) async fn collect_connection_status_rows(
    cfg: &StoreBackendConfig,
) -> Vec<DatabaseStatusRow> {
    vec![
        DatabaseStatusRow {
            db: relational_status_label(&cfg.relational),
            status: relational_connection_status(&cfg.relational).await,
        },
        DatabaseStatusRow {
            db: events_status_label(&cfg.events),
            status: events_connection_status(&cfg.events).await,
        },
    ]
}

fn relational_status_label(cfg: &RelationalBackendConfig) -> &'static str {
    if cfg.has_postgres() {
        RELATIONAL_POSTGRES_LABEL
    } else {
        RELATIONAL_SQLITE_LABEL
    }
}

fn events_status_label(cfg: &EventsBackendConfig) -> &'static str {
    if cfg.has_clickhouse() {
        EVENTS_CLICKHOUSE_LABEL
    } else {
        EVENTS_DUCKDB_LABEL
    }
}

async fn relational_connection_status(cfg: &RelationalBackendConfig) -> DatabaseConnectionStatus {
    if let Some(dsn) = cfg.postgres_dsn.as_deref() {
        match check_postgres_connection(dsn).await {
            Ok(_) => DatabaseConnectionStatus::Connected,
            Err(err) => classify_connection_error(&err.to_string()),
        }
    } else {
        match cfg.resolve_sqlite_db_path() {
            Ok(path) => match check_sqlite_connection(&path).await {
                Ok(_) => DatabaseConnectionStatus::Connected,
                Err(err) => classify_connection_error(&err.to_string()),
            },
            Err(err) => classify_connection_error(&err.to_string()),
        }
    }
}

async fn events_connection_status(cfg: &EventsBackendConfig) -> DatabaseConnectionStatus {
    if cfg.has_clickhouse() {
        let clickhouse_url = cfg
            .clickhouse_url
            .clone()
            .unwrap_or_else(|| "http://localhost:8123".to_string());
        let clickhouse_database = cfg
            .clickhouse_database
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let endpoint = clickhouse_endpoint(&clickhouse_url, &clickhouse_database);
        match run_clickhouse_sql_http(
            &endpoint,
            cfg.clickhouse_user.as_deref(),
            cfg.clickhouse_password.as_deref(),
            "SELECT 1 FORMAT TabSeparated",
        )
        .await
        {
            Ok(_) => DatabaseConnectionStatus::Connected,
            Err(err) => classify_connection_error(&err.to_string()),
        }
    } else {
        let duckdb_path = cfg.duckdb_path_or_default();
        match check_duckdb_connection(&duckdb_path).await {
            Ok(_) => DatabaseConnectionStatus::Connected,
            Err(err) => classify_connection_error(&err.to_string()),
        }
    }
}

async fn check_postgres_connection(dsn: &str) -> Result<()> {
    let client = connect_postgres_client(dsn).await?;

    let row = tokio::time::timeout(Duration::from_secs(10), client.query_one("SELECT 1", &[]))
        .await
        .context("Postgres health query timeout after 10s")?
        .context("running Postgres health query `SELECT 1`")?;
    let value: i32 = row
        .try_get(0)
        .context("reading Postgres health query result")?;
    if value != 1 {
        bail!("unexpected Postgres health query result: {value}");
    }

    Ok(())
}

async fn check_sqlite_connection(path: &Path) -> Result<()> {
    let db_path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .with_context(|| format!("opening SQLite database at {}", db_path.display()))?;

        let value: i32 = conn
            .query_row("SELECT 1", [], |row| row.get(0))
            .context("running SQLite health query `SELECT 1`")?;
        if value != 1 {
            bail!("unexpected SQLite health query result: {value}");
        }

        Ok(())
    })
    .await
    .context("joining SQLite health query task")?
}

async fn check_duckdb_connection(path: &Path) -> Result<()> {
    let db_path = path.to_path_buf();
    let rows = crate::host::devql::duckdb_query_rows_path(&db_path, "SELECT 1 AS value").await?;
    let value = rows
        .first()
        .and_then(|row| row.get("value"))
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("DuckDB health query returned no rows"))?;
    if value != 1 {
        bail!("unexpected DuckDB health query result: {value}");
    }
    Ok(())
}

fn clickhouse_endpoint(url: &str, database: &str) -> String {
    let base = url.trim_end_matches('/');
    format!("{base}/?database={database}")
}
