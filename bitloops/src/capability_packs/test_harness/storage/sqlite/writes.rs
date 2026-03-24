use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::models::{
    CommitRecord, CurrentFileStateRecord, CurrentProductionArtefactRecord,
    CurrentProductionEdgeRecord, FileStateRecord, ProductionArtefactRecord, ProductionEdgeRecord,
    RepositoryRecord, TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord,
    TestClassificationRecord, TestDiscoveryDiagnosticRecord, TestDiscoveryRunRecord, TestRunRecord,
};

pub(super) fn table_exists(conn: &Connection, table_name: &str) -> Result<bool> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table_name],
            |row| row.get(0),
        )
        .with_context(|| format!("failed checking for sqlite table `{table_name}`"))?;
    Ok(count > 0)
}

pub(super) fn clear_existing_production_data(conn: &Connection, commit_sha: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM test_artefact_edges_current WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing test_artefact_edges_current for commit")?;
    conn.execute(
        r#"DELETE FROM coverage_hits WHERE capture_id IN (
            SELECT capture_id FROM coverage_captures WHERE commit_sha = ?1
        )"#,
        params![commit_sha],
    )
    .context("failed clearing coverage_hits for commit")?;
    conn.execute(
        "DELETE FROM coverage_captures WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing coverage_captures for commit")?;
    conn.execute(
        "DELETE FROM test_classifications WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing test_classifications for commit")?;
    conn.execute(
        "DELETE FROM test_runs WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing test_runs for commit")?;
    conn.execute(
        "DELETE FROM artefact_edges_current WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing artefact_edges_current for commit")?;
    conn.execute(
        "DELETE FROM artefacts_current WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing artefacts_current for commit")?;
    if table_exists(conn, "current_file_state")? {
        conn.execute(
            "DELETE FROM current_file_state WHERE commit_sha = ?1",
            params![commit_sha],
        )
        .context("failed clearing current_file_state for commit")?;
    }
    conn.execute(
        "DELETE FROM file_state WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing file_state for commit")?;
    conn.execute(
        "DELETE FROM commits WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing commits for commit")?;
    Ok(())
}

pub(super) fn clear_existing_test_discovery_data(
    conn: &Connection,
    commit_sha: &str,
) -> Result<()> {
    conn.execute(
        "DELETE FROM test_classifications WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing existing test classifications for commit")?;
    conn.execute(
        r#"DELETE FROM coverage_hits WHERE capture_id IN (
            SELECT capture_id FROM coverage_captures WHERE commit_sha = ?1
        )"#,
        params![commit_sha],
    )
    .context("failed clearing existing coverage_hits for commit")?;
    conn.execute(
        "DELETE FROM coverage_captures WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing existing coverage_captures for commit")?;
    conn.execute(
        "DELETE FROM test_artefact_edges_current WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing existing test edges for commit")?;
    conn.execute(
        "DELETE FROM test_runs WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing existing test runs for commit")?;
    conn.execute(
        "DELETE FROM test_discovery_diagnostics WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing existing discovery diagnostics for commit")?;
    conn.execute(
        "DELETE FROM test_discovery_runs WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing existing discovery runs for commit")?;
    conn.execute(
        "DELETE FROM test_artefacts_current WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing existing test artefacts for commit")?;
    Ok(())
}

pub(super) fn upsert_repository(conn: &Connection, repository: &RepositoryRecord) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
VALUES (?1, ?2, ?3, ?4, ?5)
ON CONFLICT(repo_id) DO UPDATE SET
  provider = excluded.provider,
  organization = excluded.organization,
  name = excluded.name,
  default_branch = excluded.default_branch
"#,
        params![
            repository.repo_id,
            repository.provider,
            repository.organization,
            repository.name,
            repository.default_branch
        ],
    )
    .with_context(|| format!("failed upserting repository {}", repository.repo_id))?;
    Ok(())
}

