use anyhow::Result;

use super::schema::ensure_search_documents_schema;
use super::storage::{
    build_current_search_document_persist_sql,
    build_delete_current_search_documents_for_artefact_sql,
    build_delete_current_search_documents_fts_for_artefact_sql,
    build_delete_current_search_documents_fts_sql, build_delete_current_search_documents_sql,
    build_search_document_from_semantic_rows, build_search_document_persist_sql,
    build_sqlite_current_search_document_fts_refresh_sql,
    build_sqlite_search_document_fts_refresh_sql,
};
use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::devql::{RelationalDialect, RelationalStorage, RelationalStorageRole};

fn current_relational_dialect() -> RelationalDialect {
    RelationalDialect::Sqlite
}

fn shared_relational_dialect(relational: &RelationalStorage) -> RelationalDialect {
    relational.dialect_for_role(RelationalStorageRole::SharedRelational)
}

async fn exec_shared_batch_transactional(
    relational: &RelationalStorage,
    statements: &[String],
) -> Result<()> {
    if statements.is_empty() {
        return Ok(());
    }
    relational
        .exec_batch_transactional_for_role(RelationalStorageRole::SharedRelational, statements)
        .await
}

async fn exec_current_batch_transactional(
    relational: &RelationalStorage,
    statements: &[String],
) -> Result<()> {
    if statements.is_empty() {
        return Ok(());
    }
    relational
        .exec_serialized_batch_transactional(statements)
        .await
}

pub(crate) async fn persist_search_document_row(
    relational: &RelationalStorage,
    input: &semantic::SemanticFeatureInput,
    rows: &semantic::SemanticFeatureRows,
) -> Result<()> {
    ensure_search_documents_schema(relational).await?;
    let row = build_search_document_from_semantic_rows(input, rows);
    let mut statements = vec![build_search_document_persist_sql(
        &row,
        shared_relational_dialect(relational),
    )];
    if relational.dialect_for_role(RelationalStorageRole::SharedRelational)
        == RelationalDialect::Sqlite
    {
        statements.push(build_sqlite_search_document_fts_refresh_sql(&row));
    }
    exec_shared_batch_transactional(relational, &statements).await
}

pub(crate) async fn persist_current_search_document_row(
    relational: &RelationalStorage,
    input: &semantic::SemanticFeatureInput,
    rows: &semantic::SemanticFeatureRows,
) -> Result<()> {
    ensure_search_documents_schema(relational).await?;
    let row = build_search_document_from_semantic_rows(input, rows);
    let mut statements = vec![build_current_search_document_persist_sql(
        &row,
        current_relational_dialect(),
    )];
    if current_relational_dialect() == RelationalDialect::Sqlite {
        statements.push(build_sqlite_current_search_document_fts_refresh_sql(&row));
    }
    exec_current_batch_transactional(relational, &statements).await
}

pub(crate) async fn clear_current_search_document_rows_for_path(
    relational: &RelationalStorage,
    repo_id: &str,
    path: &str,
) -> Result<()> {
    ensure_search_documents_schema(relational).await?;
    let mut statements = vec![build_delete_current_search_documents_sql(repo_id, path)];
    if current_relational_dialect() == RelationalDialect::Sqlite {
        statements.push(build_delete_current_search_documents_fts_sql(repo_id, path));
    }
    exec_current_batch_transactional(relational, &statements).await
}

pub(crate) async fn clear_current_search_document_rows_for_artefact(
    relational: &RelationalStorage,
    repo_id: &str,
    artefact_id: &str,
) -> Result<()> {
    ensure_search_documents_schema(relational).await?;
    let mut statements = vec![build_delete_current_search_documents_for_artefact_sql(
        repo_id,
        artefact_id,
    )];
    if current_relational_dialect() == RelationalDialect::Sqlite {
        statements.push(build_delete_current_search_documents_fts_for_artefact_sql(
            artefact_id,
        ));
    }
    exec_current_batch_transactional(relational, &statements).await
}
