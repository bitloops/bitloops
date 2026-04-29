pub fn codecity_sqlite_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS codecity_floor_health_current (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    floor_index INTEGER NOT NULL,
    artefact_id TEXT,
    symbol_id TEXT,
    commit_sha TEXT,
    config_fingerprint TEXT NOT NULL,
    health_risk REAL,
    health_status TEXT NOT NULL,
    health_confidence REAL NOT NULL,
    colour TEXT NOT NULL,
    churn INTEGER NOT NULL,
    complexity REAL NOT NULL,
    bug_count INTEGER NOT NULL,
    coverage REAL,
    author_concentration REAL,
    distinct_authors INTEGER NOT NULL,
    commits_touching INTEGER NOT NULL,
    bug_fix_commits INTEGER NOT NULL,
    covered_lines INTEGER,
    total_coverable_lines INTEGER,
    complexity_source TEXT NOT NULL,
    coverage_source TEXT NOT NULL,
    git_history_source TEXT NOT NULL,
    missing_signals_json TEXT NOT NULL DEFAULT '[]',
    updated_at TEXT NOT NULL,
    PRIMARY KEY (repo_id, path, floor_index)
);

CREATE TABLE IF NOT EXISTS codecity_file_health_current (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    commit_sha TEXT,
    config_fingerprint TEXT NOT NULL,
    health_risk REAL,
    health_status TEXT NOT NULL,
    health_confidence REAL NOT NULL,
    colour TEXT NOT NULL,
    floor_count INTEGER NOT NULL,
    high_risk_floor_count INTEGER NOT NULL,
    insufficient_data_floor_count INTEGER NOT NULL,
    average_floor_risk REAL,
    max_floor_risk REAL,
    missing_signals_json TEXT NOT NULL DEFAULT '[]',
    summary_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL,
    PRIMARY KEY (repo_id, path)
);

