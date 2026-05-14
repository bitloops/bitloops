use std::path::Path;

use anyhow::Result;

use super::storage::{
    search_documents_postgres_shared_schema_sql,
    search_documents_sqlite_current_projection_schema_sql, search_documents_sqlite_schema_sql,
};
use crate::host::devql::{RelationalStorage, postgres_exec, sqlite_exec_path_allow_create};

pub(crate) async fn init_postgres_search_documents_schema(
    pg_client: &tokio_postgres::Client,
) -> Result<()> {
    postgres_exec(pg_client, search_documents_postgres_shared_schema_sql()).await
}

pub(crate) async fn init_sqlite_search_documents_schema(sqlite_path: &Path) -> Result<()> {
    if crate::host::devql::types::sqlite_path_uses_remote_shared_relational_authority(sqlite_path) {
        return init_sqlite_current_projection_search_documents_schema(sqlite_path).await;
    }
    sqlite_exec_path_allow_create(sqlite_path, search_documents_sqlite_schema_sql()).await
}

pub(crate) async fn init_sqlite_current_projection_search_documents_schema(
    sqlite_path: &Path,
) -> Result<()> {
    sqlite_exec_path_allow_create(
        sqlite_path,
        search_documents_sqlite_current_projection_schema_sql(),
    )
    .await
}

pub(crate) async fn ensure_search_documents_schema(relational: &RelationalStorage) -> Result<()> {
    let remote_shared = relational.has_remote_shared_relational_authority();
    crate::host::devql::ensure_sqlite_schema_once(
        relational.sqlite_path(),
        "search_documents_sqlite",
        |sqlite_path| async move {
            if remote_shared {
                init_sqlite_current_projection_search_documents_schema(&sqlite_path).await
            } else {
                init_sqlite_search_documents_schema(&sqlite_path).await
            }
        },
    )
    .await?;
    if let Some(remote_client) = relational.remote_client() {
        crate::host::devql::ensure_sqlite_schema_once(
            relational.sqlite_path(),
            "search_documents_postgres",
            |_| async move { init_postgres_search_documents_schema(remote_client).await },
        )
        .await?;
    }
    Ok(())
}
