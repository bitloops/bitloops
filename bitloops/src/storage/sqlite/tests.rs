use anyhow::{Context, Result};
use std::time::{Duration, Instant};
use tempfile::TempDir;

use super::SqliteConnectionPool;
use super::introspection::{sqlite_table_has_column, sqlite_table_pk_columns};

#[test]
fn sqlite_connection_pool_execute_batch_waits_for_shared_write_lock() -> Result<()> {
    let temp = TempDir::new().context("creating temp dir")?;
    let sqlite_path = temp.path().join("runtime.sqlite");
    let sqlite = SqliteConnectionPool::connect(sqlite_path.clone())?;
    sqlite.execute_batch("CREATE TABLE sample (value INTEGER NOT NULL);")?;

    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let sqlite_path_for_blocker = sqlite_path.clone();
    let blocker = std::thread::spawn(move || -> Result<()> {
        super::with_sqlite_write_lock(&sqlite_path_for_blocker, || {
            locked_tx.send(()).expect("signal lock held");
            std::thread::sleep(Duration::from_millis(150));
            Ok(())
        })
    });
    locked_rx.recv().context("waiting for blocker lock")?;

    let started = Instant::now();
    sqlite.execute_batch("INSERT INTO sample (value) VALUES (1);")?;
    let elapsed = started.elapsed();

    blocker
        .join()
        .map_err(|_| anyhow::anyhow!("joining blocker thread"))??;
    assert!(
        elapsed >= Duration::from_millis(100),
        "SQLite execute_batch writes should wait for the shared write lock; elapsed={elapsed:?}"
    );

    let count: i64 = sqlite.with_connection(|conn| {
        conn.query_row("SELECT COUNT(*) FROM sample", [], |row| row.get(0))
            .map_err(anyhow::Error::from)
    })?;
    assert_eq!(count, 1);

    Ok(())
}

#[test]
fn sqlite_connection_pool_with_connection_is_read_only() -> Result<()> {
    let temp = TempDir::new().context("creating temp dir")?;
    let sqlite_path = temp.path().join("runtime.sqlite");
    let sqlite = SqliteConnectionPool::connect(sqlite_path)?;
    sqlite.execute_batch("CREATE TABLE sample (value INTEGER NOT NULL);")?;

    let err = sqlite
        .with_connection(|conn| {
            conn.execute("INSERT INTO sample (value) VALUES (1)", [])
                .map(|_| ())
                .map_err(anyhow::Error::from)
        })
        .expect_err("read-only SQLite connection should reject writes");

    let message = format!("{err:#}").to_ascii_lowercase();
    assert!(
        message.contains("readonly") || message.contains("read-only"),
        "expected read-only SQLite error, got {err:#}"
    );

    Ok(())
}

#[test]
fn sqlite_connection_pool_uses_wal_and_normal_synchronous() -> Result<()> {
    let temp = TempDir::new().context("creating temp dir")?;
    let sqlite_path = temp.path().join("runtime.sqlite");
    let sqlite = SqliteConnectionPool::connect(sqlite_path)?;

    let (journal_mode, synchronous): (String, i64) = sqlite.with_connection(|conn| {
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode;", [], |row| row.get(0))
            .context("read journal_mode pragma")?;
        let synchronous: i64 = conn
            .query_row("PRAGMA synchronous;", [], |row| row.get(0))
            .context("read synchronous pragma")?;
        Ok((journal_mode, synchronous))
    })?;

    assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
    assert_eq!(
        synchronous, 1,
        "SQLite NORMAL synchronous pragma should be enabled"
    );

    Ok(())
}

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

    let index_count: i64 = sqlite.with_connection(|conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND tbl_name = 'workspace_revisions'",
            [],
            |row| row.get(0),
        )
        .map_err(anyhow::Error::from)
    })?;
    assert!(
        index_count >= 2,
        "expected at least 2 indexes on workspace_revisions, found {index_count}"
    );

    Ok(())
}