pub(super) fn upsert_commit(conn: &Connection, commit: &CommitRecord) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO commits (
  commit_sha, repo_id, author_name, author_email, commit_message, committed_at
) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
ON CONFLICT(commit_sha) DO UPDATE SET
  repo_id = excluded.repo_id,
  author_name = excluded.author_name,
  author_email = excluded.author_email,
  commit_message = excluded.commit_message,
  committed_at = excluded.committed_at
"#,
        params![
            commit.commit_sha,
            commit.repo_id,
            commit.author_name,
            commit.author_email,
            commit.commit_message,
            commit.committed_at
        ],
    )
    .with_context(|| format!("failed upserting commit {}", commit.commit_sha))?;
    Ok(())
}

pub(super) fn upsert_file_state(conn: &Connection, row: &FileStateRecord) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO file_state (repo_id, commit_sha, path, blob_sha)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT(repo_id, commit_sha, path) DO UPDATE SET
  blob_sha = excluded.blob_sha
"#,
        params![row.repo_id, row.commit_sha, row.path, row.blob_sha],
    )
    .with_context(|| {
        format!(
            "failed upserting file_state {} {}",
            row.commit_sha, row.path
        )
    })?;
    Ok(())
}

pub(super) fn upsert_current_file_state(
    conn: &Connection,
    row: &CurrentFileStateRecord,
) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO current_file_state (repo_id, path, commit_sha, blob_sha, committed_at)
VALUES (?1, ?2, ?3, ?4, ?5)
ON CONFLICT(repo_id, path) DO UPDATE SET
  commit_sha = excluded.commit_sha,
  blob_sha = excluded.blob_sha,
  committed_at = excluded.committed_at,
  updated_at = datetime('now')
"#,
        params![
            row.repo_id,
            row.path,
            row.commit_sha,
            row.blob_sha,
            row.committed_at
        ],
    )
    .with_context(|| format!("failed upserting current_file_state {}", row.path))?;
    Ok(())
}

pub(super) fn upsert_production_artefact(
    conn: &Connection,
    artefact: &ProductionArtefactRecord,
) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO artefacts (
  artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind,
  language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte,
  end_byte, signature, modifiers, docstring, content_hash
) VALUES (
  ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18
)
ON CONFLICT(artefact_id) DO UPDATE SET
  symbol_id = excluded.symbol_id,
  repo_id = excluded.repo_id,
  blob_sha = excluded.blob_sha,
  path = excluded.path,
  language = excluded.language,
  canonical_kind = excluded.canonical_kind,
  language_kind = excluded.language_kind,
  symbol_fqn = excluded.symbol_fqn,
  parent_artefact_id = excluded.parent_artefact_id,
  start_line = excluded.start_line,
  end_line = excluded.end_line,
  start_byte = excluded.start_byte,
  end_byte = excluded.end_byte,
  signature = excluded.signature,
  modifiers = excluded.modifiers,
  docstring = excluded.docstring,
  content_hash = excluded.content_hash
"#,
        params![
            artefact.artefact_id,
            artefact.symbol_id,
            artefact.repo_id,
            artefact.blob_sha,
            artefact.path,
            artefact.language,
            artefact.canonical_kind,
            artefact.language_kind,
            artefact.symbol_fqn,
            artefact.parent_artefact_id,
            artefact.start_line,
            artefact.end_line,
            artefact.start_byte,
            artefact.end_byte,
            artefact.signature,
            artefact.modifiers,
            artefact.docstring,
            artefact.content_hash
        ],
    )
    .with_context(|| format!("failed upserting artefact {}", artefact.artefact_id))?;
    Ok(())
}

pub(super) fn upsert_current_production_artefact(
    conn: &Connection,
    artefact: &CurrentProductionArtefactRecord,
) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO artefacts_current (
  repo_id, branch, symbol_id, artefact_id, commit_sha, blob_sha, path, language, canonical_kind,
  language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line,
  start_byte, end_byte, signature, modifiers, docstring, content_hash
) VALUES (
  ?1, 'main', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20
)
ON CONFLICT(repo_id, branch, symbol_id) DO UPDATE SET
  artefact_id = excluded.artefact_id,
  commit_sha = excluded.commit_sha,
  blob_sha = excluded.blob_sha,
  path = excluded.path,
  language = excluded.language,
  canonical_kind = excluded.canonical_kind,
  language_kind = excluded.language_kind,
  symbol_fqn = excluded.symbol_fqn,
  parent_symbol_id = excluded.parent_symbol_id,
  parent_artefact_id = excluded.parent_artefact_id,
  start_line = excluded.start_line,
  end_line = excluded.end_line,
  start_byte = excluded.start_byte,
  end_byte = excluded.end_byte,
  signature = excluded.signature,
  modifiers = excluded.modifiers,
  docstring = excluded.docstring,
  content_hash = excluded.content_hash,
  updated_at = datetime('now')
