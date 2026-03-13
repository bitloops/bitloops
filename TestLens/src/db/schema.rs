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

CREATE TABLE IF NOT EXISTS test_coverage (
  coverage_id TEXT PRIMARY KEY,
  repo_id TEXT NOT NULL,
  commit_sha TEXT NOT NULL,
  test_artefact_id TEXT NOT NULL REFERENCES artefacts(artefact_id),
  artefact_id TEXT NOT NULL REFERENCES artefacts(artefact_id),
  line INTEGER NOT NULL,
  branch_id INTEGER,
  covered INTEGER NOT NULL,
  hit_count INTEGER DEFAULT 0
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
"#;
