pub(crate) fn knowledge_schema_sql_sqlite() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS knowledge_sources (
    knowledge_source_id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    source_kind TEXT NOT NULL,
    canonical_external_id TEXT NOT NULL,
    canonical_url TEXT NOT NULL,
    provenance_json TEXT NOT NULL,
    created_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS knowledge_sources_external_uq
ON knowledge_sources (provider, source_kind, canonical_external_id);

CREATE TABLE IF NOT EXISTS knowledge_items (
    knowledge_item_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    knowledge_source_id TEXT NOT NULL,
    item_kind TEXT NOT NULL,
    latest_knowledge_item_version_id TEXT,
    provenance_json TEXT NOT NULL,
    created_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS knowledge_items_repo_source_uq
ON knowledge_items (repo_id, knowledge_source_id);

CREATE TABLE IF NOT EXISTS knowledge_relation_assertions (
    relation_assertion_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    knowledge_item_id TEXT NOT NULL,
    source_knowledge_item_version_id TEXT,
    target_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    relation_type TEXT NOT NULL,
    association_method TEXT NOT NULL,
    confidence REAL,
    provenance_json TEXT NOT NULL,
    created_at TEXT DEFAULT (datetime('now'))
);
"#
}

pub(crate) fn knowledge_schema_sql_duckdb() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS knowledge_document_versions (
    knowledge_item_version_id VARCHAR PRIMARY KEY,
    knowledge_item_id VARCHAR NOT NULL,
    provider VARCHAR NOT NULL,
    source_kind VARCHAR NOT NULL,
    content_hash VARCHAR NOT NULL,
    title VARCHAR,
    state VARCHAR,
    author VARCHAR,
    updated_at VARCHAR,
    body_preview VARCHAR,
    normalized_fields_json VARCHAR NOT NULL,
    storage_backend VARCHAR NOT NULL,
    storage_path VARCHAR NOT NULL,
    payload_mime_type VARCHAR,
    payload_size_bytes BIGINT,
    provenance_json VARCHAR NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
"#
}
