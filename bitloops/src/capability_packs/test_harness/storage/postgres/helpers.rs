use anyhow::{Context, Result};
use tokio_postgres::{GenericClient, Row, types::FromSqlOwned};

use crate::models::{
    TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord, TestClassificationRecord,
    TestRunRecord,
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
    conn.execute("DELETE FROM test_artefact_edges_current", &[])
        .await
        .context("failed clearing existing test edges")?;
    conn.execute(
        "DELETE FROM test_runs WHERE commit_sha = $1",
        &[&commit_sha],
    )
    .await
    .context("failed clearing existing test runs for commit")?;
    conn.execute("DELETE FROM test_artefacts_current", &[])
        .await
        .context("failed clearing existing test artefacts")?;
    Ok(())
}

pub(super) async fn upsert_test_artefact_current(
    conn: &impl GenericClient,
    artefact: &TestArtefactCurrentRecord,
) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO test_artefacts_current (
  repo_id, path, content_id, symbol_id, artefact_id, language, canonical_kind,
  language_kind, symbol_fqn, name, parent_symbol_id, parent_artefact_id, start_line,
  end_line, start_byte, end_byte, signature, modifiers, docstring, discovery_source
) VALUES (
  $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20
)
ON CONFLICT(repo_id, path, symbol_id) DO UPDATE SET
  artefact_id = excluded.artefact_id,
  content_id = excluded.content_id,
  language = excluded.language,
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
  discovery_source = excluded.discovery_source,
  updated_at = now()
"#,
        &[
            &artefact.repo_id,
            &artefact.path,
            &artefact.content_id,
            &artefact.symbol_id,
            &artefact.artefact_id,
            &artefact.language,
            &artefact.canonical_kind,
            &artefact.language_kind,
            &artefact.symbol_fqn,
            &artefact.name,
            &artefact.parent_symbol_id,
            &artefact.parent_artefact_id,
            &artefact.start_line,
            &artefact.end_line,
            &artefact.start_byte,
            &artefact.end_byte,
            &artefact.signature,
            &artefact.modifiers,
            &artefact.docstring,
            &artefact.discovery_source,
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
  repo_id, path, content_id, edge_id, from_artefact_id, from_symbol_id, to_artefact_id,
  to_symbol_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata
) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
ON CONFLICT(repo_id, edge_id) DO UPDATE SET
  content_id = excluded.content_id,
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
  updated_at = now()
"#,
        &[
            &edge.repo_id,
            &edge.path,
            &edge.content_id,
            &edge.edge_id,
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
