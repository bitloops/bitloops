use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub(super) struct DuckDbPool {
    path: PathBuf,
    connection: Arc<Mutex<duckdb::Connection>>,
}

impl fmt::Debug for DuckDbPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DuckDbPool")
            .field("path", &self.path.display().to_string())
            .finish()
    }
}

impl DuckDbPool {
    pub(super) async fn connect(path: PathBuf) -> Result<Self> {
        let connect_path = path.clone();
        let connection =
            tokio::task::spawn_blocking(move || open_duckdb_connection_existing(&connect_path))
                .await
                .context("joining DuckDB connect task")??;
        let pool = Self {
            path,
            connection: Arc::new(Mutex::new(connection)),
        };
        let _ = pool.ping().await?;
        Ok(pool)
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    async fn with_connection<T>(
        &self,
        operation: impl FnOnce(&duckdb::Connection) -> Result<T> + Send + 'static,
    ) -> Result<T>
    where
        T: Send + 'static,
    {
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || -> Result<T> {
            let conn = connection
                .lock()
                .map_err(|err| anyhow!("locking DuckDB dashboard connection: {err}"))?;
            operation(&conn)
        })
        .await
        .context("joining DuckDB connection task")?
    }

    pub(super) async fn ping(&self) -> Result<i32> {
        self.with_connection(|conn| {
            let mut stmt = conn
                .prepare("SELECT 1")
                .context("preparing DuckDB health query")?;
            let mut rows = stmt.query([]).context("executing DuckDB health query")?;
            let row = rows
                .next()
                .context("iterating DuckDB health query rows")?
                .ok_or_else(|| anyhow!("DuckDB health query returned no rows"))?;
            let value: i32 = row.get(0).context("reading DuckDB health query result")?;
            Ok(value)
        })
        .await
    }

    pub(super) async fn query_rows(&self, sql: &str) -> Result<Vec<Value>> {
        let sql = sql.to_string();
        self.with_connection(move |conn| duckdb_query_rows_with_connection(conn, &sql))
            .await
    }
}

pub(super) fn ensure_duckdb_file_exists(path: &Path) -> Result<()> {
    if path.is_file() {
        return Ok(());
    }

    bail!(
        "DuckDB database file not found at {}. Run `bitloops init` to create and initialise stores.",
        path.display()
    );
}

pub(super) fn open_duckdb_connection_existing(path: &Path) -> Result<duckdb::Connection> {
    ensure_duckdb_file_exists(path)?;
    duckdb::Connection::open_with_flags(
        path,
        duckdb::Config::default().access_mode(duckdb::AccessMode::ReadOnly)?,
    )
    .with_context(|| format!("opening DuckDB events database at {}", path.display()))
}

pub(super) fn duckdb_query_rows_with_connection(
    conn: &duckdb::Connection,
    sql: &str,
) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(sql).context("preparing DuckDB query")?;
    let mut rows = stmt.query([]).context("executing DuckDB query")?;
    let column_names = rows
        .as_ref()
        .map(|statement| statement.column_names())
        .unwrap_or_default();
    let mut out = Vec::new();

    while let Some(row) = rows.next().context("iterating DuckDB query rows")? {
        let mut obj = serde_json::Map::new();
        for (idx, column_name) in column_names.iter().enumerate() {
            let value_ref = row.get_ref(idx).with_context(|| {
                format!("reading DuckDB value for column index {idx} (`{column_name}`)")
            })?;
            let owned: duckdb::types::Value = value_ref.to_owned();
            obj.insert(
                column_name.clone(),
                crate::host::devql::duckdb_value_to_json(owned),
            );
        }
        out.push(Value::Object(obj));
    }

    Ok(out)
}
