use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone)]
pub struct SqliteConnectionPool {
    db_path: PathBuf,
}

impl SqliteConnectionPool {
    pub fn connect(db_path: PathBuf) -> Result<Self> {
        create_sqlite_file_if_missing(&db_path)?;
        let pool = Self { db_path };
        pool.with_connection(|_| Ok(()))?;
        Ok(pool)
    }

    pub fn connect_existing(db_path: PathBuf) -> Result<Self> {
        ensure_sqlite_file_exists(&db_path)?;
        let pool = Self { db_path };
        pool.with_connection(|_| Ok(()))?;
        Ok(pool)
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn initialise_checkpoint_schema(&self) -> Result<()> {
        self.execute_batch(crate::engine::devql::checkpoint_schema_sql_sqlite())
            .context("initialising SQLite checkpoint schema")?;
        self.with_connection(|conn| match conn.execute_batch("ALTER TABLE sessions ADD COLUMN ended_at TEXT;") {
            Ok(()) => Ok(()),
            Err(err) if err.to_string().contains("duplicate column name: ended_at") => Ok(()),
            Err(err) => Err(err).context("executing SQLite ended_at migration"),
        })
        .context("migrating SQLite checkpoint schema for sessions.ended_at")
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

fn create_sqlite_file_if_missing(db_path: &Path) -> Result<()> {
    ensure_sqlite_parent_dir(db_path)?;
    if db_path.exists() {
        return Ok(());
    }

    let conn = rusqlite::Connection::open(db_path)
        .with_context(|| format!("creating SQLite database at {}", db_path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(30))
        .context("setting SQLite busy timeout")?;
    Ok(())
}

fn ensure_sqlite_file_exists(db_path: &Path) -> Result<()> {
    if db_path.is_file() {
        return Ok(());
    }

    bail!(
        "SQLite database file not found at {}. Run `bitloops init` to create and initialise stores.",
        db_path.display()
    )
}

fn open_sqlite_connection(db_path: &Path) -> Result<rusqlite::Connection> {
    ensure_sqlite_file_exists(db_path)?;
    let conn =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
            .with_context(|| format!("opening SQLite database at {}", db_path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(30))
        .context("setting SQLite busy timeout")?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use tempfile::TempDir;

    #[test]
    fn sqlite_connection_pool_initialises_checkpoint_schema_tables() -> Result<()> {
        let temp = TempDir::new().context("creating temp dir")?;
        let sqlite_path = temp.path().join("nested").join("checkpoints.sqlite");
        let sqlite = SqliteConnectionPool::connect(sqlite_path)?;
        sqlite.initialise_checkpoint_schema()?;

        for table in [
            "sessions",
            "temporary_checkpoints",
            "checkpoints",
            "checkpoint_sessions",
            "commit_checkpoints",
            "pre_prompt_states",
            "pre_task_markers",
            "checkpoint_blobs",
        ] {
            let exists = sqlite.with_connection(|conn| {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )?;
                Ok(count == 1)
            })?;
            assert!(exists, "expected sqlite checkpoint table `{table}`");
        }

        Ok(())
    }
}
