use std::path::Path;

use anyhow::Result;

use super::storage::{
    semantic_features_postgres_upgrade_sql, semantic_features_sqlite_schema_sql,
    upgrade_sqlite_semantic_features_schema,
};
use crate::host::devql::{RelationalStorage, postgres_exec, sqlite_exec_path_allow_create};

fn semantic_features_postgres_shared_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS symbol_semantics (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    semantic_features_input_hash TEXT NOT NULL,
    docstring_summary TEXT,
    llm_summary TEXT,
    template_summary TEXT NOT NULL,
    summary TEXT NOT NULL,
    confidence REAL,
    source_model TEXT,
    generated_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS symbol_semantics_repo_blob_idx
ON symbol_semantics (repo_id, blob_sha);

CREATE TABLE IF NOT EXISTS symbol_features (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    semantic_features_input_hash TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    normalized_signature TEXT,
    modifiers JSONB NOT NULL DEFAULT '[]'::jsonb,
    identifier_tokens JSONB NOT NULL DEFAULT '[]'::jsonb,
    normalized_body_tokens JSONB NOT NULL DEFAULT '[]'::jsonb,
    parent_kind TEXT,
    context_tokens JSONB NOT NULL DEFAULT '[]'::jsonb,
    generated_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS symbol_features_repo_blob_idx
ON symbol_features (repo_id, blob_sha);
"#
}

fn semantic_features_sqlite_current_projection_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS symbol_semantics_current (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    semantic_features_input_hash TEXT NOT NULL,
    docstring_summary TEXT,
    llm_summary TEXT,
    template_summary TEXT NOT NULL,
    summary TEXT NOT NULL,
    confidence REAL,
    source_model TEXT,
    generated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS symbol_semantics_current_repo_path_idx
ON symbol_semantics_current (repo_id, path);

CREATE UNIQUE INDEX IF NOT EXISTS symbol_semantics_current_repo_artefact_idx
ON symbol_semantics_current (repo_id, artefact_id);

CREATE TABLE IF NOT EXISTS symbol_features_current (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    semantic_features_input_hash TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    normalized_signature TEXT,
    modifiers TEXT NOT NULL DEFAULT '[]',
    identifier_tokens TEXT NOT NULL DEFAULT '[]',
    normalized_body_tokens TEXT NOT NULL DEFAULT '[]',
    parent_kind TEXT,
    context_tokens TEXT NOT NULL DEFAULT '[]',
    generated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS symbol_features_current_repo_path_idx
ON symbol_features_current (repo_id, path);

CREATE UNIQUE INDEX IF NOT EXISTS symbol_features_current_repo_artefact_idx
ON symbol_features_current (repo_id, artefact_id);
"#
}

pub(crate) async fn init_postgres_semantic_features_schema(
    pg_client: &tokio_postgres::Client,
) -> Result<()> {
    postgres_exec(pg_client, semantic_features_postgres_shared_schema_sql()).await?;
    postgres_exec(pg_client, semantic_features_postgres_upgrade_sql()).await
}

pub(crate) async fn init_sqlite_semantic_features_schema(sqlite_path: &Path) -> Result<()> {
    if crate::host::devql::types::sqlite_path_uses_remote_shared_relational_authority(sqlite_path) {
        return init_sqlite_current_projection_semantic_features_schema(sqlite_path).await;
    }
    sqlite_exec_path_allow_create(sqlite_path, semantic_features_sqlite_schema_sql()).await?;
    upgrade_sqlite_semantic_features_schema(sqlite_path).await
}

pub(crate) async fn init_sqlite_current_projection_semantic_features_schema(
    sqlite_path: &Path,
) -> Result<()> {
    sqlite_exec_path_allow_create(
        sqlite_path,
        semantic_features_sqlite_current_projection_schema_sql(),
    )
    .await
}

pub(crate) async fn ensure_semantic_features_schema(relational: &RelationalStorage) -> Result<()> {
    let remote_shared = relational.has_remote_shared_relational_authority();
    crate::host::devql::ensure_sqlite_schema_once(
        relational.sqlite_path(),
        "semantic_features_sqlite",
        |sqlite_path| async move {
            if remote_shared {
                init_sqlite_current_projection_semantic_features_schema(&sqlite_path).await
            } else {
                init_sqlite_semantic_features_schema(&sqlite_path).await
            }
        },
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
