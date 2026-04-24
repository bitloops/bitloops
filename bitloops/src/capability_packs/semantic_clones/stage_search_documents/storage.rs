use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::devql::{RelationalDialect, esc_pg};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchDocumentRow {
    pub artefact_id: String,
    pub repo_id: String,
    pub blob_sha: String,
    pub path: String,
    pub symbol_id: Option<String>,
    pub signature_text: Option<String>,
    pub summary_text: Option<String>,
    pub body_text: String,
    pub searchable_text: String,
}

pub(crate) fn build_search_document_from_semantic_rows(
    input: &semantic::SemanticFeatureInput,
    rows: &semantic::SemanticFeatureRows,
) -> SearchDocumentRow {
    let signature_text = normalize_search_text(input.signature.as_deref());
    let summary_text = normalize_search_text(Some(rows.semantics.summary.as_str()));
    let body_text = normalize_search_text(Some(input.body.as_str())).unwrap_or_default();
    let searchable_text = [
        signature_text.clone(),
        summary_text.clone(),
        Some(body_text.clone()),
    ]
    .into_iter()
    .flatten()
    .filter(|value| !value.is_empty())
    .collect::<Vec<_>>()
    .join("\n\n");

    SearchDocumentRow {
        artefact_id: input.artefact_id.clone(),
        repo_id: input.repo_id.clone(),
        blob_sha: input.blob_sha.clone(),
        path: input.path.clone(),
        symbol_id: input.symbol_id.clone(),
        signature_text,
        summary_text,
        body_text,
        searchable_text,
    }
}

pub(crate) fn search_documents_postgres_schema_sql() -> &'static str {
    r#"
CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE TABLE IF NOT EXISTS symbol_search_documents (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    symbol_id TEXT,
    signature_text TEXT,
    summary_text TEXT,
    body_text TEXT NOT NULL,
    searchable_text TEXT NOT NULL,
    generated_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS symbol_search_documents_repo_blob_idx
ON symbol_search_documents (repo_id, blob_sha);

CREATE INDEX IF NOT EXISTS symbol_search_documents_repo_path_idx
ON symbol_search_documents (repo_id, path);

CREATE INDEX IF NOT EXISTS symbol_search_documents_tsv_idx
ON symbol_search_documents
USING GIN ((
    setweight(to_tsvector('simple', COALESCE(signature_text, '')), 'A') ||
    setweight(to_tsvector('simple', COALESCE(summary_text, '')), 'B') ||
    setweight(to_tsvector('simple', COALESCE(body_text, '')), 'C')
));

CREATE INDEX IF NOT EXISTS symbol_search_documents_searchable_trgm_idx
ON symbol_search_documents
USING GIN (searchable_text gin_trgm_ops);

CREATE TABLE IF NOT EXISTS symbol_search_documents_current (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    signature_text TEXT,
    summary_text TEXT,
    body_text TEXT NOT NULL,
    searchable_text TEXT NOT NULL,
    generated_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS symbol_search_documents_current_repo_path_idx
ON symbol_search_documents_current (repo_id, path);

CREATE UNIQUE INDEX IF NOT EXISTS symbol_search_documents_current_repo_artefact_idx
ON symbol_search_documents_current (repo_id, artefact_id);

CREATE INDEX IF NOT EXISTS symbol_search_documents_current_tsv_idx
ON symbol_search_documents_current
USING GIN ((
    setweight(to_tsvector('simple', COALESCE(signature_text, '')), 'A') ||
    setweight(to_tsvector('simple', COALESCE(summary_text, '')), 'B') ||
    setweight(to_tsvector('simple', COALESCE(body_text, '')), 'C')
));

CREATE INDEX IF NOT EXISTS symbol_search_documents_current_searchable_trgm_idx
ON symbol_search_documents_current
USING GIN (searchable_text gin_trgm_ops);
"#
}

pub(crate) fn search_documents_sqlite_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS symbol_search_documents (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    symbol_id TEXT,
    signature_text TEXT,
    summary_text TEXT,
    body_text TEXT NOT NULL,
    searchable_text TEXT NOT NULL,
    generated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS symbol_search_documents_repo_blob_idx
ON symbol_search_documents (repo_id, blob_sha);

CREATE INDEX IF NOT EXISTS symbol_search_documents_repo_path_idx
ON symbol_search_documents (repo_id, path);

CREATE VIRTUAL TABLE IF NOT EXISTS symbol_search_documents_fts USING fts5(
    artefact_id UNINDEXED,
    repo_id UNINDEXED,
    blob_sha UNINDEXED,
    path UNINDEXED,
    symbol_id UNINDEXED,
    signature_text,
    summary_text,
    body_text,
    searchable_text,
    tokenize = 'unicode61'
);

CREATE TABLE IF NOT EXISTS symbol_search_documents_current (
    artefact_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT,
    signature_text TEXT,
    summary_text TEXT,
    body_text TEXT NOT NULL,
    searchable_text TEXT NOT NULL,
    generated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS symbol_search_documents_current_repo_path_idx
ON symbol_search_documents_current (repo_id, path);

CREATE UNIQUE INDEX IF NOT EXISTS symbol_search_documents_current_repo_artefact_idx
ON symbol_search_documents_current (repo_id, artefact_id);

CREATE VIRTUAL TABLE IF NOT EXISTS symbol_search_documents_current_fts USING fts5(
    artefact_id UNINDEXED,
    repo_id UNINDEXED,
    content_id UNINDEXED,
    path UNINDEXED,
    symbol_id UNINDEXED,
    signature_text,
    summary_text,
    body_text,
    searchable_text,
    tokenize = 'unicode61'
);
"#
}

pub(crate) fn build_search_document_persist_sql(
    row: &SearchDocumentRow,
    dialect: RelationalDialect,
) -> String {
    let now_sql = current_timestamp_sql(dialect);
    format!(
        "INSERT INTO symbol_search_documents (
            artefact_id, repo_id, blob_sha, path, symbol_id, signature_text, summary_text,
            body_text, searchable_text
         )
         VALUES (
            '{artefact_id}', '{repo_id}', '{blob_sha}', '{path}', {symbol_id},
            {signature_text}, {summary_text}, '{body_text}', '{searchable_text}'
         )
         ON CONFLICT (artefact_id) DO UPDATE SET
            repo_id = EXCLUDED.repo_id,
            blob_sha = EXCLUDED.blob_sha,
            path = EXCLUDED.path,
            symbol_id = EXCLUDED.symbol_id,
            signature_text = EXCLUDED.signature_text,
            summary_text = EXCLUDED.summary_text,
            body_text = EXCLUDED.body_text,
            searchable_text = EXCLUDED.searchable_text,
            generated_at = {now_sql}",
        artefact_id = esc_pg(&row.artefact_id),
        repo_id = esc_pg(&row.repo_id),
        blob_sha = esc_pg(&row.blob_sha),
        path = esc_pg(&row.path),
        symbol_id = sql_optional_string(row.symbol_id.as_deref()),
        signature_text = sql_optional_string(row.signature_text.as_deref()),
        summary_text = sql_optional_string(row.summary_text.as_deref()),
        body_text = esc_pg(&row.body_text),
        searchable_text = esc_pg(&row.searchable_text),
    )
}

pub(crate) fn build_current_search_document_persist_sql(
    row: &SearchDocumentRow,
    dialect: RelationalDialect,
) -> String {
    let now_sql = current_timestamp_sql(dialect);
    format!(
        "INSERT INTO symbol_search_documents_current (
            artefact_id, repo_id, path, content_id, symbol_id, signature_text, summary_text,
            body_text, searchable_text
         )
         VALUES (
            '{artefact_id}', '{repo_id}', '{path}', '{content_id}', {symbol_id},
            {signature_text}, {summary_text}, '{body_text}', '{searchable_text}'
         )
         ON CONFLICT (artefact_id) DO UPDATE SET
            repo_id = EXCLUDED.repo_id,
            path = EXCLUDED.path,
            content_id = EXCLUDED.content_id,
            symbol_id = EXCLUDED.symbol_id,
            signature_text = EXCLUDED.signature_text,
            summary_text = EXCLUDED.summary_text,
            body_text = EXCLUDED.body_text,
            searchable_text = EXCLUDED.searchable_text,
            generated_at = {now_sql}",
        artefact_id = esc_pg(&row.artefact_id),
        repo_id = esc_pg(&row.repo_id),
        path = esc_pg(&row.path),
        content_id = esc_pg(&row.blob_sha),
        symbol_id = sql_optional_string(row.symbol_id.as_deref()),
        signature_text = sql_optional_string(row.signature_text.as_deref()),
        summary_text = sql_optional_string(row.summary_text.as_deref()),
        body_text = esc_pg(&row.body_text),
        searchable_text = esc_pg(&row.searchable_text),
    )
}

