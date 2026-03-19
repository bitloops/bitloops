pub(crate) fn postgres_test_domain_schema_sql() -> &'static str {
    r#"
CREATE TABLE IF NOT EXISTS test_suites (
    suite_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    language TEXT NOT NULL,
    path TEXT NOT NULL,
    name TEXT NOT NULL,
    symbol_fqn TEXT,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER,
    end_byte INTEGER,
    signature TEXT,
    discovery_source TEXT NOT NULL,
    created_at DATETIME DEFAULT now()
);

CREATE INDEX IF NOT EXISTS test_suites_commit_idx
ON test_suites (repo_id, commit_sha);

CREATE INDEX IF NOT EXISTS test_suites_path_idx
ON test_suites (repo_id, commit_sha, path);

CREATE TABLE IF NOT EXISTS test_scenarios (
    scenario_id TEXT PRIMARY KEY,
    suite_id TEXT REFERENCES test_suites(suite_id) ON DELETE CASCADE,
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    language TEXT NOT NULL,
    path TEXT NOT NULL,
    name TEXT NOT NULL,
    symbol_fqn TEXT,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    start_byte INTEGER,
    end_byte INTEGER,
    signature TEXT,
    discovery_source TEXT NOT NULL,
    created_at DATETIME DEFAULT now()
);

CREATE INDEX IF NOT EXISTS test_scenarios_commit_idx
ON test_scenarios (repo_id, commit_sha);

CREATE INDEX IF NOT EXISTS test_scenarios_suite_idx
ON test_scenarios (repo_id, commit_sha, suite_id);

CREATE INDEX IF NOT EXISTS test_scenarios_path_idx
ON test_scenarios (repo_id, commit_sha, path);

CREATE TABLE IF NOT EXISTS test_links (
    test_link_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    test_scenario_id TEXT NOT NULL REFERENCES test_scenarios(scenario_id) ON DELETE CASCADE,
    production_artefact_id TEXT NOT NULL REFERENCES artefacts(artefact_id) ON DELETE CASCADE,
    production_symbol_id TEXT,
    link_source TEXT NOT NULL DEFAULT 'static_analysis',
    evidence_json TEXT DEFAULT '{}',
    created_at DATETIME DEFAULT now()
);

CREATE INDEX IF NOT EXISTS test_links_production_idx
ON test_links (repo_id, commit_sha, production_artefact_id);

CREATE INDEX IF NOT EXISTS test_links_scenario_idx
ON test_links (repo_id, commit_sha, test_scenario_id);

CREATE UNIQUE INDEX IF NOT EXISTS test_links_natural_uq
ON test_links (commit_sha, test_scenario_id, production_artefact_id, link_source);

CREATE TABLE IF NOT EXISTS test_runs (
    run_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    test_scenario_id TEXT NOT NULL REFERENCES test_scenarios(scenario_id) ON DELETE CASCADE,
    status TEXT NOT NULL,
    duration_ms INTEGER,
    ran_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS test_runs_commit_idx
ON test_runs (repo_id, commit_sha, test_scenario_id);

CREATE INDEX IF NOT EXISTS test_runs_latest_idx
ON test_runs (repo_id, test_scenario_id, ran_at);

CREATE TABLE IF NOT EXISTS test_classifications (
    classification_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    test_scenario_id TEXT NOT NULL REFERENCES test_scenarios(scenario_id) ON DELETE CASCADE,
    classification TEXT NOT NULL,
    classification_source TEXT NOT NULL DEFAULT 'coverage_derived',
    fan_out INTEGER NOT NULL,
    boundary_crossings INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS test_classifications_commit_idx
ON test_classifications (repo_id, commit_sha, test_scenario_id);

CREATE TABLE IF NOT EXISTS coverage_captures (
    capture_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    tool TEXT NOT NULL DEFAULT 'unknown',
    format TEXT NOT NULL DEFAULT 'lcov',
    scope_kind TEXT NOT NULL DEFAULT 'workspace',
    subject_test_scenario_id TEXT REFERENCES test_scenarios(scenario_id) ON DELETE SET NULL,
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
    production_artefact_id TEXT NOT NULL REFERENCES artefacts(artefact_id) ON DELETE CASCADE,
    file_path TEXT NOT NULL,
    line INTEGER NOT NULL,
    branch_id INTEGER NOT NULL DEFAULT -1,
    covered INTEGER NOT NULL,
    hit_count INTEGER DEFAULT 0,
    PRIMARY KEY (capture_id, production_artefact_id, line, branch_id)
);

CREATE INDEX IF NOT EXISTS coverage_hits_production_idx
ON coverage_hits (production_artefact_id, capture_id);

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
"#
}

#[cfg(test)]
mod tests {
    use super::postgres_test_domain_schema_sql;

    #[test]
    fn postgres_test_domain_schema_includes_core_tables() {
        let sql = postgres_test_domain_schema_sql();
        for table in [
            "test_suites",
            "test_scenarios",
            "test_links",
            "coverage_captures",
            "coverage_hits",
            "test_discovery_runs",
        ] {
            assert!(
                sql.contains(table),
                "expected Postgres test-domain schema to include `{table}`"
            );
        }
    }
}
