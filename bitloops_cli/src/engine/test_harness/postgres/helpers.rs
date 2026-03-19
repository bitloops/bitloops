use anyhow::{Context, Result};
use tokio_postgres::{GenericClient, Row, types::FromSqlOwned};

use crate::domain::{
    ListedArtefactRecord, TestClassificationRecord, TestDiscoveryDiagnosticRecord,
    TestDiscoveryRunRecord, TestLinkRecord, TestRunRecord, TestScenarioRecord, TestSuiteRecord,
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
        "DELETE FROM test_links WHERE commit_sha = $1",
        &[&commit_sha],
    )
    .await
    .context("failed clearing existing test links for commit")?;
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
        "DELETE FROM test_scenarios WHERE commit_sha = $1",
        &[&commit_sha],
    )
    .await
    .context("failed clearing existing test scenarios for commit")?;
    conn.execute(
        "DELETE FROM test_suites WHERE commit_sha = $1",
        &[&commit_sha],
    )
    .await
    .context("failed clearing existing test suites for commit")?;
    Ok(())
}

pub(super) async fn upsert_test_suite(
    conn: &impl GenericClient,
    suite: &TestSuiteRecord,
) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO test_suites (
  suite_id, repo_id, commit_sha, language, path, name, symbol_fqn, start_line,
  end_line, start_byte, end_byte, signature, discovery_source
) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
ON CONFLICT(suite_id) DO UPDATE SET
  repo_id = excluded.repo_id,
  commit_sha = excluded.commit_sha,
  language = excluded.language,
  path = excluded.path,
  name = excluded.name,
  symbol_fqn = excluded.symbol_fqn,
  start_line = excluded.start_line,
  end_line = excluded.end_line,
  start_byte = excluded.start_byte,
  end_byte = excluded.end_byte,
  signature = excluded.signature,
  discovery_source = excluded.discovery_source
"#,
        &[
            &suite.suite_id,
            &suite.repo_id,
            &suite.commit_sha,
            &suite.language,
            &suite.path,
            &suite.name,
            &suite.symbol_fqn,
            &suite.start_line,
            &suite.end_line,
            &suite.start_byte,
            &suite.end_byte,
            &suite.signature,
            &suite.discovery_source,
        ],
    )
    .await
    .with_context(|| format!("failed upserting test suite {}", suite.suite_id))?;
    Ok(())
}

pub(super) async fn upsert_test_scenario(
    conn: &impl GenericClient,
    scenario: &TestScenarioRecord,
) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO test_scenarios (
  scenario_id, suite_id, repo_id, commit_sha, language, path, name, symbol_fqn,
  start_line, end_line, start_byte, end_byte, signature, discovery_source
) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
ON CONFLICT(scenario_id) DO UPDATE SET
  suite_id = excluded.suite_id,
  repo_id = excluded.repo_id,
  commit_sha = excluded.commit_sha,
  language = excluded.language,
  path = excluded.path,
  name = excluded.name,
  symbol_fqn = excluded.symbol_fqn,
  start_line = excluded.start_line,
  end_line = excluded.end_line,
  start_byte = excluded.start_byte,
  end_byte = excluded.end_byte,
  signature = excluded.signature,
  discovery_source = excluded.discovery_source
"#,
        &[
            &scenario.scenario_id,
            &scenario.suite_id,
            &scenario.repo_id,
            &scenario.commit_sha,
            &scenario.language,
            &scenario.path,
            &scenario.name,
            &scenario.symbol_fqn,
            &scenario.start_line,
            &scenario.end_line,
            &scenario.start_byte,
            &scenario.end_byte,
            &scenario.signature,
            &scenario.discovery_source,
        ],
    )
    .await
    .with_context(|| format!("failed upserting test scenario {}", scenario.scenario_id))?;
    Ok(())
}

