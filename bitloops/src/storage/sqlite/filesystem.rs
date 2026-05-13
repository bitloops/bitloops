use std::path::Path;

use anyhow::{Context, Result, bail};

use super::{ReadOnlySqliteConnectionPool, SqliteConnectionPool};

impl SqliteConnectionPool {
    pub fn connect(db_path: std::path::PathBuf) -> Result<Self> {
        create_sqlite_file_if_missing(&db_path)?;
        let pool = Self { db_path };
        pool.with_connection(|_| Ok(()))?;
        Ok(pool)
    }

    pub fn connect_existing(db_path: std::path::PathBuf) -> Result<Self> {
        ensure_sqlite_file_exists(&db_path)?;
        let pool = Self { db_path };
        pool.with_connection(|_| Ok(()))?;
        Ok(pool)
    }

    pub fn execute_batch(&self, sql: &str) -> Result<()> {
        self.with_connection(|conn| {
            conn.execute_batch(sql)
                .context("executing SQLite statements")?;
            Ok(())
        })
    }

    pub fn with_connection<T>(
        &self,
        operation: impl FnOnce(&rusqlite::Connection) -> Result<T>,
    ) -> Result<T> {
        let conn = open_sqlite_connection(&self.db_path)?;
        operation(&conn)
    }
}

impl ReadOnlySqliteConnectionPool {
    pub fn connect_existing(db_path: std::path::PathBuf) -> Result<Self> {
        ensure_sqlite_file_exists(&db_path)?;
        let pool = Self { db_path };
        pool.with_connection(|_| Ok(()))?;
        Ok(pool)
    }

    pub fn with_connection<T>(
        &self,
        operation: impl FnOnce(&rusqlite::Connection) -> Result<T>,
    ) -> Result<T> {
        let conn = open_read_only_sqlite_connection(&self.db_path)?;
        operation(&conn)
    }
}

fn ensure_sqlite_parent_dir(db_path: &Path) -> Result<()> {
    let parent = db_path
        .parent()
        .filter(|candidate| !candidate.as_os_str().is_empty());
    if let Some(parent) = parent {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating SQLite parent directory {}", parent.display()))?;
    }
    Ok(())
}

pub(super) fn create_sqlite_file_if_missing(db_path: &Path) -> Result<()> {
    ensure_sqlite_parent_dir(db_path)?;
    if db_path.exists() {
        return Ok(());
    }

    let conn = rusqlite::Connection::open(db_path)
        .with_context(|| format!("creating SQLite database at {}", db_path.display()))?;
    configure_sqlite_connection(&conn)?;
    Ok(())
}

pub(super) fn ensure_sqlite_file_exists(db_path: &Path) -> Result<()> {
    if db_path.is_file() {
        return Ok(());
    }

    bail!(
        "SQLite database file not found at {}. Run `bitloops init` to create and initialise stores.",
        db_path.display()
    )
}

pub(super) fn open_sqlite_connection(db_path: &Path) -> Result<rusqlite::Connection> {
    ensure_sqlite_file_exists(db_path)?;
    let conn =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
            .with_context(|| format!("opening SQLite database at {}", db_path.display()))?;
    configure_sqlite_connection(&conn)?;
    Ok(conn)
}

pub(super) fn open_read_only_sqlite_connection(db_path: &Path) -> Result<rusqlite::Connection> {
    ensure_sqlite_file_exists(db_path)?;
    let conn =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| {
                format!("opening SQLite database read-only at {}", db_path.display())
            })?;
    configure_read_only_sqlite_connection(&conn)?;
    Ok(conn)
}

fn configure_sqlite_connection(conn: &rusqlite::Connection) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(30))
        .context("setting SQLite busy timeout")?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;",
    )
    .context("configuring SQLite pragmas")?;
    Ok(())
}

fn configure_read_only_sqlite_connection(conn: &rusqlite::Connection) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(30))
        .context("setting SQLite busy timeout")?;
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA query_only = ON;")
        .context("configuring read-only SQLite pragmas")?;
    Ok(())
}
