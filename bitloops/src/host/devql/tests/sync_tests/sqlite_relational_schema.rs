use rusqlite::Connection;
use tempfile::tempdir;

#[tokio::test]
async fn sync_schema_creates_all_tables() {
    let temp = tempdir().expect("temp dir");
    let db_path = temp.path().join("devql.sqlite");
    let db = Connection::open(&db_path).expect("open sqlite db");
    db.execute_batch(
        r#"
CREATE TABLE current_file_state (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    committed_at TEXT NOT NULL,
    updated_at TEXT DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, path)
);
"#,
    )
    .expect("seed legacy current_file_state");
    drop(db);

    crate::host::devql::init_sqlite_schema(&db_path)
        .await
        .expect("initialise sqlite relational schema");

    let db = Connection::open(&db_path).expect("open sqlite db");

    for table in &[
        "repo_sync_state",
        "current_file_state",
        "artefacts_current",
        "artefact_edges_current",
        "content_cache",
        "content_cache_artefacts",
        "content_cache_edges",
    ] {
        let count: i64 = db
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='{table}'"
                ),
                [],
                |row| row.get(0),
            )
            .expect("read sqlite_master");
        assert_eq!(count, 1, "table {table} should exist");
    }

    let legacy_sync_state_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='sync_state'",
            [],
            |row| row.get(0),
        )
        .expect("read sqlite_master for sync_state");
    assert_eq!(
        legacy_sync_state_count, 1,
        "legacy sync_state table should still exist"
    );

    for column in &[
        "file_role",
        "text_index_mode",
        "head_content_id",
        "index_content_id",
        "worktree_content_id",
        "effective_content_id",
        "effective_source",
        "parser_version",
        "extractor_version",
        "exists_in_head",
        "exists_in_index",
        "exists_in_worktree",
        "last_synced_at",
    ] {
        let column_count: i64 = db
            .query_row(
                "SELECT COUNT(*) \
                 FROM pragma_table_info('current_file_state') \
                 WHERE name = ?1",
                [column],
                |row| row.get(0),
            )
            .expect("read pragma_table_info");
        assert_eq!(
            column_count, 1,
            "column {column} should exist on current_file_state"
        );
    }

    for table in &[
        "repo_sync_state",
        "current_file_state",
        "artefacts_current",
        "artefact_edges_current",
    ] {
        let mut stmt = db
            .prepare(&format!("PRAGMA foreign_key_list({table})"))
            .expect("prepare foreign_key_list");
        let fk_rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(6)?,
                ))
            })
            .expect("query foreign_key_list")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect foreign keys");
        assert!(
            fk_rows
                .iter()
                .any(|(referenced_table, from_column, to_column, on_delete)| {
                    referenced_table == "repositories"
                        && from_column == "repo_id"
                        && to_column == "repo_id"
                        && on_delete.eq_ignore_ascii_case("CASCADE")
                }),
            "table {table} should reference repositories(repo_id) with ON DELETE CASCADE"
        );
    }
}

#[test]
fn sync_artefacts_current_migration_sql_recreates_current_state_tables() {
    let sql = crate::host::devql::sync::schema::sync_artefacts_current_migration_sql();

    assert!(sql.contains("DROP TABLE IF EXISTS artefacts_current;"));
    assert!(sql.contains("DROP TABLE IF EXISTS artefact_edges_current;"));

    assert!(sql.contains("CREATE TABLE IF NOT EXISTS artefacts_current ("));
    assert!(sql.contains("content_id TEXT NOT NULL"));
    assert!(sql.contains("modifiers TEXT NOT NULL DEFAULT '[]'"));
    assert!(sql.contains("PRIMARY KEY (repo_id, path, symbol_id)"));
    assert!(sql.contains("UNIQUE (repo_id, artefact_id)"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefacts_current_path_idx"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefacts_current_kind_idx"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefacts_current_fqn_idx"));
    assert!(!sql.contains("branch TEXT"));
    assert!(!sql.contains("commit_sha"));
    assert!(!sql.contains("revision_kind"));
    assert!(!sql.contains("revision_id"));
    assert!(!sql.contains("temp_checkpoint_id"));
    assert!(!sql.contains("blob_sha"));

    assert!(sql.contains("CREATE TABLE IF NOT EXISTS artefact_edges_current ("));
    assert!(sql.contains("metadata TEXT NOT NULL DEFAULT '{}'"));
    assert!(sql.contains("PRIMARY KEY (repo_id, edge_id)"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefact_edges_current_path_idx"));
    assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefact_edges_current_from_idx"));
    assert!(!sql.contains("artefact_edges_current_branch_from_idx"));
    assert!(!sql.contains("artefact_edges_current_branch_to_idx"));
    assert!(!sql.contains("JSONB"));
}
