pub(crate) fn sync_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS repo_sync_state (
    repo_id TEXT PRIMARY KEY,
    repo_root TEXT NOT NULL,
    active_branch TEXT,
    head_commit_sha TEXT,
    head_tree_sha TEXT,
    parser_version TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    last_sync_started_at TEXT,
    last_sync_completed_at TEXT,
    last_sync_status TEXT,
    last_sync_reason TEXT,
    FOREIGN KEY (repo_id) REFERENCES repositories(repo_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS project_contexts_current (
    repo_id TEXT NOT NULL,
    context_id TEXT NOT NULL,
    root TEXT NOT NULL,
    kind TEXT NOT NULL,
    detection_source TEXT NOT NULL,
    frameworks_json TEXT NOT NULL DEFAULT '[]',
    runtime_profile TEXT,
    config_files_json TEXT NOT NULL DEFAULT '[]',
    config_fingerprint TEXT NOT NULL,
    source_versions_json TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (repo_id, context_id),
    FOREIGN KEY (repo_id) REFERENCES repositories(repo_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS current_file_state (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    analysis_mode TEXT NOT NULL DEFAULT 'code',
    file_role TEXT NOT NULL DEFAULT 'source_code',
    text_index_mode TEXT NOT NULL DEFAULT 'none',
    language TEXT NOT NULL,
    resolved_language TEXT NOT NULL DEFAULT '',
    dialect TEXT,
    primary_context_id TEXT,
    secondary_context_ids_json TEXT NOT NULL DEFAULT '[]',
    frameworks_json TEXT NOT NULL DEFAULT '[]',
    runtime_profile TEXT,
    classification_reason TEXT NOT NULL DEFAULT '',
    context_fingerprint TEXT,
    extraction_fingerprint TEXT NOT NULL DEFAULT '',
    head_content_id TEXT,
    index_content_id TEXT,
    worktree_content_id TEXT,
    effective_content_id TEXT NOT NULL,
    effective_source TEXT NOT NULL,
    parser_version TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    exists_in_head INTEGER NOT NULL,
    exists_in_index INTEGER NOT NULL,
    exists_in_worktree INTEGER NOT NULL,
    last_synced_at TEXT NOT NULL,
    PRIMARY KEY (repo_id, path),
    FOREIGN KEY (repo_id) REFERENCES repositories(repo_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS content_cache (
    content_id TEXT NOT NULL,
    language TEXT NOT NULL,
    extraction_fingerprint TEXT NOT NULL,
    parser_version TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    retention_class TEXT NOT NULL,
    parse_status TEXT NOT NULL,
    parsed_at TEXT NOT NULL,
    last_accessed_at TEXT NOT NULL,
    PRIMARY KEY (content_id, language, extraction_fingerprint, parser_version, extractor_version)
);

CREATE TABLE IF NOT EXISTS content_cache_artefacts (
    content_id TEXT NOT NULL,
    language TEXT NOT NULL,
    extraction_fingerprint TEXT NOT NULL,
    parser_version TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    artifact_key TEXT NOT NULL,
    canonical_kind TEXT,
    language_kind TEXT NOT NULL,
    name TEXT NOT NULL,
    parent_artifact_key TEXT,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    signature TEXT NOT NULL,
    modifiers TEXT NOT NULL,
    docstring TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (content_id, language, extraction_fingerprint, parser_version, extractor_version, artifact_key)
);

CREATE TABLE IF NOT EXISTS content_cache_edges (
    content_id TEXT NOT NULL,
    language TEXT NOT NULL,
    extraction_fingerprint TEXT NOT NULL,
    parser_version TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    edge_key TEXT NOT NULL,
    from_artifact_key TEXT NOT NULL,
    to_artifact_key TEXT,
    to_symbol_ref TEXT,
    edge_kind TEXT NOT NULL,
    start_line INTEGER,
    end_line INTEGER,
    metadata TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (content_id, language, extraction_fingerprint, parser_version, extractor_version, edge_key)
);
"#
}

pub(crate) fn sync_repo_sync_state_migration_sql() -> &'static str {
    r#"
DROP TABLE IF EXISTS repo_sync_state;

CREATE TABLE IF NOT EXISTS repo_sync_state (
    repo_id TEXT PRIMARY KEY,
    repo_root TEXT NOT NULL,
    active_branch TEXT,
    head_commit_sha TEXT,
    head_tree_sha TEXT,
    parser_version TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    last_sync_started_at TEXT,
    last_sync_completed_at TEXT,
    last_sync_status TEXT,
    last_sync_reason TEXT,
    FOREIGN KEY (repo_id) REFERENCES repositories(repo_id) ON DELETE CASCADE
);
"#
}

pub(crate) fn sync_project_contexts_current_migration_sql() -> &'static str {
    r#"
DROP TABLE IF EXISTS project_contexts_current;

CREATE TABLE IF NOT EXISTS project_contexts_current (
    repo_id TEXT NOT NULL,
    context_id TEXT NOT NULL,
    root TEXT NOT NULL,
    kind TEXT NOT NULL,
    detection_source TEXT NOT NULL,
    frameworks_json TEXT NOT NULL DEFAULT '[]',
    runtime_profile TEXT,
    config_files_json TEXT NOT NULL DEFAULT '[]',
    config_fingerprint TEXT NOT NULL,
    source_versions_json TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (repo_id, context_id),
    FOREIGN KEY (repo_id) REFERENCES repositories(repo_id) ON DELETE CASCADE
);
"#
}

pub(crate) fn sync_current_file_state_migration_sql() -> &'static str {
    r#"
DROP TABLE IF EXISTS current_file_state;

CREATE TABLE IF NOT EXISTS current_file_state (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    analysis_mode TEXT NOT NULL DEFAULT 'code',
    file_role TEXT NOT NULL DEFAULT 'source_code',
    text_index_mode TEXT NOT NULL DEFAULT 'none',
    language TEXT NOT NULL,
    resolved_language TEXT NOT NULL DEFAULT '',
    dialect TEXT,
    primary_context_id TEXT,
    secondary_context_ids_json TEXT NOT NULL DEFAULT '[]',
    frameworks_json TEXT NOT NULL DEFAULT '[]',
    runtime_profile TEXT,
    classification_reason TEXT NOT NULL DEFAULT '',
    context_fingerprint TEXT,
    extraction_fingerprint TEXT NOT NULL DEFAULT '',
    head_content_id TEXT,
    index_content_id TEXT,
    worktree_content_id TEXT,
    effective_content_id TEXT NOT NULL,
    effective_source TEXT NOT NULL,
    parser_version TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    exists_in_head INTEGER NOT NULL,
    exists_in_index INTEGER NOT NULL,
    exists_in_worktree INTEGER NOT NULL,
    last_synced_at TEXT NOT NULL,
    PRIMARY KEY (repo_id, path),
    FOREIGN KEY (repo_id) REFERENCES repositories(repo_id) ON DELETE CASCADE
);
"#
}

pub(crate) fn sync_content_cache_migration_sql() -> &'static str {
    r#"
DROP TABLE IF EXISTS content_cache_edges;
DROP TABLE IF EXISTS content_cache_artefacts;
DROP TABLE IF EXISTS content_cache;

CREATE TABLE IF NOT EXISTS content_cache (
    content_id TEXT NOT NULL,
    language TEXT NOT NULL,
    extraction_fingerprint TEXT NOT NULL,
    parser_version TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    retention_class TEXT NOT NULL,
    parse_status TEXT NOT NULL,
    parsed_at TEXT NOT NULL,
    last_accessed_at TEXT NOT NULL,
    PRIMARY KEY (content_id, language, extraction_fingerprint, parser_version, extractor_version)
);

CREATE TABLE IF NOT EXISTS content_cache_artefacts (
    content_id TEXT NOT NULL,
    language TEXT NOT NULL,
    extraction_fingerprint TEXT NOT NULL,
    parser_version TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    artifact_key TEXT NOT NULL,
    canonical_kind TEXT,
    language_kind TEXT NOT NULL,
    name TEXT NOT NULL,
    parent_artifact_key TEXT,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    signature TEXT NOT NULL,
    modifiers TEXT NOT NULL,
    docstring TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (content_id, language, extraction_fingerprint, parser_version, extractor_version, artifact_key)
);

CREATE TABLE IF NOT EXISTS content_cache_edges (
    content_id TEXT NOT NULL,
    language TEXT NOT NULL,
    extraction_fingerprint TEXT NOT NULL,
    parser_version TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    edge_key TEXT NOT NULL,
    from_artifact_key TEXT NOT NULL,
    to_artifact_key TEXT,
    to_symbol_ref TEXT,
    edge_kind TEXT NOT NULL,
    start_line INTEGER,
    end_line INTEGER,
    metadata TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (content_id, language, extraction_fingerprint, parser_version, extractor_version, edge_key)
);
"#
}

pub(crate) fn sync_artefacts_current_migration_sql() -> &'static str {
    r#"
DROP TABLE IF EXISTS artefacts_current;

CREATE TABLE IF NOT EXISTS artefacts_current (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
    language TEXT NOT NULL,
    extraction_fingerprint TEXT NOT NULL DEFAULT '',
    canonical_kind TEXT,
    language_kind TEXT,
    symbol_fqn TEXT,
    parent_symbol_id TEXT,
    parent_artefact_id TEXT,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    signature TEXT,
    modifiers TEXT NOT NULL DEFAULT '[]',
    docstring TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (repo_id, path, symbol_id),
    UNIQUE (repo_id, artefact_id),
    FOREIGN KEY (repo_id) REFERENCES repositories(repo_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS artefacts_current_path_idx
ON artefacts_current (repo_id, path);

CREATE INDEX IF NOT EXISTS artefacts_current_kind_idx
ON artefacts_current (repo_id, canonical_kind);

CREATE INDEX IF NOT EXISTS artefacts_current_fqn_idx
ON artefacts_current (repo_id, symbol_fqn);

DROP TABLE IF EXISTS artefact_edges_current;

CREATE TABLE IF NOT EXISTS artefact_edges_current (
    repo_id TEXT NOT NULL,
    edge_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    from_symbol_id TEXT NOT NULL,
    from_artefact_id TEXT NOT NULL,
    to_symbol_id TEXT,
    to_artefact_id TEXT,
    to_symbol_ref TEXT,
    edge_kind TEXT NOT NULL,
    language TEXT NOT NULL,
    start_line INTEGER,
    end_line INTEGER,
    metadata TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL,
    CHECK (to_symbol_id IS NOT NULL OR to_symbol_ref IS NOT NULL),
    CHECK (
        (start_line IS NULL AND end_line IS NULL)
        OR (start_line IS NOT NULL AND end_line IS NOT NULL AND start_line > 0 AND end_line >= start_line)
    ),
    PRIMARY KEY (repo_id, edge_id),
    FOREIGN KEY (repo_id) REFERENCES repositories(repo_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS artefact_edges_current_path_idx
ON artefact_edges_current (repo_id, path);

CREATE INDEX IF NOT EXISTS artefact_edges_current_from_idx
ON artefact_edges_current (repo_id, from_symbol_id, edge_kind);
"#
}
