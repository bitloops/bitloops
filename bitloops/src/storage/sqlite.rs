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
        self.execute_batch(crate::host::devql::checkpoint_schema_sql_sqlite())
            .context("initialising SQLite checkpoint schema")?;
        self.with_connection(|conn| {
            match conn.execute_batch("ALTER TABLE sessions ADD COLUMN ended_at TEXT;") {
                Ok(()) => Ok(()),
                Err(err) if err.to_string().contains("duplicate column name: ended_at") => Ok(()),
                Err(err) => Err(err).context("executing SQLite ended_at migration"),
            }
        })
        .context("migrating SQLite checkpoint schema for sessions.ended_at")
    }

    pub fn initialise_devql_schema(&self) -> Result<()> {
        self.execute_batch(crate::host::devql::devql_schema_sql_sqlite())
            .context("initialising SQLite DevQL schema")?;
        self.migrate_devql_checkpoint_columns()
            .context("migrating SQLite DevQL checkpoint columns")?;
        self.migrate_workspace_revisions_uniqueness()
            .context("migrating SQLite workspace_revisions uniqueness")
    }

    fn migrate_devql_checkpoint_columns(&self) -> Result<()> {
        let migrations = [
            (
                "artefacts_current",
                "revision_kind",
                "ALTER TABLE artefacts_current ADD COLUMN revision_kind TEXT NOT NULL DEFAULT 'commit';",
            ),
            (
                "artefacts_current",
                "revision_id",
                "ALTER TABLE artefacts_current ADD COLUMN revision_id TEXT NOT NULL DEFAULT '';",
            ),
            (
                "artefacts_current",
                "temp_checkpoint_id",
                "ALTER TABLE artefacts_current ADD COLUMN temp_checkpoint_id INTEGER;",
            ),
            (
                "artefact_edges_current",
                "revision_kind",
                "ALTER TABLE artefact_edges_current ADD COLUMN revision_kind TEXT NOT NULL DEFAULT 'commit';",
            ),
            (
                "artefact_edges_current",
                "revision_id",
                "ALTER TABLE artefact_edges_current ADD COLUMN revision_id TEXT NOT NULL DEFAULT '';",
            ),
            (
                "artefact_edges_current",
                "temp_checkpoint_id",
                "ALTER TABLE artefact_edges_current ADD COLUMN temp_checkpoint_id INTEGER;",
            ),
        ];
        self.with_connection(|conn| {
            for (_table, column, sql) in migrations {
                match conn.execute_batch(sql) {
                    Ok(()) => {}
                    Err(err)
                        if err
                            .to_string()
                            .contains(&format!("duplicate column name: {column}")) => {}
                    Err(err) => {
                        return Err(err).with_context(|| format!("adding column {column}"));
                    }
                }
            }
            Ok(())
        })
    }

    fn migrate_workspace_revisions_uniqueness(&self) -> Result<()> {
        self.with_connection(|conn| {
            conn.execute_batch(
                r#"
DELETE FROM workspace_revisions
WHERE id NOT IN (
    SELECT MAX(id)
    FROM workspace_revisions
    GROUP BY repo_id, tree_hash
);
DROP INDEX IF EXISTS workspace_revisions_tree_idx;
CREATE UNIQUE INDEX IF NOT EXISTS workspace_revisions_repo_tree_unique_idx
ON workspace_revisions (repo_id, tree_hash);
"#,
            )
            .context("hardening workspace_revisions uniqueness")?;
            Ok(())
        })
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
    fn sqlite_connection_pool_initialises_devql_schema_workspace_revisions_table() -> Result<()> {
        let temp = TempDir::new().context("creating temp dir")?;
        let sqlite_path = temp.path().join("devql.sqlite");
        let sqlite = SqliteConnectionPool::connect(sqlite_path)?;
        sqlite.initialise_devql_schema()?;

        let exists = sqlite.with_connection(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'workspace_revisions'",
                [],
                |row| row.get(0),
            )?;
            Ok(count == 1)
        })?;
        assert!(
            exists,
            "workspace_revisions table should exist after initialise_devql_schema"
        );

        // Verify indexes were also created
        let index_count: i64 = sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND tbl_name = 'workspace_revisions'",
                [],
                |row| row.get(0),
            ).map_err(anyhow::Error::from)
        })?;
        assert!(
            index_count >= 2,
            "expected at least 2 indexes on workspace_revisions, found {index_count}"
        );

        Ok(())
    }

    #[test]
    fn workspace_revisions_table_supports_insert_and_dedup_query() -> Result<()> {
        let temp = TempDir::new().context("creating temp dir")?;
        let sqlite_path = temp.path().join("devql.sqlite");
        let sqlite = SqliteConnectionPool::connect(sqlite_path)?;
        sqlite.initialise_devql_schema()?;

        // Insert two rows for different repos and one duplicate tree_hash
        sqlite.with_connection(|conn| {
            conn.execute(
                "INSERT INTO workspace_revisions (repo_id, tree_hash) VALUES ('repo-a', 'hash-1')",
                [],
            )?;
            conn.execute(
                "INSERT INTO workspace_revisions (repo_id, tree_hash) VALUES ('repo-a', 'hash-2')",
                [],
            )?;
            conn.execute(
                "INSERT INTO workspace_revisions (repo_id, tree_hash) VALUES ('repo-b', 'hash-1')",
                [],
            )?;
            Ok(())
        })?;

        let latest_a: String = sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT tree_hash FROM workspace_revisions WHERE repo_id = 'repo-a' ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            ).map_err(anyhow::Error::from)
        })?;
        assert_eq!(
            latest_a, "hash-2",
            "latest tree_hash for repo-a should be hash-2"
        );

        let latest_b: String = sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT tree_hash FROM workspace_revisions WHERE repo_id = 'repo-b' ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            ).map_err(anyhow::Error::from)
        })?;
        assert_eq!(
            latest_b, "hash-1",
            "latest tree_hash for repo-b should be hash-1"
        );

        // autoincrement ids must be monotone
        let ids: Vec<i64> = sqlite.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT id FROM workspace_revisions ORDER BY id ASC")?;
            let rows = stmt.query_map([], |row| row.get(0))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(anyhow::Error::from)
        })?;
        assert_eq!(ids, vec![1, 2, 3], "ids must be autoincremented from 1");

        Ok(())
    }

    #[test]
    fn workspace_revisions_enforces_unique_tree_hash_per_repo() -> Result<()> {
        let temp = TempDir::new().context("creating temp dir")?;
        let sqlite_path = temp.path().join("devql.sqlite");
        let sqlite = SqliteConnectionPool::connect(sqlite_path)?;
        sqlite.initialise_devql_schema()?;

        sqlite.with_connection(|conn| {
            conn.execute(
                "INSERT INTO workspace_revisions (repo_id, tree_hash) VALUES ('repo-a', 'hash-1')",
                [],
            )?;
            let duplicate = conn.execute(
                "INSERT INTO workspace_revisions (repo_id, tree_hash) VALUES ('repo-a', 'hash-1')",
                [],
            );
            assert!(
                duplicate.is_err(),
                "duplicate repo/tree_hash inserts should be rejected by SQLite"
            );
            Ok(())
        })?;

        let duplicate_count: i64 = sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM workspace_revisions WHERE repo_id = 'repo-a' AND tree_hash = 'hash-1'",
                [],
                |row| row.get(0),
            )
            .map_err(anyhow::Error::from)
        })?;
        assert_eq!(
            duplicate_count, 1,
            "workspace_revisions should store at most one row per repo/tree_hash pair"
        );

        Ok(())
    }

    #[test]
    fn initialise_devql_schema_is_idempotent() -> Result<()> {
        let temp = TempDir::new().context("creating temp dir")?;
        let sqlite_path = temp.path().join("devql.sqlite");
        let sqlite = SqliteConnectionPool::connect(sqlite_path)?;
        // Calling twice should not error
        sqlite.initialise_devql_schema()?;
        sqlite.initialise_devql_schema()?;

        let exists = sqlite.with_connection(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'workspace_revisions'",
                [],
                |row| row.get(0),
            )?;
            Ok(count == 1)
        })?;
        assert!(
            exists,
            "workspace_revisions should still exist after double init"
        );
        Ok(())
    }

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

    #[test]
    fn initialise_devql_schema_migrates_legacy_artefacts_current_missing_checkpoint_columns()
    -> Result<()> {
        let temp = TempDir::new().context("creating temp dir")?;
        let sqlite_path = temp.path().join("devql.sqlite");
        let sqlite = SqliteConnectionPool::connect(sqlite_path)?;

        // Create artefacts_current with the old schema (missing checkpoint columns).
        sqlite.execute_batch(
            "CREATE TABLE artefacts_current (
                repo_id TEXT NOT NULL,
                symbol_id TEXT NOT NULL,
                artefact_id TEXT NOT NULL,
                commit_sha TEXT NOT NULL,
                blob_sha TEXT NOT NULL,
                path TEXT NOT NULL,
                language TEXT NOT NULL,
                canonical_kind TEXT,
                language_kind TEXT,
                symbol_fqn TEXT,
                parent_symbol_id TEXT,
                parent_artefact_id TEXT,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                start_byte INTEGER NOT NULL,
                end_byte INTEGER NOT NULL,
                signature TEXT,
                modifiers TEXT NOT NULL DEFAULT '[]',
                docstring TEXT,
                content_hash TEXT,
                updated_at TEXT DEFAULT (datetime('now')),
                PRIMARY KEY (repo_id, symbol_id)
            );",
        )?;

        // Run the current migration path.
        sqlite.initialise_devql_schema()?;

        // The runtime INSERT that was failing before the migration fix.
        sqlite.with_connection(|conn| {
            conn.execute(
                "INSERT INTO artefacts_current
                    (repo_id, symbol_id, artefact_id, commit_sha,
                     revision_kind, revision_id, temp_checkpoint_id,
                     blob_sha, path, language, start_line, end_line,
                     start_byte, end_byte)
                 VALUES ('r', 's', 'a', 'c',
                         'commit', 'c', NULL,
                         'b', 'p', 'rust', 1, 10, 0, 100)",
                [],
            )?;
            Ok(())
        })?;

        // The runtime SELECT that was failing before the migration fix.
        let revision_kind: String = sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT revision_kind FROM artefacts_current WHERE repo_id = 'r' AND symbol_id = 's'",
                [],
                |row| row.get(0),
            )
            .map_err(anyhow::Error::from)
        })?;
        assert_eq!(revision_kind, "commit");

        Ok(())
    }

    #[test]
    fn initialise_devql_schema_migrates_legacy_artefact_edges_current_missing_checkpoint_columns()
    -> Result<()> {
        let temp = TempDir::new().context("creating temp dir")?;
        let sqlite_path = temp.path().join("devql.sqlite");
        let sqlite = SqliteConnectionPool::connect(sqlite_path)?;

        // Create artefact_edges_current with the old schema (missing checkpoint columns).
        sqlite.execute_batch(
            "CREATE TABLE artefact_edges_current (
                edge_id TEXT PRIMARY KEY,
                repo_id TEXT NOT NULL,
                commit_sha TEXT NOT NULL,
                blob_sha TEXT NOT NULL,
                path TEXT NOT NULL,
                from_symbol_id TEXT NOT NULL,
                from_artefact_id TEXT NOT NULL,
                to_symbol_id TEXT,
                to_artefact_id TEXT,
                to_symbol_ref TEXT,
                edge_kind TEXT NOT NULL,
                language TEXT NOT NULL,
                start_line INTEGER,
                end_line INTEGER,
                metadata TEXT DEFAULT '{}',
                updated_at TEXT DEFAULT (datetime('now')),
                CHECK (to_symbol_id IS NOT NULL OR to_symbol_ref IS NOT NULL)
            );",
        )?;

        sqlite.initialise_devql_schema()?;

        sqlite.with_connection(|conn| {
            conn.execute(
                "INSERT INTO artefact_edges_current
                    (edge_id, repo_id, commit_sha, revision_kind, revision_id,
                     temp_checkpoint_id, blob_sha, path, from_symbol_id,
                     from_artefact_id, to_symbol_ref, edge_kind, language)
                 VALUES ('e1', 'r', 'c', 'commit', 'c',
                         NULL, 'b', 'p', 'from_s',
                         'from_a', 'ref', 'imports', 'rust')",
                [],
            )?;
            Ok(())
        })?;

        let revision_kind: String = sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT revision_kind FROM artefact_edges_current WHERE edge_id = 'e1'",
                [],
                |row| row.get(0),
            )
            .map_err(anyhow::Error::from)
        })?;
        assert_eq!(revision_kind, "commit");

        Ok(())
    }
}