CREATE TABLE IF NOT EXISTS codecity_health_runs_current (
    repo_id TEXT PRIMARY KEY,
    commit_sha TEXT,
    config_fingerprint TEXT NOT NULL,
    health_status TEXT NOT NULL,
    health_generated_at TEXT,
    health_summary_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS codecity_dependency_evidence_current (
    repo_id TEXT NOT NULL,
    evidence_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    commit_sha TEXT,
    from_path TEXT NOT NULL,
    to_path TEXT,
    to_symbol_ref TEXT,
    from_boundary_id TEXT,
    to_boundary_id TEXT,
    from_zone TEXT,
    to_zone TEXT,
    from_symbol_id TEXT,
    from_artefact_id TEXT,
    to_symbol_id TEXT,
    to_artefact_id TEXT,
    edge_id TEXT,
    edge_kind TEXT NOT NULL,
    language TEXT,
    start_line INTEGER,
    end_line INTEGER,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    resolved INTEGER NOT NULL DEFAULT 0,
    cross_boundary INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    PRIMARY KEY (repo_id, evidence_id)
);

CREATE INDEX IF NOT EXISTS codecity_dependency_evidence_from_idx
ON codecity_dependency_evidence_current (repo_id, from_path);

CREATE INDEX IF NOT EXISTS codecity_dependency_evidence_to_idx
ON codecity_dependency_evidence_current (repo_id, to_path);

CREATE INDEX IF NOT EXISTS codecity_dependency_evidence_boundary_idx
ON codecity_dependency_evidence_current (repo_id, from_boundary_id, to_boundary_id);

CREATE INDEX IF NOT EXISTS codecity_dependency_evidence_edge_kind_idx
ON codecity_dependency_evidence_current (repo_id, edge_kind);

CREATE TABLE IF NOT EXISTS codecity_file_dependency_arcs_current (
    repo_id TEXT NOT NULL,
    arc_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    commit_sha TEXT,
    from_path TEXT NOT NULL,
    to_path TEXT NOT NULL,
    from_boundary_id TEXT,
    to_boundary_id TEXT,
    from_zone TEXT,
    to_zone TEXT,
    edge_count INTEGER NOT NULL,
    import_count INTEGER NOT NULL DEFAULT 0,
    call_count INTEGER NOT NULL DEFAULT 0,
    reference_count INTEGER NOT NULL DEFAULT 0,
    export_count INTEGER NOT NULL DEFAULT 0,
    inheritance_count INTEGER NOT NULL DEFAULT 0,
    weight REAL NOT NULL,
    cross_boundary INTEGER NOT NULL DEFAULT 0,
    has_violation INTEGER NOT NULL DEFAULT 0,
    highest_severity TEXT,
    evidence_ids_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    PRIMARY KEY (repo_id, arc_id)
);

CREATE INDEX IF NOT EXISTS codecity_file_dependency_arcs_from_idx
ON codecity_file_dependency_arcs_current (repo_id, from_path);

CREATE INDEX IF NOT EXISTS codecity_file_dependency_arcs_to_idx
ON codecity_file_dependency_arcs_current (repo_id, to_path);

CREATE INDEX IF NOT EXISTS codecity_file_dependency_arcs_cross_boundary_idx
ON codecity_file_dependency_arcs_current (repo_id, cross_boundary, weight DESC);

CREATE INDEX IF NOT EXISTS codecity_file_dependency_arcs_violation_idx
ON codecity_file_dependency_arcs_current (repo_id, has_violation, highest_severity);

CREATE TABLE IF NOT EXISTS codecity_architecture_violations_current (
    repo_id TEXT NOT NULL,
    violation_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    commit_sha TEXT,
    boundary_id TEXT,
    boundary_root TEXT,
    pattern TEXT NOT NULL,
    rule TEXT NOT NULL,
    severity TEXT NOT NULL,
    from_path TEXT NOT NULL,
    to_path TEXT,
    from_zone TEXT,
    to_zone TEXT,
    from_boundary_id TEXT,
    to_boundary_id TEXT,
    arc_id TEXT,
    message TEXT NOT NULL,
    explanation TEXT NOT NULL,
    recommendation TEXT,
    evidence_ids_json TEXT NOT NULL DEFAULT '[]',
    evidence_json TEXT NOT NULL DEFAULT '[]',
    confidence REAL NOT NULL DEFAULT 1.0,
    suppressed INTEGER NOT NULL DEFAULT 0,
    suppression_reason TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY (repo_id, violation_id)
);

CREATE INDEX IF NOT EXISTS codecity_architecture_violations_boundary_idx
ON codecity_architecture_violations_current (repo_id, boundary_id, severity);

CREATE INDEX IF NOT EXISTS codecity_architecture_violations_from_idx
ON codecity_architecture_violations_current (repo_id, from_path);

CREATE INDEX IF NOT EXISTS codecity_architecture_violations_to_idx
ON codecity_architecture_violations_current (repo_id, to_path);

CREATE INDEX IF NOT EXISTS codecity_architecture_violations_rule_idx
ON codecity_architecture_violations_current (repo_id, pattern, rule, severity);

CREATE TABLE IF NOT EXISTS codecity_render_arcs_current (
    repo_id TEXT NOT NULL,
    render_arc_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    commit_sha TEXT,
    arc_kind TEXT NOT NULL,
    visibility TEXT NOT NULL,
    severity TEXT,
    from_path TEXT,
    to_path TEXT,
    from_boundary_id TEXT,
    to_boundary_id TEXT,
    source_arc_id TEXT,
    violation_id TEXT,
    weight REAL NOT NULL DEFAULT 1.0,
    label TEXT,
    tooltip TEXT,
    from_x REAL NOT NULL,
    from_y REAL NOT NULL,
    from_z REAL NOT NULL,
    to_x REAL NOT NULL,
    to_y REAL NOT NULL,
    to_z REAL NOT NULL,
    control_y REAL NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    PRIMARY KEY (repo_id, render_arc_id)
);

CREATE INDEX IF NOT EXISTS codecity_render_arcs_kind_idx
ON codecity_render_arcs_current (repo_id, arc_kind, visibility);

CREATE INDEX IF NOT EXISTS codecity_render_arcs_file_idx
ON codecity_render_arcs_current (repo_id, from_path, to_path);

CREATE INDEX IF NOT EXISTS codecity_render_arcs_boundary_idx
ON codecity_render_arcs_current (repo_id, from_boundary_id, to_boundary_id);
"#
}
