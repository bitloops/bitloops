pub const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS repositories (
    repo_id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    organization TEXT NOT NULL,
    name TEXT NOT NULL,
    default_branch TEXT,
    created_at TEXT DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS commits (
    commit_sha TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    author_name TEXT,
    author_email TEXT,
    commit_message TEXT,
    committed_at TEXT
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
    commit_sha TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    committed_at TEXT NOT NULL,
    updated_at TEXT DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, path)
);

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
    modifiers TEXT NOT NULL DEFAULT '[]',
    docstring TEXT,
    content_hash TEXT,
    created_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS artefacts_blob_idx
ON artefacts (repo_id, blob_sha);

CREATE INDEX IF NOT EXISTS artefacts_path_idx
ON artefacts (repo_id, path);

CREATE INDEX IF NOT EXISTS artefacts_kind_idx
ON artefacts (repo_id, canonical_kind);

CREATE INDEX IF NOT EXISTS artefacts_symbol_idx
ON artefacts (repo_id, symbol_id);

CREATE TABLE IF NOT EXISTS artefacts_current (
    repo_id TEXT NOT NULL,
    symbol_id TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    revision_kind TEXT NOT NULL DEFAULT 'commit',
    revision_id TEXT NOT NULL DEFAULT '',
    temp_checkpoint_id INTEGER,
    blob_sha TEXT NOT NULL,
    path TEXT NOT NULL,
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
    modifiers TEXT NOT NULL DEFAULT '[]',
    docstring TEXT,
    content_hash TEXT,
    updated_at TEXT DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, symbol_id)
);

CREATE INDEX IF NOT EXISTS artefacts_current_path_idx
ON artefacts_current (repo_id, path);

CREATE INDEX IF NOT EXISTS artefacts_current_kind_idx
ON artefacts_current (repo_id, canonical_kind);

CREATE INDEX IF NOT EXISTS artefacts_current_artefact_idx
ON artefacts_current (repo_id, artefact_id);

CREATE INDEX IF NOT EXISTS artefacts_current_symbol_fqn_idx
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
    metadata TEXT DEFAULT '{}',
    created_at TEXT DEFAULT (datetime('now')),
    CHECK (to_artefact_id IS NOT NULL OR to_symbol_ref IS NOT NULL),
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
ON artefact_edges (repo_id, edge_kind, to_symbol_ref);

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
    edge_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    revision_kind TEXT NOT NULL DEFAULT 'commit',
    revision_id TEXT NOT NULL DEFAULT '',
    temp_checkpoint_id INTEGER,
    blob_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    from_symbol_id TEXT NOT NULL,
    from_artefact_id TEXT NOT NULL,
    to_symbol_id TEXT,
    to_artefact_id TEXT,
    to_symbol_ref TEXT,
    edge_kind TEXT NOT NULL,
    language TEXT NOT NULL,
    start_line INTEGER,
    end_line INTEGER,
    metadata TEXT DEFAULT '{}',
    updated_at TEXT DEFAULT (datetime('now')),
    CHECK (to_symbol_id IS NOT NULL OR to_symbol_ref IS NOT NULL),
    CHECK (
        (start_line IS NULL AND end_line IS NULL)
        OR (start_line IS NOT NULL AND end_line IS NOT NULL AND start_line > 0 AND end_line >= start_line)
    )
);

CREATE INDEX IF NOT EXISTS artefact_edges_current_path_idx
ON artefact_edges_current (repo_id, path);

