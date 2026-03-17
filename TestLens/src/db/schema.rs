pub const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS artefacts (
  artefact_id TEXT PRIMARY KEY,
  symbol_id TEXT,
  repo_id TEXT NOT NULL,
  blob_sha TEXT,
  commit_sha TEXT NOT NULL,
  path TEXT NOT NULL,
  language TEXT NOT NULL,
  canonical_kind TEXT NOT NULL,
  language_kind TEXT,
  symbol_fqn TEXT,
  parent_artefact_id TEXT,
  start_line INTEGER NOT NULL,
  end_line INTEGER NOT NULL,
  start_byte INTEGER,
  end_byte INTEGER,
  signature TEXT,
  content_hash TEXT
);

CREATE TABLE IF NOT EXISTS test_links (
  test_link_id TEXT PRIMARY KEY,
  test_artefact_id TEXT NOT NULL REFERENCES artefacts(artefact_id),
  production_artefact_id TEXT NOT NULL REFERENCES artefacts(artefact_id),
  link_source TEXT NOT NULL DEFAULT 'static_analysis',
  commit_sha TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS coverage_captures (
  capture_id TEXT PRIMARY KEY,
  repo_id TEXT NOT NULL,
  commit_sha TEXT NOT NULL,
  tool TEXT NOT NULL DEFAULT 'unknown',
  format TEXT NOT NULL DEFAULT 'lcov',
  scope_kind TEXT NOT NULL DEFAULT 'workspace',
  subject_test_artefact_id TEXT REFERENCES artefacts(artefact_id),
  line_truth INTEGER NOT NULL DEFAULT 1,
  branch_truth INTEGER NOT NULL DEFAULT 0,
  captured_at TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'complete',
  metadata_json TEXT
);

CREATE TABLE IF NOT EXISTS coverage_hits (
  capture_id TEXT NOT NULL REFERENCES coverage_captures(capture_id),
  artefact_id TEXT NOT NULL REFERENCES artefacts(artefact_id),
  file_path TEXT NOT NULL,
  line INTEGER NOT NULL,
  branch_id INTEGER NOT NULL DEFAULT -1,
  covered INTEGER NOT NULL,
  hit_count INTEGER DEFAULT 0,
  PRIMARY KEY (capture_id, artefact_id, line, branch_id)
);

CREATE TABLE IF NOT EXISTS test_runs (
  run_id TEXT PRIMARY KEY,
  repo_id TEXT NOT NULL,
  commit_sha TEXT NOT NULL,
  test_artefact_id TEXT NOT NULL REFERENCES artefacts(artefact_id),
  status TEXT NOT NULL,
  duration_ms INTEGER,
  ran_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS test_classifications (
  classification_id TEXT PRIMARY KEY,
  test_artefact_id TEXT NOT NULL REFERENCES artefacts(artefact_id),
  commit_sha TEXT NOT NULL,
  classification TEXT NOT NULL,
  classification_source TEXT NOT NULL DEFAULT 'coverage_derived',
  fan_out INTEGER NOT NULL,
  boundary_crossings INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_coverage_hits_artefact
  ON coverage_hits(artefact_id, capture_id);

CREATE INDEX IF NOT EXISTS idx_coverage_captures_commit_scope
  ON coverage_captures(commit_sha, scope_kind);
"#;