#[test]
fn sqlite_connection_pool_initialises_checkpoint_provenance_tables() -> Result<()> {
    let temp = TempDir::new().context("creating temp dir")?;
    let sqlite_path = temp.path().join("devql.sqlite");
    let sqlite = SqliteConnectionPool::connect(sqlite_path)?;
    sqlite.initialise_devql_schema()?;

    let exists = sqlite.with_connection(|conn| {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'checkpoint_files'",
            [],
            |row| row.get(0),
        )?;
        Ok(count == 1)
    })?;
    assert!(
        exists,
        "checkpoint_files should exist after initialise_devql_schema"
    );

    let pk_columns =
        sqlite.with_connection(|conn| sqlite_table_pk_columns(conn, "checkpoint_files"))?;
    assert_eq!(pk_columns, vec!["relation_id".to_string()]);

    let index_names: Vec<String> = sqlite.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT name
             FROM sqlite_master
             WHERE type = 'index'
               AND tbl_name = 'checkpoint_files'
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
            "checkpoint_files_agent_time_idx".to_string(),
            "checkpoint_files_change_kind_idx".to_string(),
            "checkpoint_files_checkpoint_idx".to_string(),
            "checkpoint_files_commit_idx".to_string(),
            "checkpoint_files_copy_source_idx".to_string(),
            "checkpoint_files_event_time_idx".to_string(),
            "checkpoint_files_lookup_idx".to_string(),
        ],
        "checkpoint_files should create the expected lookup indexes"
    );

    let artefact_exists = sqlite.with_connection(|conn| {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'checkpoint_artefacts'",
            [],
            |row| row.get(0),
        )?;
        Ok(count == 1)
    })?;
    assert!(
        artefact_exists,
        "checkpoint_artefacts should exist after initialise_devql_schema"
    );

    let artefact_pk_columns =
        sqlite.with_connection(|conn| sqlite_table_pk_columns(conn, "checkpoint_artefacts"))?;
    assert_eq!(artefact_pk_columns, vec!["relation_id".to_string()]);

    let lineage_exists = sqlite.with_connection(|conn| {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'checkpoint_artefact_lineage'",
            [],
            |row| row.get(0),
        )?;
        Ok(count == 1)
    })?;
    assert!(
        lineage_exists,
        "checkpoint_artefact_lineage should exist after initialise_devql_schema"
    );

    Ok(())
}

#[test]
fn workspace_revisions_table_supports_insert_and_dedup_query() -> Result<()> {
    let temp = TempDir::new().context("creating temp dir")?;
    let sqlite_path = temp.path().join("devql.sqlite");
    let sqlite = SqliteConnectionPool::connect(sqlite_path)?;
    sqlite.initialise_devql_schema()?;

    sqlite.with_write_connection(|conn| {
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

    sqlite.with_write_connection(|conn| {
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
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'checkpoint_files'",
            [],
            |row| row.get(0),
        )?;
        Ok(count == 1)
    })?;
    assert!(
        projection_exists,
        "checkpoint_files should still exist after double init"
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

    let duplicate_insert_rejected = sqlite.with_write_connection(|conn| {
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

    sqlite.initialise_devql_schema()?;

    sqlite.with_write_connection(|conn| {
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

    sqlite.with_write_connection(|conn| {
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

#[test]
fn initialise_devql_schema_cuts_over_legacy_historical_artefacts_shape() -> Result<()> {
    let temp = TempDir::new().context("creating temp dir")?;
    let sqlite_path = temp.path().join("devql.sqlite");
    let sqlite = SqliteConnectionPool::connect(sqlite_path)?;

    sqlite.execute_batch(
        "CREATE TABLE artefacts (
            artefact_id TEXT PRIMARY KEY,
            symbol_id TEXT,
            repo_id TEXT NOT NULL,
            blob_sha TEXT NOT NULL,
            path TEXT NOT NULL,
            language TEXT NOT NULL,
            canonical_kind TEXT,
            language_kind TEXT,
            symbol_fqn TEXT,
            parent_artefact_id TEXT,
            start_line INTEGER NOT NULL,
            end_line INTEGER NOT NULL,
            start_byte INTEGER NOT NULL,
            end_byte INTEGER NOT NULL,
            signature TEXT,
            modifiers TEXT NOT NULL DEFAULT '[]',
            docstring TEXT,
            content_hash TEXT,
            created_at TEXT DEFAULT (datetime('now'))
        );
        INSERT INTO artefacts (
            artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind,
            symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature,
            modifiers, docstring, content_hash
        ) VALUES (
            'legacy-a1', 'legacy-s1', 'repo-legacy', 'blob-1', 'src/main.rs', 'rust', 'function',
            'function', 'src::main::f', NULL, 10, 20, 100, 200, 'f()', '[]', 'docs', 'hash-1'
        );",
    )?;

    sqlite.initialise_devql_schema()?;

    let has_legacy_blob_sha =
        sqlite.with_connection(|conn| sqlite_table_has_column(conn, "artefacts", "blob_sha"))?;
    assert!(
        !has_legacy_blob_sha,
        "artefacts.blob_sha should be removed after cutover"
    );

    let has_legacy_start_line =
        sqlite.with_connection(|conn| sqlite_table_has_column(conn, "artefacts", "start_line"))?;
    assert!(
        !has_legacy_start_line,
        "artefacts.start_line should be removed after cutover"
    );

    let snapshots_row_count: i64 = sqlite.with_connection(|conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM artefact_snapshots WHERE repo_id = 'repo-legacy' AND artefact_id = 'legacy-a1' AND blob_sha = 'blob-1'",
            [],
            |row| row.get(0),
        )
        .map_err(anyhow::Error::from)
    })?;
    assert_eq!(
        snapshots_row_count, 1,
        "cutover should backfill one artefact snapshot row from legacy artefacts placement columns"
    );

    let historical_view_sql: String = sqlite.with_connection(|conn| {
        conn.query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'view' AND name = 'artefacts_historical'",
            [],
            |row| row.get(0),
        )
        .map_err(anyhow::Error::from)
    })?;
    assert!(
        !historical_view_sql.contains("UNION ALL"),
        "artefacts_historical should be recreated as join-only view after cutover"
    );

    Ok(())
}