CREATE INDEX IF NOT EXISTS artefact_edges_current_from_idx
ON artefact_edges_current (repo_id, from_symbol_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_current_to_idx
ON artefact_edges_current (repo_id, to_symbol_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_current_kind_idx
ON artefact_edges_current (repo_id, edge_kind);

CREATE INDEX IF NOT EXISTS artefact_edges_current_symbol_ref_idx
ON artefact_edges_current (repo_id, to_symbol_ref);

CREATE UNIQUE INDEX IF NOT EXISTS artefact_edges_current_natural_uq
ON artefact_edges_current (
    repo_id,
    from_symbol_id,
    edge_kind,
    COALESCE(to_symbol_id, ''),
    COALESCE(to_symbol_ref, ''),
    COALESCE(start_line, -1),
    COALESCE(end_line, -1),
    COALESCE(metadata, '{}')
);

CREATE TABLE IF NOT EXISTS test_artefacts_current (
    artefact_id TEXT NOT NULL,
    symbol_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    language TEXT NOT NULL,
    canonical_kind TEXT NOT NULL,
    language_kind TEXT,
    symbol_fqn TEXT,
    name TEXT NOT NULL,
    parent_artefact_id TEXT,
    parent_symbol_id TEXT,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER,
    end_byte INTEGER,
    signature TEXT,
    modifiers TEXT NOT NULL DEFAULT '[]',
    docstring TEXT,
    content_hash TEXT,
    discovery_source TEXT NOT NULL,
    revision_kind TEXT NOT NULL DEFAULT 'commit',
    revision_id TEXT NOT NULL DEFAULT '',
    updated_at TEXT DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, symbol_id)
);

CREATE INDEX IF NOT EXISTS idx_test_artefacts_current_path
ON test_artefacts_current (repo_id, path);

CREATE INDEX IF NOT EXISTS idx_test_artefacts_current_kind
ON test_artefacts_current (repo_id, canonical_kind);

CREATE INDEX IF NOT EXISTS idx_test_artefacts_current_parent
ON test_artefacts_current (repo_id, parent_symbol_id);

CREATE TABLE IF NOT EXISTS test_artefact_edges_current (
    edge_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    blob_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    from_artefact_id TEXT NOT NULL,
    from_symbol_id TEXT NOT NULL,
    to_artefact_id TEXT,
    to_symbol_id TEXT,
    to_symbol_ref TEXT,
    edge_kind TEXT NOT NULL,
    language TEXT NOT NULL,
    start_line INTEGER,
    end_line INTEGER,
    metadata TEXT DEFAULT '{}',
    revision_kind TEXT NOT NULL DEFAULT 'commit',
    revision_id TEXT NOT NULL DEFAULT '',
    updated_at TEXT DEFAULT (datetime('now')),
    CHECK (to_symbol_id IS NOT NULL OR to_symbol_ref IS NOT NULL)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_test_artefact_edges_current_natural
ON test_artefact_edges_current (repo_id, from_symbol_id, edge_kind, to_symbol_id, to_symbol_ref);

CREATE INDEX IF NOT EXISTS idx_test_artefact_edges_current_from
ON test_artefact_edges_current (repo_id, from_symbol_id);

CREATE INDEX IF NOT EXISTS idx_test_artefact_edges_current_to
ON test_artefact_edges_current (repo_id, to_symbol_id);

CREATE INDEX IF NOT EXISTS idx_test_artefact_edges_current_path
ON test_artefact_edges_current (repo_id, path);

CREATE TABLE IF NOT EXISTS test_runs (
    run_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    test_symbol_id TEXT NOT NULL,
    status TEXT NOT NULL,
    duration_ms INTEGER,
    ran_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS test_runs_commit_idx
ON test_runs (repo_id, commit_sha, test_symbol_id);

CREATE INDEX IF NOT EXISTS test_runs_latest_idx
ON test_runs (repo_id, test_symbol_id, ran_at);

CREATE TABLE IF NOT EXISTS test_classifications (
    classification_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    test_symbol_id TEXT NOT NULL,
    classification TEXT NOT NULL,
    classification_source TEXT NOT NULL DEFAULT 'coverage_derived',
    fan_out INTEGER NOT NULL,
    boundary_crossings INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS test_classifications_commit_idx
ON test_classifications (repo_id, commit_sha, test_symbol_id);

CREATE TABLE IF NOT EXISTS coverage_captures (
    capture_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    tool TEXT NOT NULL DEFAULT 'unknown',
    format TEXT NOT NULL DEFAULT 'lcov',
    scope_kind TEXT NOT NULL DEFAULT 'workspace',
    subject_test_symbol_id TEXT,
    line_truth INTEGER NOT NULL DEFAULT 1,
    branch_truth INTEGER NOT NULL DEFAULT 0,
    captured_at TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'complete',
    metadata_json TEXT
);

CREATE INDEX IF NOT EXISTS coverage_captures_commit_scope_idx
ON coverage_captures (repo_id, commit_sha, scope_kind);

CREATE TABLE IF NOT EXISTS coverage_hits (
    capture_id TEXT NOT NULL REFERENCES coverage_captures(capture_id) ON DELETE CASCADE,
    production_symbol_id TEXT NOT NULL,
    file_path TEXT NOT NULL,
    line INTEGER NOT NULL,
    branch_id INTEGER NOT NULL DEFAULT -1,
    covered INTEGER NOT NULL,
    hit_count INTEGER DEFAULT 0,
    PRIMARY KEY (capture_id, production_symbol_id, line, branch_id)
);

CREATE INDEX IF NOT EXISTS coverage_hits_production_idx
ON coverage_hits (production_symbol_id, capture_id);

CREATE TABLE IF NOT EXISTS coverage_diagnostics (
    diagnostic_id TEXT PRIMARY KEY,
    capture_id TEXT REFERENCES coverage_captures(capture_id) ON DELETE CASCADE,
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    path TEXT,
    line INTEGER,
    severity TEXT NOT NULL,
    code TEXT NOT NULL,
    message TEXT NOT NULL,
    metadata_json TEXT
);

CREATE INDEX IF NOT EXISTS coverage_diagnostics_commit_idx
ON coverage_diagnostics (repo_id, commit_sha);

CREATE INDEX IF NOT EXISTS coverage_diagnostics_capture_idx
ON coverage_diagnostics (capture_id);

CREATE TABLE IF NOT EXISTS test_discovery_runs (
    discovery_run_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    language TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    status TEXT NOT NULL,
    enumeration_status TEXT,
    notes_json TEXT,
    stats_json TEXT
);

CREATE INDEX IF NOT EXISTS test_discovery_runs_commit_idx
ON test_discovery_runs (repo_id, commit_sha);

CREATE TABLE IF NOT EXISTS test_discovery_diagnostics (
    diagnostic_id TEXT PRIMARY KEY,
    discovery_run_id TEXT REFERENCES test_discovery_runs(discovery_run_id) ON DELETE CASCADE,
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    path TEXT,
    line INTEGER,
    severity TEXT NOT NULL,
    code TEXT NOT NULL,
    message TEXT NOT NULL,
    metadata_json TEXT
);

CREATE INDEX IF NOT EXISTS test_discovery_diagnostics_commit_idx
ON test_discovery_diagnostics (repo_id, commit_sha);

CREATE INDEX IF NOT EXISTS test_discovery_diagnostics_run_idx
ON test_discovery_diagnostics (discovery_run_id);
"#;
