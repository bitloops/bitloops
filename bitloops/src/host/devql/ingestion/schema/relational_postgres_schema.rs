pub(crate) fn postgres_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS repositories (
    repo_id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    organization TEXT NOT NULL,
    name TEXT NOT NULL,
    default_branch TEXT,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE TABLE IF NOT EXISTS commits (
    commit_sha TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    author_name TEXT,
    author_email TEXT,
    commit_message TEXT,
    committed_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS file_state (
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    PRIMARY KEY (repo_id, commit_sha, path)
);

CREATE INDEX IF NOT EXISTS file_state_blob_idx
ON file_state (repo_id, blob_sha);

CREATE INDEX IF NOT EXISTS file_state_commit_idx
ON file_state (repo_id, commit_sha);

CREATE TABLE IF NOT EXISTS current_file_state (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    language TEXT NOT NULL,
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
    PRIMARY KEY (repo_id, path)
);

-- Exact checkpoint-to-file snapshot projection for event-backed artefact filters.
CREATE TABLE IF NOT EXISTS checkpoint_file_snapshots (
    repo_id TEXT NOT NULL,
    checkpoint_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    event_time TIMESTAMPTZ NOT NULL,
    agent TEXT NOT NULL DEFAULT '',
    branch TEXT NOT NULL DEFAULT '',
    strategy TEXT NOT NULL DEFAULT '',
    commit_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    PRIMARY KEY (repo_id, checkpoint_id, path, blob_sha)
);

CREATE INDEX IF NOT EXISTS checkpoint_file_snapshots_lookup_idx
ON checkpoint_file_snapshots (repo_id, path, blob_sha);

CREATE INDEX IF NOT EXISTS checkpoint_file_snapshots_agent_time_idx
ON checkpoint_file_snapshots (repo_id, agent, event_time DESC);

CREATE INDEX IF NOT EXISTS checkpoint_file_snapshots_event_time_idx
ON checkpoint_file_snapshots (repo_id, event_time DESC);

CREATE INDEX IF NOT EXISTS checkpoint_file_snapshots_checkpoint_idx
ON checkpoint_file_snapshots (repo_id, checkpoint_id);

CREATE INDEX IF NOT EXISTS checkpoint_file_snapshots_commit_idx
ON checkpoint_file_snapshots (repo_id, commit_sha);

CREATE TABLE IF NOT EXISTS artefacts (
    artefact_id TEXT PRIMARY KEY,
    symbol_id TEXT,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    language TEXT NOT NULL,
    canonical_kind TEXT,
    language_kind TEXT,
    symbol_fqn TEXT,
    parent_artefact_id TEXT,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    signature TEXT,
    modifiers JSONB NOT NULL DEFAULT '[]'::jsonb,
    docstring TEXT,
    content_hash TEXT,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS artefacts_blob_idx
ON artefacts (repo_id, blob_sha);

CREATE INDEX IF NOT EXISTS artefacts_path_idx
ON artefacts (repo_id, path);

CREATE INDEX IF NOT EXISTS artefacts_kind_idx
ON artefacts (repo_id, canonical_kind);

CREATE INDEX IF NOT EXISTS artefacts_symbol_idx
ON artefacts (repo_id, symbol_id)
WHERE symbol_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS artefacts_current (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
    language TEXT NOT NULL,
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
    modifiers JSONB NOT NULL DEFAULT '[]',
    docstring TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (repo_id, path, symbol_id),
    UNIQUE (repo_id, artefact_id)
);

CREATE INDEX IF NOT EXISTS artefacts_current_path_idx
ON artefacts_current (repo_id, path);

CREATE INDEX IF NOT EXISTS artefacts_current_kind_idx
ON artefacts_current (repo_id, canonical_kind);

CREATE INDEX IF NOT EXISTS artefacts_current_fqn_idx
ON artefacts_current (repo_id, symbol_fqn);

CREATE TABLE IF NOT EXISTS artefact_edges (
    edge_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    from_artefact_id TEXT NOT NULL,
    to_artefact_id TEXT,
    to_symbol_ref TEXT,
    edge_kind TEXT NOT NULL,
    language TEXT NOT NULL,
    start_line INTEGER,
    end_line INTEGER,
    metadata JSONB DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ DEFAULT now(),
    CONSTRAINT artefact_edges_target_chk
        CHECK (to_artefact_id IS NOT NULL OR to_symbol_ref IS NOT NULL),
    CONSTRAINT artefact_edges_line_range_chk
        CHECK (
            (start_line IS NULL AND end_line IS NULL)
            OR (start_line IS NOT NULL AND end_line IS NOT NULL AND start_line > 0 AND end_line >= start_line)
        )
);

CREATE INDEX IF NOT EXISTS artefact_edges_blob_idx
ON artefact_edges (repo_id, blob_sha);

CREATE INDEX IF NOT EXISTS artefact_edges_from_idx
ON artefact_edges (repo_id, from_artefact_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_to_idx
ON artefact_edges (repo_id, to_artefact_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_kind_idx
ON artefact_edges (repo_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_symbol_ref_idx
ON artefact_edges (repo_id, edge_kind, to_symbol_ref)
WHERE to_symbol_ref IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_natural_uq
ON artefact_edges (
    repo_id,
    blob_sha,
    from_artefact_id,
    edge_kind,
    COALESCE(to_artefact_id, ''),
    COALESCE(to_symbol_ref, ''),
    COALESCE(start_line, -1),
    COALESCE(end_line, -1)
);

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
    metadata JSONB DEFAULT '{}',
    updated_at TEXT NOT NULL,
    CONSTRAINT artefact_edges_current_target_chk
        CHECK (to_symbol_id IS NOT NULL OR to_symbol_ref IS NOT NULL),
    CONSTRAINT artefact_edges_current_line_range_chk
        CHECK (
            (start_line IS NULL AND end_line IS NULL)
            OR (start_line IS NOT NULL AND end_line IS NOT NULL AND start_line > 0 AND end_line >= start_line)
        ),
    PRIMARY KEY (repo_id, edge_id)
);

CREATE INDEX IF NOT EXISTS artefact_edges_current_path_idx
ON artefact_edges_current (repo_id, path);

CREATE INDEX IF NOT EXISTS artefact_edges_current_from_idx
ON artefact_edges_current (repo_id, from_symbol_id, edge_kind);

CREATE TABLE IF NOT EXISTS workspace_revisions (
    id         BIGSERIAL PRIMARY KEY,
    repo_id    TEXT      NOT NULL,
    tree_hash  TEXT      NOT NULL,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS workspace_revisions_repo_idx
ON workspace_revisions (repo_id);

CREATE UNIQUE INDEX IF NOT EXISTS workspace_revisions_repo_tree_unique_idx
ON workspace_revisions (repo_id, tree_hash);

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
    last_sync_reason TEXT
);

CREATE TABLE IF NOT EXISTS commit_ingest_ledger (
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    history_status TEXT NOT NULL,
    checkpoint_status TEXT NOT NULL,
    checkpoint_id TEXT,
    last_error TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (repo_id, commit_sha)
);

CREATE INDEX IF NOT EXISTS commit_ingest_ledger_repo_idx
ON commit_ingest_ledger (repo_id);

CREATE TABLE IF NOT EXISTS content_cache (
    content_id TEXT NOT NULL,
    language TEXT NOT NULL,
    parser_version TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    retention_class TEXT NOT NULL,
    parse_status TEXT NOT NULL,
    parsed_at TEXT NOT NULL,
    last_accessed_at TEXT NOT NULL,
    PRIMARY KEY (content_id, language, parser_version, extractor_version)
);

CREATE TABLE IF NOT EXISTS content_cache_artefacts (
    content_id TEXT NOT NULL,
    language TEXT NOT NULL,
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
    modifiers JSONB NOT NULL DEFAULT '[]',
    docstring TEXT,
    metadata JSONB NOT NULL DEFAULT '{}',
    PRIMARY KEY (content_id, language, parser_version, extractor_version, artifact_key)
);

CREATE TABLE IF NOT EXISTS content_cache_edges (
    content_id TEXT NOT NULL,
    language TEXT NOT NULL,
    parser_version TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    edge_key TEXT NOT NULL,
    from_artifact_key TEXT NOT NULL,
    to_artifact_key TEXT,
    to_symbol_ref TEXT,
    edge_kind TEXT NOT NULL,
    start_line INTEGER,
    end_line INTEGER,
    metadata JSONB NOT NULL DEFAULT '{}',
    PRIMARY KEY (content_id, language, parser_version, extractor_version, edge_key)
);
"#
}

#[cfg(test)]
mod tests {
    use super::postgres_schema_sql;

    #[test]
    fn postgres_schema_sql_uses_sync_current_state_indexes() {
        let sql = postgres_schema_sql();
        assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefacts_current_path_idx"));
        assert!(sql.contains("ON artefacts_current (repo_id, path);"));
        assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefacts_current_kind_idx"));
        assert!(sql.contains("ON artefacts_current (repo_id, canonical_kind);"));
        assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefacts_current_fqn_idx"));
        assert!(sql.contains("ON artefacts_current (repo_id, symbol_fqn);"));
        assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefact_edges_current_path_idx"));
        assert!(sql.contains("ON artefact_edges_current (repo_id, path);"));
        assert!(sql.contains("CREATE INDEX IF NOT EXISTS artefact_edges_current_from_idx"));
        assert!(sql.contains("ON artefact_edges_current (repo_id, from_symbol_id, edge_kind);"));
        assert!(!sql.contains("artefacts_current_branch_path_idx"));
        assert!(!sql.contains("artefacts_current_branch_kind_idx"));
        assert!(!sql.contains("artefacts_current_branch_fqn_idx"));
        assert!(!sql.contains("artefact_edges_current_branch_from_idx"));
        assert!(!sql.contains("artefact_edges_current_branch_to_idx"));
    }
}
