use anyhow::{Context, Result};
use tokio_postgres::{GenericClient, Row, types::FromSqlOwned};

use crate::models::{
    ListedArtefactRecord, TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord,
    TestClassificationRecord, TestDiscoveryDiagnosticRecord, TestDiscoveryRunRecord, TestRunRecord,
};

pub(super) async fn clear_existing_test_discovery_data(
    conn: &impl GenericClient,
    commit_sha: &str,
) -> Result<()> {
    conn.execute(
        "DELETE FROM test_classifications WHERE commit_sha = $1",
        &[&commit_sha],
    )
    .await
    .context("failed clearing existing test classifications for commit")?;
    conn.execute(
        r#"DELETE FROM coverage_hits WHERE capture_id IN (
            SELECT capture_id FROM coverage_captures WHERE commit_sha = $1
        )"#,
        &[&commit_sha],
    )
    .await
    .context("failed clearing existing coverage_hits for commit")?;
    conn.execute(
        "DELETE FROM coverage_captures WHERE commit_sha = $1",
        &[&commit_sha],
    )
    .await
    .context("failed clearing existing coverage_captures for commit")?;
    conn.execute(
        "DELETE FROM test_artefact_edges_current WHERE commit_sha = $1",
        &[&commit_sha],
    )
    .await
    .context("failed clearing existing test edges for commit")?;
    conn.execute(
        "DELETE FROM test_runs WHERE commit_sha = $1",
        &[&commit_sha],
    )
    .await
    .context("failed clearing existing test runs for commit")?;
    conn.execute(
        "DELETE FROM test_discovery_diagnostics WHERE commit_sha = $1",
        &[&commit_sha],
    )
    .await
    .context("failed clearing existing discovery diagnostics for commit")?;
    conn.execute(
        "DELETE FROM test_discovery_runs WHERE commit_sha = $1",
        &[&commit_sha],
    )
    .await
    .context("failed clearing existing discovery runs for commit")?;
    conn.execute(
        "DELETE FROM test_artefacts_current WHERE commit_sha = $1",
        &[&commit_sha],
    )
    .await
    .context("failed clearing existing test artefacts for commit")?;
    Ok(())
}

pub(super) async fn upsert_test_artefact_current(
    conn: &impl GenericClient,
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
  $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20,
  $21, $22, $23, $24
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
  updated_at = now()
"#,
        &[
            &artefact.artefact_id,
            &artefact.symbol_id,
            &artefact.repo_id,
            &artefact.commit_sha,
            &artefact.blob_sha,
            &artefact.path,
            &artefact.language,
            &artefact.canonical_kind,
            &artefact.language_kind,
            &artefact.symbol_fqn,
            &artefact.name,
            &artefact.parent_artefact_id,
            &artefact.parent_symbol_id,
            &artefact.start_line,
            &artefact.end_line,
            &artefact.start_byte,
            &artefact.end_byte,
            &artefact.signature,
            &artefact.modifiers,
            &artefact.docstring,
            &artefact.content_hash,
            &artefact.discovery_source,
            &artefact.revision_kind,
            &artefact.revision_id,
        ],
    )
    .await
    .with_context(|| format!("failed upserting test artefact {}", artefact.symbol_id))?;
    Ok(())
}

pub(super) async fn upsert_test_artefact_edge_current(
    conn: &impl GenericClient,
    edge: &TestArtefactEdgeCurrentRecord,
) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO test_artefact_edges_current (
  edge_id, repo_id, commit_sha, blob_sha, path, from_artefact_id, from_symbol_id, to_artefact_id,
  to_symbol_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata,
  revision_kind, revision_id
) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
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
  updated_at = now()
"#,
        &[
            &edge.edge_id,
            &edge.repo_id,
            &edge.commit_sha,
            &edge.blob_sha,
            &edge.path,
            &edge.from_artefact_id,
            &edge.from_symbol_id,
            &edge.to_artefact_id,
            &edge.to_symbol_id,
            &edge.to_symbol_ref,
            &edge.edge_kind,
            &edge.language,
            &edge.start_line,
            &edge.end_line,
            &edge.metadata,
            &edge.revision_kind,
            &edge.revision_id,
        ],
    )
    .await
    .with_context(|| format!("failed upserting test edge {}", edge.edge_id))?;
    Ok(())
}