"#,
        params![
            artefact.repo_id,
            artefact.symbol_id,
            artefact.artefact_id,
            artefact.commit_sha,
            artefact.blob_sha,
            artefact.path,
            artefact.language,
            artefact.canonical_kind,
            artefact.language_kind,
            artefact.symbol_fqn,
            artefact.parent_symbol_id,
            artefact.parent_artefact_id,
            artefact.start_line,
            artefact.end_line,
            artefact.start_byte,
            artefact.end_byte,
            artefact.signature,
            artefact.modifiers,
            artefact.docstring,
            artefact.content_hash
        ],
    )
    .with_context(|| format!("failed upserting current artefact {}", artefact.symbol_id))?;
    Ok(())
}

pub(super) fn upsert_production_edge(conn: &Connection, edge: &ProductionEdgeRecord) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO artefact_edges (
  edge_id, repo_id, blob_sha, from_artefact_id, to_artefact_id, to_symbol_ref,
  edge_kind, language, start_line, end_line, metadata
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
ON CONFLICT(edge_id) DO UPDATE SET
  repo_id = excluded.repo_id,
  blob_sha = excluded.blob_sha,
  from_artefact_id = excluded.from_artefact_id,
  to_artefact_id = excluded.to_artefact_id,
  to_symbol_ref = excluded.to_symbol_ref,
  edge_kind = excluded.edge_kind,
  language = excluded.language,
  start_line = excluded.start_line,
  end_line = excluded.end_line,
  metadata = excluded.metadata
"#,
        params![
            edge.edge_id,
            edge.repo_id,
            edge.blob_sha,
            edge.from_artefact_id,
            edge.to_artefact_id,
            edge.to_symbol_ref,
            edge.edge_kind,
            edge.language,
            edge.start_line,
            edge.end_line,
            edge.metadata
        ],
    )
    .with_context(|| format!("failed upserting production edge {}", edge.edge_id))?;
    Ok(())
}

pub(super) fn upsert_current_production_edge(
    conn: &Connection,
    edge: &CurrentProductionEdgeRecord,
) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO artefact_edges_current (
  edge_id, repo_id, branch, commit_sha, blob_sha, path, from_symbol_id, from_artefact_id,
  to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line,
  end_line, metadata
) VALUES (?1, ?2, 'main', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
ON CONFLICT(repo_id, branch, edge_id) DO UPDATE SET
  repo_id = excluded.repo_id,
  commit_sha = excluded.commit_sha,
  blob_sha = excluded.blob_sha,
  path = excluded.path,
  from_symbol_id = excluded.from_symbol_id,
  from_artefact_id = excluded.from_artefact_id,
  to_symbol_id = excluded.to_symbol_id,
  to_artefact_id = excluded.to_artefact_id,
  to_symbol_ref = excluded.to_symbol_ref,
  edge_kind = excluded.edge_kind,
  language = excluded.language,
  start_line = excluded.start_line,
  end_line = excluded.end_line,
  metadata = excluded.metadata,
  updated_at = datetime('now')
"#,
        params![
            edge.edge_id,
            edge.repo_id,
            edge.commit_sha,
            edge.blob_sha,
            edge.path,
            edge.from_symbol_id,
            edge.from_artefact_id,
            edge.to_symbol_id,
            edge.to_artefact_id,
            edge.to_symbol_ref,
            edge.edge_kind,
            edge.language,
            edge.start_line,
            edge.end_line,
            edge.metadata
        ],
    )
    .with_context(|| format!("failed upserting current production edge {}", edge.edge_id))?;
    Ok(())
}

