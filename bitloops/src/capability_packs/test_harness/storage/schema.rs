pub(crate) fn sqlite_test_domain_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS test_artefacts_current (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
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
    discovery_source TEXT NOT NULL,
    updated_at TEXT DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, path, symbol_id),
    UNIQUE (repo_id, artefact_id)
);

CREATE INDEX IF NOT EXISTS idx_test_artefacts_current_path
ON test_artefacts_current (repo_id, path);

CREATE INDEX IF NOT EXISTS idx_test_artefacts_current_kind
ON test_artefacts_current (repo_id, canonical_kind);

CREATE INDEX IF NOT EXISTS idx_test_artefacts_current_parent
ON test_artefacts_current (repo_id, parent_symbol_id);

CREATE TABLE IF NOT EXISTS test_artefact_edges_current (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    edge_id TEXT NOT NULL,
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
    updated_at TEXT DEFAULT (datetime('now')),
    PRIMARY KEY (repo_id, edge_id),
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

"#
}

pub(crate) fn postgres_test_domain_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS test_artefacts_current (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    symbol_id TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
    language TEXT NOT NULL,
    canonical_kind TEXT NOT NULL,
    language_kind TEXT,
    symbol_fqn TEXT,
    name TEXT NOT NULL,
    parent_artefact_id TEXT,
    parent_symbol_id TEXT,
    start_line BIGINT NOT NULL,
    end_line BIGINT NOT NULL,
    start_byte BIGINT,
    end_byte BIGINT,
    signature TEXT,
    modifiers TEXT NOT NULL DEFAULT '[]',
    docstring TEXT,
    discovery_source TEXT NOT NULL,
    updated_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (repo_id, path, symbol_id),
    UNIQUE (repo_id, artefact_id)
);

CREATE INDEX IF NOT EXISTS idx_test_artefacts_current_path
ON test_artefacts_current (repo_id, path);

CREATE INDEX IF NOT EXISTS idx_test_artefacts_current_kind
ON test_artefacts_current (repo_id, canonical_kind);

CREATE INDEX IF NOT EXISTS idx_test_artefacts_current_parent
ON test_artefacts_current (repo_id, parent_symbol_id);

CREATE TABLE IF NOT EXISTS test_artefact_edges_current (
    repo_id TEXT NOT NULL,
    path TEXT NOT NULL,
    content_id TEXT NOT NULL,
    edge_id TEXT NOT NULL,
    from_artefact_id TEXT NOT NULL,
    from_symbol_id TEXT NOT NULL,
    to_artefact_id TEXT,
    to_symbol_id TEXT,
    to_symbol_ref TEXT,
    edge_kind TEXT NOT NULL,
    language TEXT NOT NULL,
    start_line BIGINT,
    end_line BIGINT,
    metadata TEXT DEFAULT '{}',
    updated_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (repo_id, edge_id),
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
    duration_ms BIGINT,
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
    fan_out BIGINT NOT NULL,
    boundary_crossings BIGINT NOT NULL DEFAULT 0
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
    line_truth BIGINT NOT NULL DEFAULT 1,
    branch_truth BIGINT NOT NULL DEFAULT 0,
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
    line BIGINT NOT NULL,
    branch_id BIGINT NOT NULL DEFAULT -1,
    covered BIGINT NOT NULL,
    hit_count BIGINT DEFAULT 0,
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
    line BIGINT,
    severity TEXT NOT NULL,
    code TEXT NOT NULL,
    message TEXT NOT NULL,
    metadata_json TEXT
);

CREATE INDEX IF NOT EXISTS coverage_diagnostics_commit_idx
ON coverage_diagnostics (repo_id, commit_sha);

CREATE INDEX IF NOT EXISTS coverage_diagnostics_capture_idx
ON coverage_diagnostics (capture_id);

"#
}

#[cfg(test)]
mod tests {
    use super::{postgres_test_domain_schema_sql, sqlite_test_domain_schema_sql};

    #[test]
    fn sqlite_test_domain_schema_includes_core_tables() {
        let sql = sqlite_test_domain_schema_sql();
        for table in [
            "test_artefacts_current",
            "test_artefact_edges_current",
            "coverage_captures",
            "coverage_hits",
            "coverage_diagnostics",
        ] {
            assert!(
                sql.contains(table),
                "expected SQLite test-domain schema to include `{table}`"
            );
        }

        for legacy_table in ["test_suites", "test_scenarios", "test_links"] {
            assert!(
                !sql.contains(legacy_table),
                "did not expect SQLite test-domain schema to include legacy table `{legacy_table}`"
            );
        }
    }

    #[test]
    fn sqlite_test_domain_schema_uses_symbol_based_test_references() {
        let sql = sqlite_test_domain_schema_sql();

        for column in [
            "test_symbol_id TEXT NOT NULL",
            "subject_test_symbol_id TEXT",
            "production_symbol_id TEXT NOT NULL",
            "PRIMARY KEY (capture_id, production_symbol_id, line, branch_id)",
        ] {
            assert!(
                sql.contains(column),
                "expected SQLite test-domain schema to include `{column}`"
            );
        }

        for legacy_column in [
            "test_scenario_id TEXT NOT NULL",
            "subject_test_scenario_id TEXT",
            "production_artefact_id TEXT NOT NULL",
            "PRIMARY KEY (capture_id, production_artefact_id, line, branch_id)",
        ] {
            assert!(
                !sql.contains(legacy_column),
                "did not expect SQLite test-domain schema to include legacy column `{legacy_column}`"
            );
        }
    }

    #[test]
    fn postgres_test_domain_schema_includes_core_tables() {
        let sql = postgres_test_domain_schema_sql();
        for table in [
            "test_artefacts_current",
            "test_artefact_edges_current",
            "coverage_captures",
            "coverage_hits",
            "coverage_diagnostics",
        ] {
            assert!(
                sql.contains(table),
                "expected Postgres test-domain schema to include `{table}`"
            );
        }

        for legacy_table in ["test_suites", "test_scenarios", "test_links"] {
            assert!(
                !sql.contains(legacy_table),
                "did not expect Postgres test-domain schema to include legacy table `{legacy_table}`"
            );
        }
    }

    #[test]
    fn postgres_test_domain_schema_uses_symbol_based_test_references() {
        let sql = postgres_test_domain_schema_sql();

        for column in [
            "test_symbol_id TEXT NOT NULL",
            "subject_test_symbol_id TEXT",
            "production_symbol_id TEXT NOT NULL",
            "PRIMARY KEY (capture_id, production_symbol_id, line, branch_id)",
        ] {
            assert!(
                sql.contains(column),
                "expected Postgres test-domain schema to include `{column}`"
            );
        }

        for legacy_column in [
            "test_scenario_id TEXT NOT NULL",
            "subject_test_scenario_id TEXT",
            "production_artefact_id TEXT NOT NULL",
            "PRIMARY KEY (capture_id, production_artefact_id, line, branch_id)",
        ] {
            assert!(
                !sql.contains(legacy_column),
                "did not expect Postgres test-domain schema to include legacy column `{legacy_column}`"
            );
        }
    }
}
