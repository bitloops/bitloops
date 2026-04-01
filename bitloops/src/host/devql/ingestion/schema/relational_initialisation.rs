use super::*;

pub(crate) async fn init_sqlite_schema(sqlite_path: &Path) -> Result<()> {
    let sqlite = crate::storage::SqliteConnectionPool::connect(sqlite_path.to_path_buf())
        .context("connecting SQLite pool for current-state schema migrations")?;
    sqlite
        .initialise_devql_schema()
        .context("creating SQLite relational DevQL tables")?;
    sqlite_exec_path_allow_create(
        sqlite_path,
        crate::host::devql::sync::schema::sync_schema_sql(),
    )
    .await
    .context("creating SQLite DevQL sync tables")?;
    let sync_tables_need_rebuild = sqlite
        .with_connection(|conn| Ok(sync_tables_need_rebuild(conn)?))
        .context("inspecting SQLite DevQL sync table shape")?;
    if sync_tables_need_rebuild {
        sqlite_exec_path_allow_create(
            sqlite_path,
            crate::host::devql::sync::schema::sync_current_file_state_migration_sql(),
        )
        .await
        .context("rebuilding SQLite sync current_file_state table")?;
        sqlite_exec_path_allow_create(
            sqlite_path,
            crate::host::devql::sync::schema::sync_artefacts_current_migration_sql(),
        )
        .await
        .context("rebuilding SQLite current-state sync tables")?;
    }
    sqlite_exec_path_allow_create(sqlite_path, edge_model_cleanup_sqlite_sql())
        .await
        .context("normalising SQLite DevQL edge model values")?;
    sqlite_exec_path_allow_create(sqlite_path, checkpoint_schema_sql_sqlite())
        .await
        .context("creating SQLite checkpoint migration tables")?;
    crate::capability_packs::semantic_clones::init_sqlite_semantic_features_schema(sqlite_path)
        .await
        .context("creating SQLite semantic feature tables")?;
    crate::capability_packs::semantic_clones::init_sqlite_semantic_embeddings_schema(sqlite_path)
        .await
        .context("creating SQLite semantic embedding tables")?;
    Ok(())
}

fn sync_tables_need_rebuild(conn: &rusqlite::Connection) -> Result<bool> {
    Ok(!current_file_state_matches_new_shape(conn)?
        || !artefacts_current_matches_new_shape(conn)?
        || !artefact_edges_current_matches_new_shape(conn)?)
}

fn current_file_state_matches_new_shape(conn: &rusqlite::Connection) -> Result<bool> {
    let expected_columns = [
        "repo_id",
        "path",
        "language",
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
    ];
    Ok(
        sqlite_table_columns(conn, "current_file_state")? == expected_columns
            && sqlite_table_pk_columns(conn, "current_file_state")?
                == vec!["repo_id".to_string(), "path".to_string()],
    )
}

fn artefacts_current_matches_new_shape(conn: &rusqlite::Connection) -> Result<bool> {
    let expected_columns = [
        "repo_id",
        "path",
        "content_id",
        "symbol_id",
        "artefact_id",
        "language",
        "canonical_kind",
        "language_kind",
        "symbol_fqn",
        "parent_symbol_id",
        "parent_artefact_id",
        "start_line",
        "end_line",
        "start_byte",
        "end_byte",
        "signature",
        "modifiers",
        "docstring",
        "updated_at",
    ];
    Ok(
        sqlite_table_columns(conn, "artefacts_current")? == expected_columns
            && sqlite_table_pk_columns(conn, "artefacts_current")?
                == vec![
                    "repo_id".to_string(),
                    "path".to_string(),
                    "symbol_id".to_string(),
                ],
    )
}

fn artefact_edges_current_matches_new_shape(conn: &rusqlite::Connection) -> Result<bool> {
    let expected_columns = [
        "repo_id",
        "edge_id",
        "path",
        "content_id",
        "from_symbol_id",
        "from_artefact_id",
        "to_symbol_id",
        "to_artefact_id",
        "to_symbol_ref",
        "edge_kind",
        "language",
        "start_line",
        "end_line",
        "metadata",
        "updated_at",
    ];
    Ok(
        sqlite_table_columns(conn, "artefact_edges_current")? == expected_columns
            && sqlite_table_pk_columns(conn, "artefact_edges_current")?
                == vec!["repo_id".to_string(), "edge_id".to_string()],
    )
}

