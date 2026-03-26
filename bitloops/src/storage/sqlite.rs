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
        self.migrate_workspace_revisions_uniqueness()
            .context("migrating SQLite workspace_revisions uniqueness (pre-schema)")?;
        self.execute_batch(crate::host::devql::devql_schema_sql_sqlite())
            .context("initialising SQLite DevQL schema")?;
        self.migrate_devql_checkpoint_columns()
            .context("migrating SQLite DevQL checkpoint columns")?;
        self.migrate_devql_branch_scope_current_tables()
            .context("migrating SQLite DevQL current-state branch scope")?;
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
            if !sqlite_table_exists(conn, "workspace_revisions")? {
                return Ok(());
            }
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

    fn migrate_devql_branch_scope_current_tables(&self) -> Result<()> {
        self.with_connection(|conn| {
            migrate_artefacts_current_branch_scope(conn)?;
            migrate_artefact_edges_current_branch_scope(conn)?;
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

fn sqlite_table_exists(conn: &rusqlite::Connection, table: &str) -> Result<bool> {
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get(0),
        )
        .context("checking SQLite table existence")?;
    Ok(exists > 0)
}

fn sqlite_table_pk_columns(conn: &rusqlite::Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .with_context(|| format!("preparing PRAGMA table_info for `{table}`"))?;
    let mut rows = stmt
        .query([])
        .with_context(|| format!("querying PRAGMA table_info for `{table}`"))?;
    let mut pk = Vec::<(i64, String)>::new();
    while let Some(row) = rows.next().context("reading PRAGMA row")? {
        let name: String = row
            .get(1)
            .with_context(|| format!("reading column name from `{table}`"))?;
        let order: i64 = row
            .get(5)
            .with_context(|| format!("reading pk order from `{table}`"))?;
        if order > 0 {
            pk.push((order, name));
        }
    }
    pk.sort_by_key(|(order, _)| *order);
    Ok(pk.into_iter().map(|(_, name)| name).collect())
}

fn sqlite_table_has_column(conn: &rusqlite::Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .with_context(|| format!("preparing PRAGMA table_info for `{table}`"))?;
    let mut rows = stmt
        .query([])
        .with_context(|| format!("querying PRAGMA table_info for `{table}`"))?;
    while let Some(row) = rows.next().context("reading PRAGMA row")? {
        let name: String = row
            .get(1)
            .with_context(|| format!("reading column name from `{table}`"))?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn migrate_artefacts_current_branch_scope(conn: &rusqlite::Connection) -> Result<()> {
    if !sqlite_table_exists(conn, "artefacts_current")? {
        return Ok(());
    }

    let has_branch = sqlite_table_has_column(conn, "artefacts_current", "branch")?;
    let pk_columns = sqlite_table_pk_columns(conn, "artefacts_current")?;
    let needs_rebuild = !has_branch
        || pk_columns
            != [
                "repo_id".to_string(),
                "branch".to_string(),
                "symbol_id".to_string(),
            ];

    if needs_rebuild {
        conn.execute_batch(
            r#"
DROP TABLE IF EXISTS artefacts_current__branch_migration;
CREATE TABLE artefacts_current__branch_migration (
    repo_id TEXT NOT NULL,
    branch TEXT NOT NULL DEFAULT 'main',
    symbol_id TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    revision_kind TEXT NOT NULL DEFAULT 'commit',
    revision_id TEXT NOT NULL DEFAULT '',
    temp_checkpoint_id INTEGER,
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
    PRIMARY KEY (repo_id, branch, symbol_id)
);
INSERT INTO artefacts_current__branch_migration (
    repo_id, branch, symbol_id, artefact_id, commit_sha, revision_kind, revision_id, temp_checkpoint_id,
    blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_symbol_id,
    parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers,
    docstring, content_hash, updated_at
)
SELECT
    ac.repo_id,
    COALESCE(NULLIF((SELECT r.default_branch FROM repositories r WHERE r.repo_id = ac.repo_id LIMIT 1), ''), 'main') AS branch,
    ac.symbol_id,
    ac.artefact_id,
    ac.commit_sha,
    ac.revision_kind,
    ac.revision_id,
    ac.temp_checkpoint_id,
    ac.blob_sha,
    ac.path,
    ac.language,
    ac.canonical_kind,
    ac.language_kind,
    ac.symbol_fqn,
    ac.parent_symbol_id,
    ac.parent_artefact_id,
    ac.start_line,
    ac.end_line,
    ac.start_byte,
    ac.end_byte,
    ac.signature,
    ac.modifiers,
    ac.docstring,
    ac.content_hash,
    ac.updated_at
FROM artefacts_current ac;
DROP TABLE artefacts_current;
ALTER TABLE artefacts_current__branch_migration RENAME TO artefacts_current;
"#,
        )
        .context("rebuilding artefacts_current with branch-scoped primary key")?;
    }

    conn.execute_batch(
        r#"
DROP INDEX IF EXISTS artefacts_current_path_idx;
DROP INDEX IF EXISTS artefacts_current_kind_idx;
DROP INDEX IF EXISTS artefacts_current_symbol_fqn_idx;
DROP INDEX IF EXISTS artefacts_current_branch_path_idx;
DROP INDEX IF EXISTS artefacts_current_branch_kind_idx;
DROP INDEX IF EXISTS artefacts_current_branch_fqn_idx;
DROP INDEX IF EXISTS artefacts_current_artefact_idx;

CREATE INDEX IF NOT EXISTS artefacts_current_branch_path_idx
ON artefacts_current (repo_id, branch, path);

CREATE INDEX IF NOT EXISTS artefacts_current_branch_kind_idx
ON artefacts_current (repo_id, branch, canonical_kind);

CREATE INDEX IF NOT EXISTS artefacts_current_artefact_idx
ON artefacts_current (repo_id, branch, artefact_id);

CREATE INDEX IF NOT EXISTS artefacts_current_branch_fqn_idx
ON artefacts_current (repo_id, branch, symbol_fqn);
"#,
    )
    .context("ensuring branch-aware artefacts_current indexes")?;

    Ok(())
}

fn migrate_artefact_edges_current_branch_scope(conn: &rusqlite::Connection) -> Result<()> {
    if !sqlite_table_exists(conn, "artefact_edges_current")? {
        return Ok(());
    }

    let has_branch = sqlite_table_has_column(conn, "artefact_edges_current", "branch")?;
    let pk_columns = sqlite_table_pk_columns(conn, "artefact_edges_current")?;
    let needs_rebuild = !has_branch
        || pk_columns
            != [
                "repo_id".to_string(),
                "branch".to_string(),
                "edge_id".to_string(),
            ];

    if needs_rebuild {
        conn.execute_batch(
            r#"
DROP TABLE IF EXISTS artefact_edges_current__branch_migration;
CREATE TABLE artefact_edges_current__branch_migration (
    edge_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    branch TEXT NOT NULL DEFAULT 'main',
    commit_sha TEXT NOT NULL,
    revision_kind TEXT NOT NULL DEFAULT 'commit',
    revision_id TEXT NOT NULL DEFAULT '',
    temp_checkpoint_id INTEGER,
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
    CHECK (to_symbol_id IS NOT NULL OR to_symbol_ref IS NOT NULL),
    CHECK (
        (start_line IS NULL AND end_line IS NULL)
        OR (start_line IS NOT NULL AND end_line IS NOT NULL AND start_line > 0 AND end_line >= start_line)
    ),
    PRIMARY KEY (repo_id, branch, edge_id)
);
INSERT INTO artefact_edges_current__branch_migration (
    edge_id, repo_id, branch, commit_sha, revision_kind, revision_id, temp_checkpoint_id, blob_sha,
    path, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref,
    edge_kind, language, start_line, end_line, metadata, updated_at
)
SELECT
    ec.edge_id,
    ec.repo_id,
    COALESCE(NULLIF((SELECT r.default_branch FROM repositories r WHERE r.repo_id = ec.repo_id LIMIT 1), ''), 'main') AS branch,
    ec.commit_sha,
    ec.revision_kind,
    ec.revision_id,
    ec.temp_checkpoint_id,
    ec.blob_sha,
    ec.path,
    ec.from_symbol_id,
    ec.from_artefact_id,
    ec.to_symbol_id,
    ec.to_artefact_id,
    ec.to_symbol_ref,
    ec.edge_kind,
    ec.language,
    ec.start_line,
    ec.end_line,
    ec.metadata,
    ec.updated_at
FROM artefact_edges_current ec;
DROP TABLE artefact_edges_current;
ALTER TABLE artefact_edges_current__branch_migration RENAME TO artefact_edges_current;
"#,
        )
        .context("rebuilding artefact_edges_current with branch-scoped primary key")?;
    }

    conn.execute_batch(
        r#"
DROP INDEX IF EXISTS artefact_edges_current_from_idx;
DROP INDEX IF EXISTS artefact_edges_current_to_idx;
DROP INDEX IF EXISTS artefact_edges_current_branch_from_idx;
DROP INDEX IF EXISTS artefact_edges_current_branch_to_idx;
DROP INDEX IF EXISTS artefact_edges_current_path_idx;
DROP INDEX IF EXISTS artefact_edges_current_kind_idx;
DROP INDEX IF EXISTS artefact_edges_current_symbol_ref_idx;
DROP INDEX IF EXISTS artefact_edges_current_natural_uq;

CREATE INDEX IF NOT EXISTS artefact_edges_current_path_idx
ON artefact_edges_current (repo_id, branch, path);

CREATE INDEX IF NOT EXISTS artefact_edges_current_branch_from_idx
ON artefact_edges_current (repo_id, branch, from_symbol_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_current_branch_to_idx
ON artefact_edges_current (repo_id, branch, to_symbol_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_current_kind_idx
ON artefact_edges_current (repo_id, branch, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_current_symbol_ref_idx
ON artefact_edges_current (repo_id, branch, to_symbol_ref);

CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_current_natural_uq
ON artefact_edges_current (
    repo_id,
    branch,
    from_symbol_id,
    edge_kind,
    COALESCE(to_symbol_id, ''),
    COALESCE(to_symbol_ref, ''),
    COALESCE(start_line, -1),
    COALESCE(end_line, -1),
    COALESCE(metadata, '{}')
);
"#,
    )
    .context("ensuring branch-aware artefact_edges_current indexes")?;

    Ok(())
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
    fn sqlite_connection_pool_initialises_checkpoint_file_snapshots_projection() -> Result<()> {
        let temp = TempDir::new().context("creating temp dir")?;
        let sqlite_path = temp.path().join("devql.sqlite");
        let sqlite = SqliteConnectionPool::connect(sqlite_path)?;
        sqlite.initialise_devql_schema()?;

        let exists = sqlite.with_connection(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'checkpoint_file_snapshots'",
                [],
                |row| row.get(0),
            )?;
            Ok(count == 1)
        })?;
        assert!(
            exists,
            "checkpoint_file_snapshots should exist after initialise_devql_schema"
        );

        let pk_columns = sqlite
            .with_connection(|conn| sqlite_table_pk_columns(conn, "checkpoint_file_snapshots"))?;
        assert_eq!(
            pk_columns,
            vec![
                "repo_id".to_string(),
                "checkpoint_id".to_string(),
                "path".to_string(),
                "blob_sha".to_string(),
            ],
            "checkpoint_file_snapshots should use the composite projection key"
        );

        let index_names: Vec<String> = sqlite.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT name
                 FROM sqlite_master
                 WHERE type = 'index'
                   AND tbl_name = 'checkpoint_file_snapshots'
                   AND name NOT LIKE 'sqlite_autoindex_%'
                 ORDER BY name",
            )?;
            let rows = stmt.query_map([], |row| row.get(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(anyhow::Error::from)
        })?;
        assert_eq!(
            index_names,
            vec![
                "checkpoint_file_snapshots_agent_time_idx".to_string(),
                "checkpoint_file_snapshots_checkpoint_idx".to_string(),
                "checkpoint_file_snapshots_commit_idx".to_string(),
                "checkpoint_file_snapshots_event_time_idx".to_string(),
                "checkpoint_file_snapshots_lookup_idx".to_string(),
            ],
            "checkpoint_file_snapshots should create the expected lookup indexes"
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

        let projection_exists = sqlite.with_connection(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'checkpoint_file_snapshots'",
                [],
                |row| row.get(0),
            )?;
            Ok(count == 1)
        })?;
        assert!(
            projection_exists,
            "checkpoint_file_snapshots should still exist after double init"
        );
        Ok(())
    }

    #[test]
    fn initialise_devql_schema_recovers_legacy_workspace_revision_duplicates() -> Result<()> {
        let temp = TempDir::new().context("creating temp dir")?;
        let sqlite_path = temp.path().join("devql.sqlite");
        let sqlite = SqliteConnectionPool::connect(sqlite_path)?;

        sqlite.execute_batch(
            r#"
CREATE TABLE workspace_revisions (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id    TEXT    NOT NULL,
    tree_hash  TEXT    NOT NULL,
    created_at TEXT    DEFAULT (datetime('now'))
);

CREATE INDEX workspace_revisions_repo_idx
ON workspace_revisions (repo_id);

INSERT INTO workspace_revisions (repo_id, tree_hash) VALUES ('repo-a', 'hash-1');
INSERT INTO workspace_revisions (repo_id, tree_hash) VALUES ('repo-a', 'hash-1');
INSERT INTO workspace_revisions (repo_id, tree_hash) VALUES ('repo-a', 'hash-2');
"#,
        )?;

        sqlite.initialise_devql_schema()?;

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
            "legacy duplicate workspace_revisions rows should be deduplicated"
        );

        let duplicate_insert_rejected = sqlite.with_connection(|conn| {
            Ok(conn
                .execute(
                    "INSERT INTO workspace_revisions (repo_id, tree_hash) VALUES ('repo-a', 'hash-2')",
                    [],
                )
                .is_err())
        })?;
        assert!(
            duplicate_insert_rejected,
            "unique repo/tree_hash inserts must be enforced after migration"
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

    #[test]
    fn initialise_devql_schema_assigns_legacy_current_state_rows_to_repository_default_branch()
    -> Result<()> {
        let temp = TempDir::new().context("creating temp dir")?;
        let sqlite_path = temp.path().join("devql.sqlite");
        let sqlite = SqliteConnectionPool::connect(sqlite_path)?;

        sqlite.execute_batch(
            "CREATE TABLE repositories (
                repo_id TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                organization TEXT NOT NULL,
                name TEXT NOT NULL,
                default_branch TEXT,
                created_at TEXT
            );
            INSERT INTO repositories (repo_id, provider, organization, name, default_branch, created_at)
            VALUES ('repo-legacy', 'git', 'bitloops', 'bitloops', 'feature/legacy-default', datetime('now'));",
        )?;

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
            );
            INSERT INTO artefacts_current (
                repo_id, symbol_id, artefact_id, commit_sha, blob_sha, path, language,
                canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id,
                start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash
            ) VALUES (
                'repo-legacy', 'legacy-symbol', 'legacy-artefact', 'legacy-commit', 'legacy-blob',
                'src/legacy.ts', 'typescript', 'function', 'function', 'src/legacy.ts::legacySymbol',
                NULL, NULL, 1, 1, 0, 10, 'legacy()', '[]', 'legacy docs', 'legacy-hash'
            );",
        )?;

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
            );
            INSERT INTO artefact_edges_current (
                edge_id, repo_id, commit_sha, blob_sha, path, from_symbol_id, from_artefact_id,
                to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata
            ) VALUES (
                'legacy-edge', 'repo-legacy', 'legacy-commit', 'legacy-blob', 'src/legacy.ts',
                'legacy-symbol', 'legacy-artefact', NULL, NULL, 'target::legacy', 'references',
                'typescript', 1, 1, '{}'
            );",
        )?;

        sqlite.initialise_devql_schema()?;

        let migrated_artefact_rows: i64 = sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM artefacts_current \
                 WHERE repo_id = 'repo-legacy' AND branch = 'feature/legacy-default' AND symbol_id = 'legacy-symbol'",
                [],
                |row| row.get(0),
            )
            .map_err(anyhow::Error::from)
        })?;
        assert_eq!(
            migrated_artefact_rows, 1,
            "legacy artefacts_current rows should migrate to the repository default branch"
        );

        let migrated_edge_rows: i64 = sqlite.with_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM artefact_edges_current \
                 WHERE repo_id = 'repo-legacy' AND branch = 'feature/legacy-default' AND edge_id = 'legacy-edge'",
                [],
                |row| row.get(0),
            )
            .map_err(anyhow::Error::from)
        })?;
        assert_eq!(
            migrated_edge_rows, 1,
            "legacy artefact_edges_current rows should migrate to the repository default branch"
        );

        Ok(())
    }
}
