//! Stage 1c: lexical/full-text search documents for hybrid artefact search.

mod persistence;
mod schema;
mod storage;

pub(crate) use self::persistence::{
    clear_current_search_document_rows_for_artefact, clear_current_search_document_rows_for_path,
    persist_current_search_document_row, persist_search_document_row,
};
pub(crate) use self::schema::{
    ensure_search_documents_schema, init_postgres_search_documents_schema,
    init_sqlite_search_documents_schema,
};
pub(crate) use self::storage::{
    SearchDocumentRow, build_current_search_document_persist_sql,
    build_delete_current_search_documents_for_artefact_sql,
    build_delete_current_search_documents_fts_sql, build_delete_current_search_documents_sql,
    build_delete_current_search_documents_fts_for_artefact_sql,
    build_search_document_from_semantic_rows, build_search_document_persist_sql,
    search_documents_postgres_schema_sql, search_documents_sqlite_schema_sql,
};