pub(super) async fn upsert_test_link(
    conn: &impl GenericClient,
    link: &TestLinkRecord,
) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO test_links (
  test_link_id, repo_id, commit_sha, test_scenario_id, production_artefact_id,
  production_symbol_id, link_source, evidence_json
) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
ON CONFLICT(test_link_id) DO UPDATE SET
  repo_id = excluded.repo_id,
  commit_sha = excluded.commit_sha,
  test_scenario_id = excluded.test_scenario_id,
  production_artefact_id = excluded.production_artefact_id,
  production_symbol_id = excluded.production_symbol_id,
  link_source = excluded.link_source,
  evidence_json = excluded.evidence_json
"#,
        &[
            &link.test_link_id,
            &link.repo_id,
            &link.commit_sha,
            &link.test_scenario_id,
            &link.production_artefact_id,
            &link.production_symbol_id,
            &link.link_source,
            &link.evidence_json,
        ],
    )
    .await
    .with_context(|| format!("failed upserting test link {}", link.test_link_id))?;
    Ok(())
}

pub(super) async fn upsert_test_run(conn: &impl GenericClient, run: &TestRunRecord) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO test_runs (run_id, repo_id, commit_sha, test_scenario_id, status, duration_ms, ran_at)
VALUES ($1, $2, $3, $4, $5, $6, $7)
ON CONFLICT(run_id) DO UPDATE SET
  repo_id = excluded.repo_id,
  commit_sha = excluded.commit_sha,
  test_scenario_id = excluded.test_scenario_id,
  status = excluded.status,
  duration_ms = excluded.duration_ms,
  ran_at = excluded.ran_at
"#,
        &[
            &run.run_id,
            &run.repo_id,
            &run.commit_sha,
            &run.test_scenario_id,
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
  classification_id, repo_id, commit_sha, test_scenario_id, classification,
  classification_source, fan_out, boundary_crossings
) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
ON CONFLICT(classification_id) DO UPDATE SET
  repo_id = excluded.repo_id,
  commit_sha = excluded.commit_sha,
  test_scenario_id = excluded.test_scenario_id,
  classification = excluded.classification,
  classification_source = excluded.classification_source,
  fan_out = excluded.fan_out,
  boundary_crossings = excluded.boundary_crossings
"#,
        &[
            &record.classification_id,
            &record.repo_id,
            &record.commit_sha,
            &record.test_scenario_id,
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
SELECT DISTINCT a.artefact_id, a.symbol_fqn, a.canonical_kind, a.path, a.start_line, a.end_line
FROM file_state fs
JOIN artefacts a
  ON a.repo_id = fs.repo_id
 AND a.blob_sha = fs.blob_sha
 AND a.path = fs.path
WHERE fs.commit_sha = $1
  AND a.canonical_kind = $2
ORDER BY a.path ASC, a.start_line ASC
"#,
            &[&commit_sha, &kind],
        )
        .await
        .context("failed querying production artefacts")?
    } else {
        conn.query(
            r#"
SELECT DISTINCT a.artefact_id, a.symbol_fqn, a.canonical_kind, a.path, a.start_line, a.end_line
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
SELECT suite_id, symbol_fqn, path, start_line, end_line
FROM test_suites
WHERE commit_sha = $1
ORDER BY path ASC, start_line ASC
"#,
            &[&commit_sha],
        )
        .await
        .context("failed querying test suites")?;

    rows.into_iter()
        .map(|row| {
            Ok(ListedArtefactRecord {
                artefact_id: get(&row, 0, "suite_id")?,
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
SELECT scenario_id, symbol_fqn, path, start_line, end_line
FROM test_scenarios
WHERE commit_sha = $1
ORDER BY path ASC, start_line ASC
"#,
            &[&commit_sha],
        )
        .await
        .context("failed querying test scenarios")?;

    rows.into_iter()
        .map(|row| {
            Ok(ListedArtefactRecord {
                artefact_id: get(&row, 0, "scenario_id")?,
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
