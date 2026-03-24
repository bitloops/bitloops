use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

pub mod schema;
pub mod seed;

/// SQLite DDL for the full test-domain schema (test discovery, coverage, classifications, etc.).
pub fn test_domain_schema_sql() -> &'static str {
    TEST_DOMAIN_SCHEMA_SQL
}

const TEST_DOMAIN_SCHEMA_SQL: &str = r#"
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

pub fn init_database(db_path: &Path, seed: bool, commit_sha: &str) -> Result<()> {
    ensure_parent_dir_exists(db_path)?;

    let mut conn = Connection::open(db_path).with_context(|| {
        format!(
            "failed to open or create sqlite database at {}",
            db_path.display()
        )
    })?;

    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .context("failed to enable foreign keys")?;

    conn.execute_batch(schema::SCHEMA_SQL)
        .context("failed to create schema")?;

    if seed {
        let seeded = seed::seed_database(&mut conn, commit_sha)?;
        println!(
            "seeded {} production artefacts for commit {}",
            seeded.artefacts, commit_sha
        );
    }

    println!("database initialized at {}", db_path.display());
    Ok(())
}

pub fn init_test_domain_database(db_path: &Path) -> Result<()> {
    ensure_parent_dir_exists(db_path)?;

    let conn = Connection::open(db_path).with_context(|| {
        format!(
            "failed to open or create sqlite database at {}",
            db_path.display()
        )
    })?;

    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .context("failed to enable foreign keys")?;
    conn.execute_batch(test_domain_schema_sql())
        .context("failed to create test-domain schema")?;

    println!("test-domain schema initialized at {}", db_path.display());
    Ok(())
}

pub fn open_existing_database(db_path: &Path) -> Result<Connection> {
    if !db_path.exists() {
        anyhow::bail!("Database not found. Run init-fixture-db.sh first.");
    }

    let conn = Connection::open(db_path)
        .with_context(|| format!("failed to open sqlite database at {}", db_path.display()))?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .context("failed to enable foreign keys")?;
    Ok(conn)
}

fn ensure_parent_dir_exists(db_path: &Path) -> Result<()> {
    if let Some(parent) = db_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create parent directory for db path {}",
                db_path.display()
            )
        })?;
    }

    Ok(())
}
