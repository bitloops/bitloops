use std::path::Path;

use anyhow::Result;

use super::storage::{
    semantic_features_postgres_schema_sql, semantic_features_postgres_upgrade_sql,
    semantic_features_sqlite_schema_sql, upgrade_sqlite_semantic_features_schema,
};
use crate::host::devql::{RelationalStorage, postgres_exec, sqlite_exec_path_allow_create};

pub(crate) async fn init_postgres_semantic_features_schema(
    pg_client: &tokio_postgres::Client,
) -> Result<()> {
    postgres_exec(pg_client, semantic_features_postgres_schema_sql()).await?;
    postgres_exec(pg_client, semantic_features_postgres_upgrade_sql()).await
}

pub(crate) async fn init_sqlite_semantic_features_schema(sqlite_path: &Path) -> Result<()> {
    sqlite_exec_path_allow_create(sqlite_path, semantic_features_sqlite_schema_sql()).await?;
    upgrade_sqlite_semantic_features_schema(sqlite_path).await
}

pub(crate) async fn ensure_semantic_features_schema(relational: &RelationalStorage) -> Result<()> {
    crate::host::devql::ensure_sqlite_schema_once(
        relational.sqlite_path(),
        "semantic_features_sqlite",
        |sqlite_path| async move { init_sqlite_semantic_features_schema(&sqlite_path).await },
    )
    .await?;
    if let Some(remote_client) = relational.remote_client() {
        crate::host::devql::ensure_sqlite_schema_once(
            relational.sqlite_path(),
            "semantic_features_postgres",
            |_| async move { init_postgres_semantic_features_schema(remote_client).await },
        )
        .await?;
    }
    Ok(())
}