pub(super) fn upsert_test_artefact_current(
    conn: &Connection,
    artefact: &TestArtefactCurrentRecord,
) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO test_artefacts_current (
  artefact_id, symbol_id, repo_id, commit_sha, blob_sha, path, language, canonical_kind,
  language_kind, symbol_fqn, name, parent_artefact_id, parent_symbol_id, start_line,
  end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash,
  discovery_source, revision_kind, revision_id
) VALUES (
  ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
  ?21, ?22, ?23, ?24
)
ON CONFLICT(repo_id, symbol_id) DO UPDATE SET
  artefact_id = excluded.artefact_id,
  commit_sha = excluded.commit_sha,
  blob_sha = excluded.blob_sha,
  language = excluded.language,
  path = excluded.path,
  canonical_kind = excluded.canonical_kind,
  language_kind = excluded.language_kind,
  name = excluded.name,
  symbol_fqn = excluded.symbol_fqn,
  parent_artefact_id = excluded.parent_artefact_id,
  parent_symbol_id = excluded.parent_symbol_id,
  start_line = excluded.start_line,
  end_line = excluded.end_line,
  start_byte = excluded.start_byte,
  end_byte = excluded.end_byte,
  signature = excluded.signature,
  modifiers = excluded.modifiers,
  docstring = excluded.docstring,
  content_hash = excluded.content_hash,
  discovery_source = excluded.discovery_source,
  revision_kind = excluded.revision_kind,
  revision_id = excluded.revision_id,
  updated_at = datetime('now')
"#,
        params![
            artefact.artefact_id,
            artefact.symbol_id,
            artefact.repo_id,
            artefact.commit_sha,
            artefact.blob_sha,
            artefact.path,
            artefact.language,
            artefact.canonical_kind,
            artefact.language_kind,
            artefact.symbol_fqn,
            artefact.name,
            artefact.parent_artefact_id,
            artefact.parent_symbol_id,
            artefact.start_line,
            artefact.end_line,
            artefact.start_byte,
            artefact.end_byte,
            artefact.signature,
            artefact.modifiers,
            artefact.docstring,
            artefact.content_hash,
            artefact.discovery_source,
            artefact.revision_kind,
            artefact.revision_id
        ],
    )
    .with_context(|| format!("failed upserting test artefact {}", artefact.symbol_id))?;
    Ok(())
}

pub(super) fn upsert_test_artefact_edge_current(
    conn: &Connection,
    edge: &TestArtefactEdgeCurrentRecord,
) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO test_artefact_edges_current (
  edge_id, repo_id, commit_sha, blob_sha, path, from_artefact_id, from_symbol_id, to_artefact_id,
  to_symbol_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata,
  revision_kind, revision_id
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
ON CONFLICT(edge_id) DO UPDATE SET
  commit_sha = excluded.commit_sha,
  blob_sha = excluded.blob_sha,
  path = excluded.path,
  from_artefact_id = excluded.from_artefact_id,
  from_symbol_id = excluded.from_symbol_id,
  to_artefact_id = excluded.to_artefact_id,
  to_symbol_id = excluded.to_symbol_id,
  to_symbol_ref = excluded.to_symbol_ref,
  edge_kind = excluded.edge_kind,
  language = excluded.language,
  start_line = excluded.start_line,
  end_line = excluded.end_line,
  metadata = excluded.metadata,
  revision_kind = excluded.revision_kind,
  revision_id = excluded.revision_id,
  updated_at = datetime('now')
"#,
        params![
            edge.edge_id,
            edge.repo_id,
            edge.commit_sha,
            edge.blob_sha,
            edge.path,
            edge.from_artefact_id,
            edge.from_symbol_id,
            edge.to_artefact_id,
            edge.to_symbol_id,
            edge.to_symbol_ref,
            edge.edge_kind,
            edge.language,
            edge.start_line,
            edge.end_line,
            edge.metadata,
            edge.revision_kind,
            edge.revision_id
        ],
    )
    .with_context(|| format!("failed upserting test edge {}", edge.edge_id))?;
    Ok(())
}

