use super::*;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PostgresSyncSchemaInitOutcome {
    pub(crate) rebuilt_current_state: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PostgresSyncSchemaPolicy {
    SafeBootstrap,
    SyncExecution,
}

pub(crate) async fn init_sqlite_schema(sqlite_path: &Path) -> Result<()> {
    if crate::host::devql::types::sqlite_path_uses_remote_shared_relational_authority(sqlite_path) {
        warn_on_legacy_shared_sqlite_tables(sqlite_path)?;
        return init_sqlite_current_projection_schema(sqlite_path).await;
    }

    let sqlite = crate::storage::SqliteConnectionPool::connect(sqlite_path.to_path_buf())
        .context("connecting SQLite pool for current-state schema migrations")?;
    sqlite
        .initialise_devql_schema()
        .context("creating SQLite relational DevQL tables")?;
    let repository_columns = sqlite
        .with_connection(|conn| sqlite_table_columns(conn, "repositories"))
        .context("inspecting SQLite repositories table shape")?;
    if !repository_columns
        .iter()
        .any(|column| column == "metadata_json")
    {
        sqlite_exec_path_allow_create(
            sqlite_path,
            "ALTER TABLE repositories ADD COLUMN metadata_json TEXT;",
        )
        .await
        .context("adding SQLite repositories.metadata_json column")?;
    }
    let historical_cutover_needed = sqlite
        .with_connection(sqlite_artefacts_historical_needs_cutover)
        .context("inspecting SQLite historical artefacts schema shape")?;
    if historical_cutover_needed {
        sqlite_exec_path_allow_create(sqlite_path, historical_artefacts_cutover_sqlite_sql())
            .await
            .context("applying SQLite historical artefacts one-shot cutover")?;
    }
    sqlite_exec_path_allow_create(
        sqlite_path,
        crate::host::devql::sync::schema::sync_schema_sql(),
    )
    .await
    .context("creating SQLite DevQL sync tables")?;
    let sync_tables_need_rebuild = sqlite
        .with_connection(sync_tables_need_rebuild)
        .context("inspecting SQLite DevQL sync table shape")?;
    if sync_tables_need_rebuild {
        sqlite_exec_path_allow_create(
            sqlite_path,
            crate::host::devql::sync::schema::sync_repo_sync_state_migration_sql(),
        )
        .await
        .context("rebuilding SQLite sync repo_sync_state table")?;
        sqlite_exec_path_allow_create(
            sqlite_path,
            crate::host::devql::sync::schema::sync_project_contexts_current_migration_sql(),
        )
        .await
        .context("rebuilding SQLite sync project_contexts_current table")?;
        sqlite_exec_path_allow_create(
            sqlite_path,
            crate::host::devql::sync::schema::sync_current_file_state_migration_sql(),
        )
        .await
        .context("rebuilding SQLite sync current_file_state table")?;
        sqlite_exec_path_allow_create(
            sqlite_path,
            crate::host::devql::sync::schema::sync_content_cache_migration_sql(),
        )
        .await
        .context("rebuilding SQLite sync content cache tables")?;
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
    sqlite_exec_path_allow_create(sqlite_path, checkpoint_relational_schema_sql_sqlite())
        .await
        .context("creating SQLite checkpoint migration tables")?;
    crate::capability_packs::semantic_clones::init_sqlite_semantic_features_schema(sqlite_path)
        .await
        .context("creating SQLite semantic feature tables")?;
    crate::capability_packs::semantic_clones::init_sqlite_search_documents_schema(sqlite_path)
        .await
        .context("creating SQLite search document tables")?;
    crate::capability_packs::semantic_clones::init_sqlite_semantic_embeddings_schema(sqlite_path)
        .await
        .context("creating SQLite semantic embedding tables")?;
    sqlite_exec_path_allow_create(
        sqlite_path,
        crate::capability_packs::semantic_clones::schema::semantic_clones_sqlite_schema_sql(),
    )
    .await
    .context("creating SQLite semantic clone tables")?;
    Ok(())
}

const LEGACY_SHARED_SQLITE_TABLES: &[&str] = &[
    "sync_state",
    "commits",
    "commit_ingest_ledger",
    "file_state",
    "artefact_snapshots",
    "artefacts",
    "artefact_edges",
    "checkpoint_files",
    "checkpoint_artefacts",
    "checkpoint_artefact_lineage",
    "symbol_semantics",
    "symbol_features",
    "symbol_embeddings",
    "symbol_clone_edges",
];

fn warn_on_legacy_shared_sqlite_tables(sqlite_path: &Path) -> Result<()> {
    if !sqlite_path.is_file() {
        return Ok(());
    }

    let sqlite = crate::storage::SqliteConnectionPool::connect_existing(sqlite_path.to_path_buf())
        .context("opening SQLite current/projection database to inspect legacy shared tables")?;
    let legacy_tables = sqlite
        .with_connection(detect_legacy_shared_sqlite_tables)
        .context("inspecting SQLite for legacy shared-table mirrors")?;
    if !legacy_tables.is_empty() {
        log::warn!(
            "remote shared relational authority is configured, but the local SQLite file still contains legacy shared tables that are now inert: {}",
            legacy_tables.join(", ")
        );
    }
    Ok(())
}

pub(crate) async fn init_sqlite_current_projection_schema(sqlite_path: &Path) -> Result<()> {
    let repositories_only_sql =
        build_schema_subset_sql(sqlite_shared_schema_sql(), &["repositories"]);
    sqlite_exec_path_allow_create(sqlite_path, &repositories_only_sql)
        .await
        .context("creating SQLite local repo catalog helper table")?;
    sqlite_exec_path_allow_create(sqlite_path, sqlite_current_projection_schema_sql())
        .await
        .context("creating SQLite current/projection relational tables")?;
    sqlite_exec_path_allow_create(
        sqlite_path,
        crate::host::devql::sync::schema::sync_schema_sql(),
    )
    .await
    .context("creating SQLite current/projection sync tables")?;
    let sqlite = crate::storage::SqliteConnectionPool::connect(sqlite_path.to_path_buf())
        .context("connecting SQLite pool for current/projection schema migrations")?;
    let sync_tables_need_rebuild = sqlite
        .with_connection(sync_tables_need_rebuild)
        .context("inspecting SQLite current/projection sync table shape")?;
    if sync_tables_need_rebuild {
        sqlite_exec_path_allow_create(
            sqlite_path,
            crate::host::devql::sync::schema::sync_repo_sync_state_migration_sql(),
        )
        .await
        .context("rebuilding SQLite current/projection repo_sync_state table")?;
        sqlite_exec_path_allow_create(
            sqlite_path,
            crate::host::devql::sync::schema::sync_project_contexts_current_migration_sql(),
        )
        .await
        .context("rebuilding SQLite current/projection project_contexts_current table")?;
        sqlite_exec_path_allow_create(
            sqlite_path,
            crate::host::devql::sync::schema::sync_current_file_state_migration_sql(),
        )
        .await
        .context("rebuilding SQLite current/projection current_file_state table")?;
        sqlite_exec_path_allow_create(
            sqlite_path,
            crate::host::devql::sync::schema::sync_content_cache_migration_sql(),
        )
        .await
        .context("rebuilding SQLite current/projection content cache tables")?;
        sqlite_exec_path_allow_create(
            sqlite_path,
            crate::host::devql::sync::schema::sync_artefacts_current_migration_sql(),
        )
        .await
        .context("rebuilding SQLite current/projection artefact tables")?;
    }
    crate::capability_packs::semantic_clones::init_sqlite_semantic_features_schema(sqlite_path)
        .await
        .context("creating SQLite current semantic feature tables")?;
    crate::capability_packs::semantic_clones::init_sqlite_search_documents_schema(sqlite_path)
        .await
        .context("creating SQLite current search document tables")?;
    crate::capability_packs::semantic_clones::init_sqlite_semantic_embeddings_schema(sqlite_path)
        .await
        .context("creating SQLite current semantic embedding tables")?;
    sqlite_exec_path_allow_create(
        sqlite_path,
        crate::capability_packs::semantic_clones::schema::semantic_clones_sqlite_current_projection_schema_sql(),
    )
    .await
    .context("creating SQLite current semantic clone tables")?;
    Ok(())
}

fn sqlite_artefacts_historical_needs_cutover(conn: &rusqlite::Connection) -> Result<bool> {
    let columns = sqlite_table_columns(conn, "artefacts")?;
    Ok([
        "blob_sha",
        "path",
        "parent_artefact_id",
        "start_line",
        "end_line",
        "start_byte",
        "end_byte",
    ]
    .iter()
    .any(|column| columns.iter().any(|existing| existing == column)))
}

fn detect_legacy_shared_sqlite_tables(conn: &rusqlite::Connection) -> Result<Vec<String>> {
    let mut found = Vec::new();
    for table in LEGACY_SHARED_SQLITE_TABLES {
        if sqlite_table_exists(conn, table)? {
            found.push((*table).to_string());
        }
    }
    Ok(found)
}

fn sync_tables_need_rebuild(conn: &rusqlite::Connection) -> Result<bool> {
    Ok(!repo_sync_state_matches_new_shape(conn)?
        || !project_contexts_current_matches_new_shape(conn)?
        || !current_file_state_matches_new_shape(conn)?
        || !content_cache_matches_new_shape(conn)?
        || !artefacts_current_matches_new_shape(conn)?
        || !artefact_edges_current_matches_new_shape(conn)?)
}

fn project_contexts_current_matches_new_shape(conn: &rusqlite::Connection) -> Result<bool> {
    let expected_columns = [
        "repo_id",
        "context_id",
        "root",
        "kind",
        "detection_source",
        "frameworks_json",
        "runtime_profile",
        "config_files_json",
        "config_fingerprint",
        "source_versions_json",
    ];
    Ok(
        sqlite_table_columns(conn, "project_contexts_current")? == expected_columns
            && sqlite_table_pk_columns(conn, "project_contexts_current")?
                == vec!["repo_id".to_string(), "context_id".to_string()]
            && sqlite_table_has_repo_catalog_fk(conn, "project_contexts_current")?,
    )
}

fn repo_sync_state_matches_new_shape(conn: &rusqlite::Connection) -> Result<bool> {
    let expected_columns = [
        "repo_id",
        "repo_root",
        "active_branch",
        "head_commit_sha",
        "head_tree_sha",
        "parser_version",
        "extractor_version",
        "scope_exclusions_fingerprint",
        "last_sync_started_at",
        "last_sync_completed_at",
        "last_sync_status",
        "last_sync_reason",
    ];
    Ok(
        sqlite_table_columns(conn, "repo_sync_state")? == expected_columns
            && sqlite_table_pk_columns(conn, "repo_sync_state")? == vec!["repo_id".to_string()]
            && sqlite_table_has_repo_catalog_fk(conn, "repo_sync_state")?,
    )
}

fn current_file_state_matches_new_shape(conn: &rusqlite::Connection) -> Result<bool> {
    let expected_columns = [
        "repo_id",
        "path",
        "analysis_mode",
        "file_role",
        "text_index_mode",
        "language",
        "resolved_language",
        "dialect",
        "primary_context_id",
        "secondary_context_ids_json",
        "frameworks_json",
        "runtime_profile",
        "classification_reason",
        "context_fingerprint",
        "extraction_fingerprint",
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
                == vec!["repo_id".to_string(), "path".to_string()]
            && sqlite_table_has_repo_catalog_fk(conn, "current_file_state")?,
    )
}

fn content_cache_matches_new_shape(conn: &rusqlite::Connection) -> Result<bool> {
    let cache_expected = [
        "content_id",
        "language",
        "extraction_fingerprint",
        "parser_version",
        "extractor_version",
        "retention_class",
        "parse_status",
        "parsed_at",
        "last_accessed_at",
    ];
    let artefacts_expected = [
        "content_id",
        "language",
        "extraction_fingerprint",
        "parser_version",
        "extractor_version",
        "artifact_key",
        "canonical_kind",
        "language_kind",
        "name",
        "parent_artifact_key",
        "start_line",
        "end_line",
        "start_byte",
        "end_byte",
        "signature",
        "modifiers",
        "docstring",
        "metadata",
    ];
    let edges_expected = [
        "content_id",
        "language",
        "extraction_fingerprint",
        "parser_version",
        "extractor_version",
        "edge_key",
        "from_artifact_key",
        "to_artifact_key",
        "to_symbol_ref",
        "edge_kind",
        "start_line",
        "end_line",
        "metadata",
    ];
    Ok(
        sqlite_table_columns(conn, "content_cache")? == cache_expected
            && sqlite_table_pk_columns(conn, "content_cache")?
                == vec![
                    "content_id".to_string(),
                    "language".to_string(),
                    "extraction_fingerprint".to_string(),
                    "parser_version".to_string(),
                    "extractor_version".to_string(),
                ]
            && sqlite_table_columns(conn, "content_cache_artefacts")? == artefacts_expected
            && sqlite_table_pk_columns(conn, "content_cache_artefacts")?
                == vec![
                    "content_id".to_string(),
                    "language".to_string(),
                    "extraction_fingerprint".to_string(),
                    "parser_version".to_string(),
                    "extractor_version".to_string(),
                    "artifact_key".to_string(),
                ]
            && sqlite_table_columns(conn, "content_cache_edges")? == edges_expected
            && sqlite_table_pk_columns(conn, "content_cache_edges")?
                == vec![
                    "content_id".to_string(),
                    "language".to_string(),
                    "extraction_fingerprint".to_string(),
                    "parser_version".to_string(),
                    "extractor_version".to_string(),
                    "edge_key".to_string(),
                ],
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
        "extraction_fingerprint",
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
                ]
            && sqlite_table_has_repo_catalog_fk(conn, "artefacts_current")?,
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
                == vec!["repo_id".to_string(), "edge_id".to_string()]
            && sqlite_table_has_repo_catalog_fk(conn, "artefact_edges_current")?,
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

fn sqlite_table_exists(conn: &rusqlite::Connection, table: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
            SELECT 1
            FROM sqlite_master
            WHERE type IN ('table', 'view') AND name = ?1
        )",
        [table],
        |row| row.get::<_, i64>(0),
    )
    .map(|exists| exists != 0)
    .map_err(anyhow::Error::from)
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

fn sqlite_table_has_repo_catalog_fk(conn: &rusqlite::Connection, table: &str) -> Result<bool> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA foreign_key_list({table})"))
        .with_context(|| format!("preparing PRAGMA foreign_key_list for `{table}`"))?;
    let mut rows = stmt
        .query([])
        .with_context(|| format!("querying PRAGMA foreign_key_list for `{table}`"))?;
    while let Some(row) = rows.next().context("reading PRAGMA foreign key row")? {
        let referenced_table: String = row
            .get(2)
            .with_context(|| format!("reading referenced table from `{table}` foreign key"))?;
        let from_column: String = row
            .get(3)
            .with_context(|| format!("reading source column from `{table}` foreign key"))?;
        let to_column: String = row
            .get(4)
            .with_context(|| format!("reading target column from `{table}` foreign key"))?;
        let on_delete: String = row
            .get(6)
            .with_context(|| format!("reading on_delete action from `{table}` foreign key"))?;
        if referenced_table == "repositories"
            && from_column == "repo_id"
            && to_column == "repo_id"
            && on_delete.eq_ignore_ascii_case("CASCADE")
        {
            return Ok(true);
        }
    }

    Ok(false)
}

pub(crate) async fn init_postgres_schema(
    _cfg: &DevqlConfig,
    pg_client: &tokio_postgres::Client,
) -> Result<PostgresSyncSchemaInitOutcome> {
    init_postgres_schema_with_policy(pg_client, PostgresSyncSchemaPolicy::SafeBootstrap).await
}

pub(crate) async fn init_postgres_schema_for_sync_execution(
    _cfg: &DevqlConfig,
    pg_client: &tokio_postgres::Client,
) -> Result<PostgresSyncSchemaInitOutcome> {
    init_postgres_schema_with_policy(pg_client, PostgresSyncSchemaPolicy::SyncExecution).await
}

async fn init_postgres_schema_with_policy(
    pg_client: &tokio_postgres::Client,
    _policy: PostgresSyncSchemaPolicy,
) -> Result<PostgresSyncSchemaInitOutcome> {
    let sql = postgres_shared_schema_sql();
    postgres_exec(pg_client, sql)
        .await
        .context("creating Postgres shared DevQL tables")?;
    postgres_exec(
        pg_client,
        "ALTER TABLE repositories ADD COLUMN IF NOT EXISTS metadata_json TEXT;",
    )
    .await
    .context("adding Postgres repositories.metadata_json column")?;

    let artefacts_alter_sql = artefacts_upgrade_sql();
    postgres_exec(pg_client, artefacts_alter_sql)
        .await
        .context("updating Postgres artefacts semantic columns")?;

    let historical_cutover_sql = historical_artefacts_cutover_postgres_sql();
    postgres_exec(pg_client, historical_cutover_sql)
        .await
        .context("applying Postgres historical artefacts one-shot cutover")?;

    let artefact_edges_hardening_sql = artefact_edges_hardening_sql();
    postgres_exec(pg_client, artefact_edges_hardening_sql)
        .await
        .context("updating Postgres artefact_edges constraints/indexes")?;

    let edge_model_cleanup_sql = edge_model_cleanup_postgres_sql();
    postgres_exec(pg_client, edge_model_cleanup_sql)
        .await
        .context("normalising Postgres DevQL edge model values")?;

    crate::capability_packs::semantic_clones::init_postgres_semantic_features_schema(pg_client)
        .await
        .context("creating Postgres shared semantic feature tables")?;
    crate::capability_packs::semantic_clones::init_postgres_search_documents_schema(pg_client)
        .await
        .context("creating Postgres shared search document tables")?;
    crate::capability_packs::semantic_clones::init_postgres_semantic_embeddings_schema(pg_client)
        .await
        .context("creating Postgres shared semantic embedding tables")?;
    postgres_exec(
        pg_client,
        crate::capability_packs::semantic_clones::schema::semantic_clones_postgres_shared_schema_sql(),
    )
    .await
    .context("creating Postgres shared semantic clone tables")?;
    let checkpoint_schema_sql = checkpoint_relational_schema_sql_postgres();
    postgres_exec(pg_client, checkpoint_schema_sql)
        .await
        .context("creating Postgres checkpoint migration tables")?;

    let test_links_upgrade_sql = test_links_upgrade_sql();
    postgres_exec(pg_client, test_links_upgrade_sql)
        .await
        .context("adding confidence/linkage_status columns to test_links")?;

    Ok(PostgresSyncSchemaInitOutcome::default())
}

async fn postgres_sync_tables_need_rebuild(pg_client: &tokio_postgres::Client) -> Result<bool> {
    Ok(
        !postgres_repo_sync_state_matches_new_shape(pg_client).await?
            || !postgres_project_contexts_current_matches_new_shape(pg_client).await?
            || !postgres_current_file_state_matches_new_shape(pg_client).await?
            || !postgres_content_cache_matches_new_shape(pg_client).await?
            || !postgres_artefacts_current_matches_new_shape(pg_client).await?
            || !postgres_artefact_edges_current_matches_new_shape(pg_client).await?,
    )
}

async fn postgres_sync_tables_are_empty(pg_client: &tokio_postgres::Client) -> Result<bool> {
    for table in [
        "repo_sync_state",
        "current_file_state",
        "artefacts_current",
        "artefact_edges_current",
    ] {
        if postgres_table_has_rows(pg_client, table).await? {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn postgres_repo_sync_state_matches_new_shape(
    pg_client: &tokio_postgres::Client,
) -> Result<bool> {
    let expected_columns = [
        "repo_id",
        "repo_root",
        "active_branch",
        "head_commit_sha",
        "head_tree_sha",
        "parser_version",
        "extractor_version",
        "scope_exclusions_fingerprint",
        "last_sync_started_at",
        "last_sync_completed_at",
        "last_sync_status",
        "last_sync_reason",
    ];
    Ok(
        postgres_table_columns(pg_client, "repo_sync_state").await? == expected_columns
            && postgres_table_pk_columns(pg_client, "repo_sync_state").await?
                == vec!["repo_id".to_string()]
            && postgres_table_has_repo_catalog_fk(pg_client, "repo_sync_state").await?,
    )
}

async fn postgres_current_file_state_matches_new_shape(
    pg_client: &tokio_postgres::Client,
) -> Result<bool> {
    let expected_columns = [
        "repo_id",
        "path",
        "analysis_mode",
        "file_role",
        "text_index_mode",
        "language",
        "resolved_language",
        "dialect",
        "primary_context_id",
        "secondary_context_ids_json",
        "frameworks_json",
        "runtime_profile",
        "classification_reason",
        "context_fingerprint",
        "extraction_fingerprint",
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
        postgres_table_columns(pg_client, "current_file_state").await? == expected_columns
            && postgres_table_pk_columns(pg_client, "current_file_state").await?
                == vec!["repo_id".to_string(), "path".to_string()]
            && postgres_table_has_repo_catalog_fk(pg_client, "current_file_state").await?,
    )
}

async fn postgres_project_contexts_current_matches_new_shape(
    pg_client: &tokio_postgres::Client,
) -> Result<bool> {
    let expected_columns = [
        "repo_id",
        "context_id",
        "root",
        "kind",
        "detection_source",
        "frameworks_json",
        "runtime_profile",
        "config_files_json",
        "config_fingerprint",
        "source_versions_json",
    ];
    Ok(
        postgres_table_columns(pg_client, "project_contexts_current").await? == expected_columns
            && postgres_table_pk_columns(pg_client, "project_contexts_current").await?
                == vec!["repo_id".to_string(), "context_id".to_string()]
            && postgres_table_has_repo_catalog_fk(pg_client, "project_contexts_current").await?,
    )
}

async fn postgres_content_cache_matches_new_shape(
    pg_client: &tokio_postgres::Client,
) -> Result<bool> {
    let cache_expected = [
        "content_id",
        "language",
        "extraction_fingerprint",
        "parser_version",
        "extractor_version",
        "retention_class",
        "parse_status",
        "parsed_at",
        "last_accessed_at",
    ];
    let artefacts_expected = [
        "content_id",
        "language",
        "extraction_fingerprint",
        "parser_version",
        "extractor_version",
        "artifact_key",
        "canonical_kind",
        "language_kind",
        "name",
        "parent_artifact_key",
        "start_line",
        "end_line",
        "start_byte",
        "end_byte",
        "signature",
        "modifiers",
        "docstring",
        "metadata",
    ];
    let edges_expected = [
        "content_id",
        "language",
        "extraction_fingerprint",
        "parser_version",
        "extractor_version",
        "edge_key",
        "from_artifact_key",
        "to_artifact_key",
        "to_symbol_ref",
        "edge_kind",
        "start_line",
        "end_line",
        "metadata",
    ];
    Ok(
        postgres_table_columns(pg_client, "content_cache").await? == cache_expected
            && postgres_table_pk_columns(pg_client, "content_cache").await?
                == vec![
                    "content_id".to_string(),
                    "language".to_string(),
                    "extraction_fingerprint".to_string(),
                    "parser_version".to_string(),
                    "extractor_version".to_string(),
                ]
            && postgres_table_columns(pg_client, "content_cache_artefacts").await?
                == artefacts_expected
            && postgres_table_pk_columns(pg_client, "content_cache_artefacts").await?
                == vec![
                    "content_id".to_string(),
                    "language".to_string(),
                    "extraction_fingerprint".to_string(),
                    "parser_version".to_string(),
                    "extractor_version".to_string(),
                    "artifact_key".to_string(),
                ]
            && postgres_table_columns(pg_client, "content_cache_edges").await? == edges_expected
            && postgres_table_pk_columns(pg_client, "content_cache_edges").await?
                == vec![
                    "content_id".to_string(),
                    "language".to_string(),
                    "extraction_fingerprint".to_string(),
                    "parser_version".to_string(),
                    "extractor_version".to_string(),
                    "edge_key".to_string(),
                ],
    )
}

async fn postgres_artefacts_current_matches_new_shape(
    pg_client: &tokio_postgres::Client,
) -> Result<bool> {
    let expected_columns = [
        "repo_id",
        "path",
        "content_id",
        "symbol_id",
        "artefact_id",
        "language",
        "extraction_fingerprint",
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
        postgres_table_columns(pg_client, "artefacts_current").await? == expected_columns
            && postgres_table_pk_columns(pg_client, "artefacts_current").await?
                == vec![
                    "repo_id".to_string(),
                    "path".to_string(),
                    "symbol_id".to_string(),
                ]
            && postgres_table_has_repo_catalog_fk(pg_client, "artefacts_current").await?,
    )
}

async fn postgres_artefact_edges_current_matches_new_shape(
    pg_client: &tokio_postgres::Client,
) -> Result<bool> {
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
        postgres_table_columns(pg_client, "artefact_edges_current").await? == expected_columns
            && postgres_table_pk_columns(pg_client, "artefact_edges_current").await?
                == vec!["repo_id".to_string(), "edge_id".to_string()]
            && postgres_table_has_repo_catalog_fk(pg_client, "artefact_edges_current").await?,
    )
}

async fn postgres_table_columns(
    pg_client: &tokio_postgres::Client,
    table: &str,
) -> Result<Vec<String>> {
    let rows = pg_client
        .query(
            "SELECT column_name
             FROM information_schema.columns
             WHERE table_schema = 'public' AND table_name = $1
             ORDER BY ordinal_position",
            &[&table],
        )
        .await
        .with_context(|| format!("querying Postgres column metadata for `{table}`"))?;
    Ok(rows.into_iter().map(|row| row.get(0)).collect())
}

async fn postgres_table_pk_columns(
    pg_client: &tokio_postgres::Client,
    table: &str,
) -> Result<Vec<String>> {
    let rows = pg_client
        .query(
            "SELECT kcu.column_name
             FROM information_schema.table_constraints tc
             JOIN information_schema.key_column_usage kcu
               ON tc.constraint_name = kcu.constraint_name
              AND tc.table_schema = kcu.table_schema
             WHERE tc.table_schema = 'public'
               AND tc.table_name = $1
               AND tc.constraint_type = 'PRIMARY KEY'
             ORDER BY kcu.ordinal_position",
            &[&table],
        )
        .await
        .with_context(|| format!("querying Postgres primary key metadata for `{table}`"))?;
    Ok(rows.into_iter().map(|row| row.get(0)).collect())
}

async fn postgres_table_has_repo_catalog_fk(
    pg_client: &tokio_postgres::Client,
    table: &str,
) -> Result<bool> {
    let rows = pg_client
        .query(
            "SELECT rc.delete_rule
             FROM information_schema.table_constraints tc
             JOIN information_schema.key_column_usage kcu
               ON tc.constraint_name = kcu.constraint_name
              AND tc.table_schema = kcu.table_schema
             JOIN information_schema.constraint_column_usage ccu
               ON ccu.constraint_name = tc.constraint_name
              AND ccu.table_schema = tc.table_schema
             JOIN information_schema.referential_constraints rc
               ON rc.constraint_name = tc.constraint_name
              AND rc.constraint_schema = tc.table_schema
             WHERE tc.table_schema = 'public'
               AND tc.table_name = $1
               AND tc.constraint_type = 'FOREIGN KEY'
               AND kcu.column_name = 'repo_id'
               AND ccu.table_name = 'repositories'
               AND ccu.column_name = 'repo_id'",
            &[&table],
        )
        .await
        .with_context(|| format!("querying Postgres foreign key metadata for `{table}`"))?;
    Ok(rows
        .into_iter()
        .map(|row| row.get::<_, String>(0))
        .any(|delete_rule| delete_rule.eq_ignore_ascii_case("CASCADE")))
}

async fn postgres_table_has_rows(pg_client: &tokio_postgres::Client, table: &str) -> Result<bool> {
    let row = pg_client
        .query_one(
            &format!("SELECT EXISTS (SELECT 1 FROM {table} LIMIT 1)"),
            &[],
        )
        .await
        .with_context(|| format!("checking whether Postgres table `{table}` contains rows"))?;
    Ok(row.get(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::devql::RelationalPrimaryBackend;
    use tempfile::TempDir;

    #[test]
    fn sync_tables_need_rebuild_true_for_empty_database() {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory sqlite");
        assert!(
            sync_tables_need_rebuild(&conn).expect("inspect empty schema"),
            "missing DevQL sync tables should require rebuild"
        );
    }

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
                    "INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
                     VALUES ('repo-1', 'local', 'local', 'repo-1', 'main')",
                    [],
                )
                .expect("insert repository catalog row");
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

    #[tokio::test]
    async fn init_sqlite_schema_uses_current_projection_split_when_shared_authority_is_remote() {
        let temp = TempDir::new().expect("temp dir");
        let sqlite_path = temp.path().join("devql.sqlite");
        let _relational = crate::host::devql::RelationalStorage::primary_backend_for_tests(
            sqlite_path.clone(),
            RelationalPrimaryBackend::Postgres,
        );

        init_sqlite_schema(&sqlite_path)
            .await
            .expect("initialise split SQLite current/projection schema");

        let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite db");
        let table_exists = |name: &str| -> bool {
            conn.query_row(
                "SELECT EXISTS(
                    SELECT 1
                    FROM sqlite_master
                    WHERE type IN ('table', 'view') AND name = ?1
                )",
                [name],
                |row| row.get::<_, i64>(0),
            )
            .expect("query sqlite_master")
                != 0
        };

        assert!(table_exists("repositories"));
        assert!(table_exists("current_file_state"));
        assert!(table_exists("artefacts_current"));
        assert!(table_exists("symbol_features_current"));
        assert!(table_exists("symbol_semantics_current"));
        assert!(table_exists("symbol_search_documents_current"));
        assert!(table_exists("symbol_embeddings_current"));
        assert!(table_exists("semantic_clone_embedding_setup_state"));
        assert!(table_exists("symbol_clone_edges_current"));
        assert!(table_exists("workspace_revisions"));

        assert!(!table_exists("commits"));
        assert!(!table_exists("artefacts"));
        assert!(!table_exists("symbol_features"));
        assert!(!table_exists("symbol_semantics"));
        assert!(!table_exists("symbol_search_documents"));
        assert!(!table_exists("symbol_embeddings"));
        assert!(!table_exists("semantic_embedding_setups"));
        assert!(!table_exists("symbol_clone_edges"));
    }

    #[test]
    fn detect_legacy_shared_sqlite_tables_reports_inert_shared_tables() {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory sqlite");
        conn.execute("CREATE TABLE sync_state(state_key TEXT)", [])
            .expect("create sync_state");
        conn.execute("CREATE TABLE checkpoint_files(relation_id TEXT)", [])
            .expect("create checkpoint_files");

        let detected = detect_legacy_shared_sqlite_tables(&conn)
            .expect("detect legacy shared sqlite tables");

        assert!(detected.contains(&"sync_state".to_string()));
        assert!(detected.contains(&"checkpoint_files".to_string()));
        assert!(!detected.contains(&"repositories".to_string()));
    }

    #[tokio::test]
    async fn init_sqlite_schema_keeps_historical_tables_when_shared_authority_is_local() {
        let temp = TempDir::new().expect("temp dir");
        let sqlite_path = temp.path().join("devql.sqlite");

        init_sqlite_schema(&sqlite_path)
            .await
            .expect("initialise full local SQLite schema");

        let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite db");
        let table_exists = |name: &str| -> bool {
            conn.query_row(
                "SELECT EXISTS(
                    SELECT 1
                    FROM sqlite_master
                    WHERE type IN ('table', 'view') AND name = ?1
                )",
                [name],
                |row| row.get::<_, i64>(0),
            )
            .expect("query sqlite_master")
                != 0
        };

        assert!(table_exists("repositories"));
        assert!(table_exists("current_file_state"));
        assert!(table_exists("artefacts_current"));
        assert!(table_exists("workspace_revisions"));

        assert!(table_exists("commits"));
        assert!(table_exists("file_state"));
        assert!(table_exists("artefacts"));
        assert!(table_exists("symbol_features"));
        assert!(table_exists("symbol_semantics"));
        assert!(table_exists("symbol_search_documents"));
        assert!(table_exists("symbol_embeddings"));
        assert!(table_exists("semantic_embedding_setups"));
        assert!(table_exists("symbol_clone_edges"));
    }
}
