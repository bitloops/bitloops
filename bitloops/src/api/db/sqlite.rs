use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub(super) struct SqlitePool {
    db_path: PathBuf,
    connection: Arc<Mutex<rusqlite::Connection>>,
}

impl fmt::Debug for SqlitePool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SqlitePool")
            .field("db_path", &self.db_path.display().to_string())
            .finish()
    }
}

impl SqlitePool {
    pub(super) async fn connect(db_path: PathBuf) -> Result<Self> {
        ensure_sqlite_file_exists(&db_path)?;
        let connect_path = db_path.clone();
        let connection = tokio::task::spawn_blocking(move || open_sqlite_connection(&connect_path))
            .await
            .context("joining SQLite connect task")??;
        let pool = Self {
            db_path,
            connection: Arc::new(Mutex::new(connection)),
        };
        let _ = pool.ping().await?;
        Ok(pool)
    }

    pub(super) fn path(&self) -> &Path {
        &self.db_path
    }

    async fn with_connection<T>(
        &self,
        operation: impl FnOnce(&rusqlite::Connection) -> Result<T> + Send + 'static,
    ) -> Result<T>
    where
        T: Send + 'static,
    {
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || -> Result<T> {
            let conn = connection
                .lock()
                .map_err(|err| anyhow!("locking SQLite dashboard connection: {err}"))?;
            operation(&conn)
        })
        .await
        .context("joining SQLite connection task")?
    }

    pub(super) async fn execute_batch(&self, sql: &str) -> Result<()> {
        let sql = sql.to_string();
        self.with_connection(move |conn| {
            conn.execute_batch(&sql)
                .context("executing SQLite statements")?;
            Ok(())
        })
        .await
    }

    pub(super) async fn ping(&self) -> Result<i32> {
        self.with_connection(|conn| {
            let value: i32 = conn
                .query_row("SELECT 1", [], |row| row.get(0))
                .context("running SQLite health query `SELECT 1`")?;
            Ok(value)
        })
        .await
    }

    pub(super) async fn query_rows(&self, sql: &str) -> Result<Vec<Value>> {
        let sql = sql.to_string();
        self.with_connection(move |conn| sqlite_query_rows_with_connection(conn, &sql))
            .await
    }
}

pub(super) fn ensure_sqlite_file_exists(db_path: &Path) -> Result<()> {
    if db_path.is_file() {
        return Ok(());
    }

    bail!(
        "SQLite database file not found at {}. Run `bitloops init` to create and initialise stores.",
        db_path.display()
    );
}

pub(super) fn open_sqlite_connection_existing(db_path: &Path) -> Result<rusqlite::Connection> {
    ensure_sqlite_file_exists(db_path)?;
    rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
        .with_context(|| format!("opening SQLite database at {}", db_path.display()))
}

pub(super) fn open_sqlite_connection(db_path: &Path) -> Result<rusqlite::Connection> {
    let conn = open_sqlite_connection_existing(db_path)?;
    conn.busy_timeout(std::time::Duration::from_secs(30))
        .context("setting SQLite busy timeout")?;
    Ok(conn)
}

pub(super) async fn sqlite_exec_path(path: &Path, sql: &str) -> Result<()> {
    let db_path = path.to_path_buf();
    let statement = sql.to_string();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = open_sqlite_connection(&db_path)?;
        conn.execute_batch(&statement)
            .context("executing SQLite statements")?;
        Ok(())
    })
    .await
    .context("joining SQLite execute task")?
}

pub(super) fn sqlite_query_rows_with_connection(
    conn: &rusqlite::Connection,
    sql: &str,
) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(sql).context("preparing SQLite query")?;
    let column_names = stmt
        .column_names()
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    let mut rows = stmt.query([]).context("executing SQLite query")?;
    let mut out = Vec::new();

    while let Some(row) = rows.next().context("iterating SQLite query rows")? {
        let mut obj = serde_json::Map::new();
        for (idx, column_name) in column_names.iter().enumerate() {
            let value_ref = row.get_ref(idx).with_context(|| {
                format!("reading SQLite value for column index {idx} (`{column_name}`)")
            })?;
            obj.insert(
                column_name.clone(),
                crate::host::devql::sqlite_value_to_json(value_ref),
            );
        }
        out.push(Value::Object(obj));
    }

    Ok(out)
}