pub(super) async fn upsert_test_run(conn: &impl GenericClient, run: &TestRunRecord) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO test_runs (run_id, repo_id, commit_sha, test_symbol_id, status, duration_ms, ran_at)
VALUES ($1, $2, $3, $4, $5, $6, $7)
ON CONFLICT(run_id) DO UPDATE SET
  repo_id = excluded.repo_id,
  commit_sha = excluded.commit_sha,
  test_symbol_id = excluded.test_symbol_id,
  status = excluded.status,
  duration_ms = excluded.duration_ms,
  ran_at = excluded.ran_at
"#,
        &[
            &run.run_id,
            &run.repo_id,
            &run.commit_sha,
            &run.test_symbol_id,
            &run.status,
            &run.duration_ms,
            &run.ran_at,
        ],
    )
    .await
    .with_context(|| format!("failed inserting test run {}", run.run_id))?;
    Ok(())
}

pub(super) async fn upsert_test_classification(
    conn: &impl GenericClient,
    record: &TestClassificationRecord,
) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO test_classifications (
  classification_id, repo_id, commit_sha, test_symbol_id, classification,
  classification_source, fan_out, boundary_crossings
) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
ON CONFLICT(classification_id) DO UPDATE SET
  repo_id = excluded.repo_id,
  commit_sha = excluded.commit_sha,
  test_symbol_id = excluded.test_symbol_id,
  classification = excluded.classification,
  classification_source = excluded.classification_source,
  fan_out = excluded.fan_out,
  boundary_crossings = excluded.boundary_crossings
"#,
        &[
            &record.classification_id,
            &record.repo_id,
            &record.commit_sha,
            &record.test_symbol_id,
            &record.classification,
            &record.classification_source,
            &record.fan_out,
            &record.boundary_crossings,
        ],
    )
    .await
    .with_context(|| format!("failed writing classification {}", record.classification_id))?;
    Ok(())
}

pub(super) async fn upsert_test_discovery_run(
    conn: &impl GenericClient,
    run: &TestDiscoveryRunRecord,
) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO test_discovery_runs (
  discovery_run_id, repo_id, commit_sha, language, started_at, finished_at, status,
  enumeration_status, notes_json, stats_json
) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
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
        &[
            &run.discovery_run_id,
            &run.repo_id,
            &run.commit_sha,
            &run.language,
            &run.started_at,
            &run.finished_at,
            &run.status,
            &run.enumeration_status,
            &run.notes_json,
            &run.stats_json,
        ],
    )
    .await
    .with_context(|| format!("failed upserting discovery run {}", run.discovery_run_id))?;
    Ok(())
}

pub(super) async fn upsert_test_discovery_diagnostic(
    conn: &impl GenericClient,
    diagnostic: &TestDiscoveryDiagnosticRecord,
) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO test_discovery_diagnostics (
  diagnostic_id, discovery_run_id, repo_id, commit_sha, path, line, severity, code,
  message, metadata_json
) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
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
        &[
            &diagnostic.diagnostic_id,
            &diagnostic.discovery_run_id,
            &diagnostic.repo_id,
            &diagnostic.commit_sha,
            &diagnostic.path,
            &diagnostic.line,
            &diagnostic.severity,
            &diagnostic.code,
            &diagnostic.message,
            &diagnostic.metadata_json,
        ],
    )
    .await
    .with_context(|| format!("failed upserting diagnostic {}", diagnostic.diagnostic_id))?;
    Ok(())
}

pub(super) async fn load_listed_production_artefacts(
    conn: &impl GenericClient,
    commit_sha: &str,
    kind: Option<&str>,
) -> Result<Vec<ListedArtefactRecord>> {
    let rows = if let Some(kind) = kind {
        conn.query(
            r#"
SELECT DISTINCT
  a.artefact_id,
  a.symbol_fqn,
  LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'unknown'))) AS kind,
  a.path,
  a.start_line,
  a.end_line
FROM file_state fs
JOIN artefacts a
  ON a.repo_id = fs.repo_id
 AND a.blob_sha = fs.blob_sha
 AND a.path = fs.path
