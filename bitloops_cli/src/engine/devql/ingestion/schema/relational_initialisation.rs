async fn init_sqlite_schema(sqlite_path: &Path) -> Result<()> {
    sqlite_exec_path_allow_create(sqlite_path, sqlite_schema_sql())
        .await
        .context("creating SQLite relational DevQL tables")?;
    sqlite_exec_path_allow_create(sqlite_path, edge_model_cleanup_sqlite_sql())
        .await
        .context("normalising SQLite DevQL edge model values")?;
    sqlite_exec_path_allow_create(sqlite_path, checkpoint_schema_sql_sqlite())
        .await
        .context("creating SQLite checkpoint migration tables")?;
    init_sqlite_semantic_features_schema(sqlite_path)
        .await
        .context("creating SQLite semantic feature tables")?;
    init_sqlite_semantic_embeddings_schema(sqlite_path)
        .await
        .context("creating SQLite semantic embedding tables")?;
    init_sqlite_semantic_clones_schema(sqlite_path)
        .await
        .context("creating SQLite semantic clone tables")?;
    Ok(())
}

async fn init_postgres_schema(
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

    let current_state_hardening_sql = current_state_hardening_sql();
    postgres_exec(pg_client, current_state_hardening_sql)
        .await
        .context("updating Postgres current-state DevQL tables")?;

    let edge_model_cleanup_sql = edge_model_cleanup_postgres_sql();
    postgres_exec(pg_client, edge_model_cleanup_sql)
        .await
        .context("normalising Postgres DevQL edge model values")?;

    init_postgres_semantic_features_schema(pg_client)
        .await
        .context("creating Postgres semantic feature tables")?;
    init_postgres_semantic_embeddings_schema(pg_client)
        .await
        .context("creating Postgres semantic embedding tables")?;
    init_postgres_semantic_clones_schema(pg_client)
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