pub(super) fn upsert_test_run(conn: &Connection, run: &TestRunRecord) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO test_runs (run_id, repo_id, commit_sha, test_symbol_id, status, duration_ms, ran_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
ON CONFLICT(run_id) DO UPDATE SET
  repo_id = excluded.repo_id,
  commit_sha = excluded.commit_sha,
  test_symbol_id = excluded.test_symbol_id,
  status = excluded.status,
  duration_ms = excluded.duration_ms,
  ran_at = excluded.ran_at
"#,
        params![
            run.run_id,
            run.repo_id,
            run.commit_sha,
            run.test_symbol_id,
            run.status,
            run.duration_ms,
            run.ran_at
        ],
    )
    .with_context(|| format!("failed inserting test run {}", run.run_id))?;
    Ok(())
}

pub(super) fn upsert_test_classification(
    conn: &Connection,
    record: &TestClassificationRecord,
) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO test_classifications (
  classification_id, repo_id, commit_sha, test_symbol_id, classification,
  classification_source, fan_out, boundary_crossings
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
ON CONFLICT(classification_id) DO UPDATE SET
  repo_id = excluded.repo_id,
  commit_sha = excluded.commit_sha,
  test_symbol_id = excluded.test_symbol_id,
  classification = excluded.classification,
  classification_source = excluded.classification_source,
  fan_out = excluded.fan_out,
  boundary_crossings = excluded.boundary_crossings
"#,
        params![
            record.classification_id,
            record.repo_id,
            record.commit_sha,
            record.test_symbol_id,
            record.classification,
            record.classification_source,
            record.fan_out,
            record.boundary_crossings
        ],
    )
    .with_context(|| format!("failed writing classification {}", record.classification_id))?;
    Ok(())
}

pub(super) fn upsert_test_discovery_run(
    conn: &Connection,
    run: &TestDiscoveryRunRecord,
) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO test_discovery_runs (
  discovery_run_id, repo_id, commit_sha, language, started_at, finished_at, status,
  enumeration_status, notes_json, stats_json
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
ON CONFLICT(discovery_run_id) DO UPDATE SET
  repo_id = excluded.repo_id,
  commit_sha = excluded.commit_sha,
  language = excluded.language,
  started_at = excluded.started_at,
  finished_at = excluded.finished_at,
  status = excluded.status,
  enumeration_status = excluded.enumeration_status,
  notes_json = excluded.notes_json,
  stats_json = excluded.stats_json
"#,
        params![
            run.discovery_run_id,
            run.repo_id,
            run.commit_sha,
            run.language,
            run.started_at,
            run.finished_at,
            run.status,
            run.enumeration_status,
            run.notes_json,
            run.stats_json
        ],
    )
    .with_context(|| format!("failed upserting discovery run {}", run.discovery_run_id))?;
    Ok(())
}

pub(super) fn upsert_test_discovery_diagnostic(
    conn: &Connection,
    diagnostic: &TestDiscoveryDiagnosticRecord,
) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO test_discovery_diagnostics (
  diagnostic_id, discovery_run_id, repo_id, commit_sha, path, line, severity, code,
  message, metadata_json
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
ON CONFLICT(diagnostic_id) DO UPDATE SET
  discovery_run_id = excluded.discovery_run_id,
  repo_id = excluded.repo_id,
  commit_sha = excluded.commit_sha,
  path = excluded.path,
  line = excluded.line,
  severity = excluded.severity,
  code = excluded.code,
  message = excluded.message,
  metadata_json = excluded.metadata_json
"#,
        params![
            diagnostic.diagnostic_id,
            diagnostic.discovery_run_id,
            diagnostic.repo_id,
            diagnostic.commit_sha,
            diagnostic.path,
            diagnostic.line,
            diagnostic.severity,
            diagnostic.code,
            diagnostic.message,
            diagnostic.metadata_json
        ],
    )
    .with_context(|| format!("failed upserting diagnostic {}", diagnostic.diagnostic_id))?;
    Ok(())
}