WHERE fs.commit_sha = $1
  AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'unknown'))) = LOWER($2)
ORDER BY a.path ASC, a.start_line ASC
"#,
            &[&commit_sha, &kind],
        )
        .await
        .context("failed querying production artefacts")?
    } else {
        conn.query(
            r#"
SELECT DISTINCT
  a.artefact_id,
  a.symbol_fqn,
  LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'unknown'))) AS kind,
  a.path,
  a.start_line,
  a.end_line
FROM file_state fs
JOIN artefacts a
  ON a.repo_id = fs.repo_id
 AND a.blob_sha = fs.blob_sha
 AND a.path = fs.path
WHERE fs.commit_sha = $1
ORDER BY a.path ASC, a.start_line ASC
"#,
            &[&commit_sha],
        )
        .await
        .context("failed querying production artefacts")?
    };

    rows.into_iter()
        .map(|row| {
            Ok(ListedArtefactRecord {
                artefact_id: get(&row, 0, "artefact_id")?,
                symbol_fqn: get(&row, 1, "symbol_fqn")?,
                kind: get(&row, 2, "kind")?,
                file_path: get(&row, 3, "file_path")?,
                start_line: get_i64(&row, 4, "start_line")?,
                end_line: get_i64(&row, 5, "end_line")?,
            })
        })
        .collect()
}

pub(super) async fn load_listed_test_suites(
    conn: &impl GenericClient,
    commit_sha: &str,
) -> Result<Vec<ListedArtefactRecord>> {
    let rows = conn
        .query(
            r#"
SELECT artefact_id, symbol_fqn, path, start_line, end_line
FROM test_artefacts_current
WHERE commit_sha = $1
  AND canonical_kind = 'test_suite'
ORDER BY path ASC, start_line ASC
"#,
            &[&commit_sha],
        )
        .await
        .context("failed querying test suites")?;

    rows.into_iter()
        .map(|row| {
            Ok(ListedArtefactRecord {
                artefact_id: get(&row, 0, "artefact_id")?,
                symbol_fqn: get(&row, 1, "symbol_fqn")?,
                kind: "test_suite".to_string(),
                file_path: get(&row, 2, "path")?,
                start_line: get_i64(&row, 3, "start_line")?,
                end_line: get_i64(&row, 4, "end_line")?,
            })
        })
        .collect()
}

pub(super) async fn load_listed_test_scenarios(
    conn: &impl GenericClient,
    commit_sha: &str,
) -> Result<Vec<ListedArtefactRecord>> {
    let rows = conn
        .query(
            r#"
SELECT artefact_id, symbol_fqn, path, start_line, end_line
FROM test_artefacts_current
WHERE commit_sha = $1
  AND canonical_kind = 'test_scenario'
ORDER BY path ASC, start_line ASC
"#,
            &[&commit_sha],
        )
        .await
        .context("failed querying test scenarios")?;

    rows.into_iter()
        .map(|row| {
            Ok(ListedArtefactRecord {
                artefact_id: get(&row, 0, "artefact_id")?,
                symbol_fqn: get(&row, 1, "symbol_fqn")?,
                kind: "test_scenario".to_string(),
                file_path: get(&row, 2, "path")?,
                start_line: get_i64(&row, 3, "start_line")?,
                end_line: get_i64(&row, 4, "end_line")?,
            })
        })
        .collect()
}

pub(super) fn get<T>(row: &Row, index: usize, field: &str) -> Result<T>
where
    T: FromSqlOwned,
{
    row.try_get(index)
        .with_context(|| format!("missing {field}"))
}

pub(super) fn get_i64(row: &Row, index: usize, field: &str) -> Result<i64> {
    row.try_get::<_, i64>(index)
        .or_else(|_| row.try_get::<_, i32>(index).map(i64::from))
        .with_context(|| format!("missing {field}"))
}

pub(super) fn get_opt_i64(row: &Row, index: usize, field: &str) -> Result<Option<i64>> {
    row.try_get::<_, Option<i64>>(index)
        .or_else(|_| {
            row.try_get::<_, Option<i32>>(index)
                .map(|value| value.map(i64::from))
        })
        .with_context(|| format!("missing {field}"))
}