fn sqlite_table_columns(conn: &rusqlite::Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .with_context(|| format!("preparing PRAGMA table_info for `{table}`"))?;
    let mut rows = stmt
        .query([])
        .with_context(|| format!("querying PRAGMA table_info for `{table}`"))?;
    let mut columns = Vec::new();
    while let Some(row) = rows.next().context("reading PRAGMA row")? {
        let name: String = row
            .get(1)
            .with_context(|| format!("reading column name from `{table}`"))?;
        columns.push(name);
    }
    Ok(columns)
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

pub(crate) async fn init_postgres_schema(
    _cfg: &DevqlConfig,
    pg_client: &tokio_postgres::Client,
) -> Result<()> {
    let sql = postgres_schema_sql();
    postgres_exec(pg_client, sql)
        .await
        .context("creating Postgres DevQL tables")?;

    let artefacts_alter_sql = artefacts_upgrade_sql();
    postgres_exec(pg_client, artefacts_alter_sql)
        .await
        .context("updating Postgres artefacts columns for byte offsets/signature")?;

    let artefact_edges_hardening_sql = artefact_edges_hardening_sql();
    postgres_exec(pg_client, artefact_edges_hardening_sql)
        .await
        .context("updating Postgres artefact_edges constraints/indexes")?;

    let edge_model_cleanup_sql = edge_model_cleanup_postgres_sql();
    postgres_exec(pg_client, edge_model_cleanup_sql)
        .await
        .context("normalising Postgres DevQL edge model values")?;

    postgres_exec(
        pg_client,
        crate::host::devql::sync::schema::sync_schema_sql(),
    )
    .await
    .context("creating Postgres DevQL sync tables")?;

    postgres_exec(
        pg_client,
        crate::host::devql::sync::schema::sync_current_file_state_migration_sql(),
    )
    .await
    .context("rebuilding Postgres sync current_file_state table")?;

    postgres_exec(
        pg_client,
        crate::host::devql::sync::schema::sync_artefacts_current_migration_sql(),
    )
    .await
    .context("rebuilding Postgres current-state sync tables")?;

    crate::capability_packs::semantic_clones::init_postgres_semantic_features_schema(pg_client)
        .await
        .context("creating Postgres semantic feature tables")?;
    crate::capability_packs::semantic_clones::init_postgres_semantic_embeddings_schema(pg_client)
        .await
        .context("creating Postgres semantic embedding tables")?;
    crate::capability_packs::semantic_clones::pipeline::init_postgres_semantic_clones_schema(
        pg_client,
    )
    .await
    .context("creating Postgres semantic clone tables")?;
    let checkpoint_schema_sql = checkpoint_schema_sql_postgres();
    postgres_exec(pg_client, checkpoint_schema_sql)
        .await
        .context("creating Postgres checkpoint migration tables")?;

    let test_links_upgrade_sql = test_links_upgrade_sql();
    postgres_exec(pg_client, test_links_upgrade_sql)
        .await
        .context("adding confidence/linkage_status columns to test_links")?;

    let workspace_revisions_sql = workspace_revisions_sql();
    postgres_exec(pg_client, workspace_revisions_sql)
        .await
        .context("creating workspace_revisions table")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn init_sqlite_schema_preserves_sync_state_on_repeated_runs() {
        let temp = TempDir::new().expect("temp dir");
        let sqlite_path = temp.path().join("devql.sqlite");

        init_sqlite_schema(&sqlite_path)
            .await
            .expect("initialise SQLite DevQL schema");

        let sqlite = crate::storage::SqliteConnectionPool::connect_existing(sqlite_path.clone())
            .expect("open existing sqlite db");
        sqlite
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO current_file_state (
                        repo_id, path, language, head_content_id, index_content_id,
                        worktree_content_id, effective_content_id, effective_source,
                        parser_version, extractor_version, exists_in_head,
                        exists_in_index, exists_in_worktree, last_synced_at
                    ) VALUES (
                        'repo-1', 'src/lib.rs', 'rust', NULL, NULL, NULL,
                        'content-1', 'head', 'parser-v1', 'extractor-v1',
                        1, 1, 1, datetime('now')
                    )",
                    [],
                )
                .expect("insert current_file_state row");
                Ok(())
            })
            .expect("seed current_file_state row");

        init_sqlite_schema(&sqlite_path)
            .await
            .expect("re-run SQLite DevQL schema initialisation");

        let sqlite = crate::storage::SqliteConnectionPool::connect_existing(sqlite_path)
            .expect("reopen existing sqlite db");
        let row_count: i64 = sqlite
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM current_file_state WHERE repo_id = 'repo-1' AND path = 'src/lib.rs'",
                    [],
                    |row| row.get(0),
                )
                .map_err(anyhow::Error::from)
            })
            .expect("count preserved current_file_state row");

        assert_eq!(row_count, 1);
    }
}
