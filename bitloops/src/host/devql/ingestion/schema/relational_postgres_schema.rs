use std::sync::OnceLock;

pub(crate) fn postgres_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS repositories (
    repo_id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    organization TEXT NOT NULL,
    name TEXT NOT NULL,
    default_branch TEXT,
    metadata_json TEXT,
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

CREATE INDEX IF NOT EXISTS file_state_path_blob_commit_idx
ON file_state (repo_id, path, blob_sha, commit_sha);

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
    secondary_context_ids_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    frameworks_json JSONB NOT NULL DEFAULT '[]'::jsonb,
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
    PRIMARY KEY (repo_id, path)
);

CREATE TABLE IF NOT EXISTS project_contexts_current (
    repo_id TEXT NOT NULL,
    context_id TEXT NOT NULL,
    root TEXT NOT NULL,
    kind TEXT NOT NULL,
    detection_source TEXT NOT NULL,
    frameworks_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    runtime_profile TEXT,
    config_files_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    config_fingerprint TEXT NOT NULL,
    source_versions_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    PRIMARY KEY (repo_id, context_id),
    FOREIGN KEY (repo_id) REFERENCES repositories(repo_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS checkpoint_files (
    relation_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    checkpoint_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    event_time TIMESTAMPTZ NOT NULL,
    agent TEXT NOT NULL DEFAULT '',
    branch TEXT NOT NULL DEFAULT '',
    strategy TEXT NOT NULL DEFAULT '',
    commit_sha TEXT NOT NULL,
    change_kind TEXT NOT NULL,
    path_before TEXT,
    path_after TEXT,
    blob_sha_before TEXT,
    blob_sha_after TEXT,
    copy_source_path TEXT,
    copy_source_blob_sha TEXT
);

CREATE INDEX IF NOT EXISTS checkpoint_files_lookup_idx
ON checkpoint_files (repo_id, path_after, blob_sha_after);

CREATE INDEX IF NOT EXISTS checkpoint_files_agent_time_idx
ON checkpoint_files (repo_id, agent, event_time DESC);

CREATE INDEX IF NOT EXISTS checkpoint_files_event_time_idx
ON checkpoint_files (repo_id, event_time DESC);

CREATE INDEX IF NOT EXISTS checkpoint_files_checkpoint_idx
ON checkpoint_files (repo_id, checkpoint_id);

CREATE INDEX IF NOT EXISTS checkpoint_files_commit_idx
ON checkpoint_files (repo_id, commit_sha);

CREATE INDEX IF NOT EXISTS checkpoint_files_change_kind_idx
ON checkpoint_files (repo_id, checkpoint_id, change_kind);

CREATE INDEX IF NOT EXISTS checkpoint_files_copy_source_idx
ON checkpoint_files (repo_id, copy_source_path, copy_source_blob_sha);

CREATE TABLE IF NOT EXISTS checkpoint_artefacts (
    relation_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    checkpoint_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    event_time TIMESTAMPTZ NOT NULL,
    agent TEXT NOT NULL DEFAULT '',
    branch TEXT NOT NULL DEFAULT '',
    strategy TEXT NOT NULL DEFAULT '',
    commit_sha TEXT NOT NULL,
    change_kind TEXT NOT NULL,
    before_symbol_id TEXT,
    after_symbol_id TEXT,
    before_artefact_id TEXT,
    after_artefact_id TEXT
);

CREATE INDEX IF NOT EXISTS checkpoint_artefacts_checkpoint_idx
ON checkpoint_artefacts (repo_id, checkpoint_id);

CREATE INDEX IF NOT EXISTS checkpoint_artefacts_before_artefact_idx
ON checkpoint_artefacts (repo_id, before_artefact_id);

CREATE INDEX IF NOT EXISTS checkpoint_artefacts_after_artefact_idx
ON checkpoint_artefacts (repo_id, after_artefact_id);

CREATE INDEX IF NOT EXISTS checkpoint_artefacts_before_symbol_idx
ON checkpoint_artefacts (repo_id, before_symbol_id);

CREATE INDEX IF NOT EXISTS checkpoint_artefacts_after_symbol_idx
ON checkpoint_artefacts (repo_id, after_symbol_id);

CREATE TABLE IF NOT EXISTS checkpoint_artefact_lineage (
    relation_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    checkpoint_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    event_time TIMESTAMPTZ NOT NULL,
    agent TEXT NOT NULL DEFAULT '',
    branch TEXT NOT NULL DEFAULT '',
    strategy TEXT NOT NULL DEFAULT '',
    commit_sha TEXT NOT NULL,
    lineage_kind TEXT NOT NULL,
    source_symbol_id TEXT NOT NULL,
    source_artefact_id TEXT NOT NULL,
    dest_symbol_id TEXT NOT NULL,
    dest_artefact_id TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS checkpoint_artefact_lineage_checkpoint_idx
ON checkpoint_artefact_lineage (repo_id, checkpoint_id);

CREATE INDEX IF NOT EXISTS checkpoint_artefact_lineage_source_idx
ON checkpoint_artefact_lineage (repo_id, source_artefact_id);

CREATE INDEX IF NOT EXISTS checkpoint_artefact_lineage_dest_idx
ON checkpoint_artefact_lineage (repo_id, dest_artefact_id);

CREATE TABLE IF NOT EXISTS artefacts (
    artefact_id TEXT PRIMARY KEY,
    symbol_id TEXT,
    repo_id TEXT NOT NULL,
    language TEXT NOT NULL,
    extraction_fingerprint TEXT,
    canonical_kind TEXT,
    language_kind TEXT,
    symbol_fqn TEXT,
    signature TEXT,
    modifiers JSONB NOT NULL DEFAULT '[]'::jsonb,
    docstring TEXT,
    content_hash TEXT,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS artefacts_kind_idx
ON artefacts (repo_id, canonical_kind);

CREATE INDEX IF NOT EXISTS artefacts_symbol_idx
ON artefacts (repo_id, symbol_id)
WHERE symbol_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS artefacts_symbol_content_hash_idx
ON artefacts (repo_id, symbol_id, content_hash)
WHERE symbol_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS artefacts_fqn_content_hash_idx
ON artefacts (repo_id, symbol_fqn, content_hash)
WHERE symbol_fqn IS NOT NULL;

CREATE TABLE IF NOT EXISTS artefact_snapshots (
    repo_id TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
    parent_artefact_id TEXT,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    created_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (repo_id, blob_sha, artefact_id)
);

CREATE INDEX IF NOT EXISTS artefact_snapshots_path_idx
ON artefact_snapshots (repo_id, path, blob_sha);

CREATE INDEX IF NOT EXISTS artefact_snapshots_parent_idx
ON artefact_snapshots (repo_id, parent_artefact_id);

CREATE INDEX IF NOT EXISTS artefact_snapshots_artefact_blob_idx
ON artefact_snapshots (repo_id, artefact_id, blob_sha);

CREATE INDEX IF NOT EXISTS artefact_snapshots_path_blob_line_idx
ON artefact_snapshots (repo_id, path, blob_sha, start_line, end_line);

CREATE OR REPLACE VIEW artefacts_historical AS
SELECT
    a.artefact_id AS artefact_id,
    a.symbol_id AS symbol_id,
    a.repo_id AS repo_id,
    s.blob_sha AS blob_sha,
    s.path AS path,
    a.language AS language,
    a.canonical_kind AS canonical_kind,
    a.language_kind AS language_kind,
    a.symbol_fqn AS symbol_fqn,
    s.parent_artefact_id AS parent_artefact_id,
    s.start_line AS start_line,
    s.end_line AS end_line,
    s.start_byte AS start_byte,
    s.end_byte AS end_byte,
    a.signature AS signature,
    a.modifiers AS modifiers,
    a.docstring AS docstring,
    a.content_hash AS content_hash,
    a.created_at AS created_at
FROM artefact_snapshots s
JOIN artefacts a
  ON a.repo_id = s.repo_id
 AND a.artefact_id = s.artefact_id;

CREATE TABLE IF NOT EXISTS artefacts_current (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
    language TEXT NOT NULL,
    extraction_fingerprint TEXT,
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

CREATE INDEX IF NOT EXISTS artefact_edges_from_blob_kind_idx
ON artefact_edges (repo_id, from_artefact_id, blob_sha, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_to_blob_kind_idx
ON artefact_edges (repo_id, to_artefact_id, blob_sha, edge_kind);

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
    scope_exclusions_fingerprint TEXT,
    last_sync_started_at TEXT,
    last_sync_completed_at TEXT,
    last_sync_status TEXT,
    last_sync_reason TEXT,
    FOREIGN KEY (repo_id) REFERENCES repositories(repo_id) ON DELETE CASCADE
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
    modifiers JSONB NOT NULL DEFAULT '[]',
    docstring TEXT,
    metadata JSONB NOT NULL DEFAULT '{}',
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
    metadata JSONB NOT NULL DEFAULT '{}',
    PRIMARY KEY (content_id, language, extraction_fingerprint, parser_version, extractor_version, edge_key)
);
"#
}

pub(crate) fn postgres_shared_schema_sql() -> &'static str {
    static SQL: OnceLock<String> = OnceLock::new();
    SQL.get_or_init(|| {
        build_schema_subset_sql(
            postgres_schema_sql(),
            &[
                "repositories",
                "commits",
                "file_state",
                "checkpoint_files",
                "checkpoint_artefacts",
                "checkpoint_artefact_lineage",
                "artefacts",
                "artefact_snapshots",
                "artefacts_historical",
                "artefact_edges",
                "commit_ingest_ledger",
            ],
        )
    })
    .as_str()
}

fn build_schema_subset_sql(full_sql: &str, included_objects: &[&str]) -> String {
    full_sql
        .split(";\n")
        .filter_map(|statement| {
            let statement = statement.trim();
            if statement.is_empty() {
                return None;
            }
            let object_name = schema_statement_object_name(statement)?;
            included_objects
                .iter()
                .any(|candidate| *candidate == object_name)
                .then(|| format!("{statement};\n"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn schema_statement_object_name(statement: &str) -> Option<&str> {
    for prefix in [
        "CREATE TABLE IF NOT EXISTS ",
        "CREATE VIEW IF NOT EXISTS ",
        "CREATE OR REPLACE VIEW ",
    ] {
        if let Some(rest) = statement.strip_prefix(prefix) {
            return rest.split_whitespace().next();
        }
    }
    if statement.starts_with("CREATE INDEX IF NOT EXISTS ")
        || statement.starts_with("CREATE UNIQUE INDEX IF NOT EXISTS ")
    {
        return statement.split_once("ON ").map(|(_, rest)| {
            rest.split_whitespace()
                .next()
                .unwrap_or_default()
                .trim_end_matches('(')
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{postgres_schema_sql, postgres_shared_schema_sql};

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

    #[test]
    fn postgres_shared_schema_sql_excludes_current_projection_tables() {
        let sql = postgres_shared_schema_sql();
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS artefacts ("));
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS commit_ingest_ledger ("));
        assert!(!sql.contains("CREATE TABLE IF NOT EXISTS current_file_state ("));
        assert!(!sql.contains("CREATE TABLE IF NOT EXISTS artefacts_current ("));
        assert!(!sql.contains("CREATE TABLE IF NOT EXISTS artefact_edges_current ("));
        assert!(!sql.contains("CREATE TABLE IF NOT EXISTS repo_sync_state ("));
        assert!(!sql.contains("CREATE TABLE IF NOT EXISTS workspace_revisions ("));
        assert!(!sql.contains("CREATE TABLE IF NOT EXISTS content_cache ("));
    }
}