pub(crate) fn build_sqlite_search_document_fts_refresh_sql(row: &SearchDocumentRow) -> String {
    format!(
        "DELETE FROM symbol_search_documents_fts WHERE artefact_id = '{artefact_id}'; \
         INSERT INTO symbol_search_documents_fts (
            artefact_id, repo_id, blob_sha, path, symbol_id, signature_text, summary_text,
            body_text, searchable_text
         )
         VALUES (
            '{artefact_id}', '{repo_id}', '{blob_sha}', '{path}', {symbol_id},
            {signature_text}, {summary_text}, '{body_text}', '{searchable_text}'
         )",
        artefact_id = esc_pg(&row.artefact_id),
        repo_id = esc_pg(&row.repo_id),
        blob_sha = esc_pg(&row.blob_sha),
        path = esc_pg(&row.path),
        symbol_id = sql_optional_string(row.symbol_id.as_deref()),
        signature_text = sql_optional_string(row.signature_text.as_deref()),
        summary_text = sql_optional_string(row.summary_text.as_deref()),
        body_text = esc_pg(&row.body_text),
        searchable_text = esc_pg(&row.searchable_text),
    )
}

pub(crate) fn build_sqlite_current_search_document_fts_refresh_sql(
    row: &SearchDocumentRow,
) -> String {
    format!(
        "DELETE FROM symbol_search_documents_current_fts WHERE artefact_id = '{artefact_id}'; \
         INSERT INTO symbol_search_documents_current_fts (
            artefact_id, repo_id, content_id, path, symbol_id, signature_text, summary_text,
            body_text, searchable_text
         )
         VALUES (
            '{artefact_id}', '{repo_id}', '{content_id}', '{path}', {symbol_id},
            {signature_text}, {summary_text}, '{body_text}', '{searchable_text}'
         )",
        artefact_id = esc_pg(&row.artefact_id),
        repo_id = esc_pg(&row.repo_id),
        content_id = esc_pg(&row.blob_sha),
        path = esc_pg(&row.path),
        symbol_id = sql_optional_string(row.symbol_id.as_deref()),
        signature_text = sql_optional_string(row.signature_text.as_deref()),
        summary_text = sql_optional_string(row.summary_text.as_deref()),
        body_text = esc_pg(&row.body_text),
        searchable_text = esc_pg(&row.searchable_text),
    )
}

pub(crate) fn build_delete_current_search_documents_sql(repo_id: &str, path: &str) -> String {
    format!(
        "DELETE FROM symbol_search_documents_current WHERE repo_id = '{}' AND path = '{}'",
        esc_pg(repo_id),
        esc_pg(path),
    )
}

pub(crate) fn build_delete_current_search_documents_fts_sql(repo_id: &str, path: &str) -> String {
    format!(
        "DELETE FROM symbol_search_documents_current_fts WHERE repo_id = '{}' AND path = '{}'",
        esc_pg(repo_id),
        esc_pg(path),
    )
}

fn normalize_search_text(value: Option<&str>) -> Option<String> {
    value
        .map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn sql_optional_string(value: Option<&str>) -> String {
    value
        .map(|value| format!("'{}'", esc_pg(value)))
        .unwrap_or_else(|| "NULL".to_string())
}

fn current_timestamp_sql(dialect: RelationalDialect) -> &'static str {
    match dialect {
        RelationalDialect::Postgres => "now()",
        RelationalDialect::Sqlite => "CURRENT_TIMESTAMP",
    }
}
