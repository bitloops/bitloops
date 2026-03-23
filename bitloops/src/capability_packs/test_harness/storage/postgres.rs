mod commit_counts;
mod helpers;
mod stage_serving;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;

use anyhow::{Context, Result, anyhow, bail};

use crate::capability_packs::test_harness::storage::schema::postgres_test_domain_schema_sql;
use crate::capability_packs::test_harness::storage::{
    TestHarnessQueryRepository, TestHarnessRepository,
};
use crate::models::{
    CoverageBranchRecord, CoverageCaptureRecord, CoverageDiagnosticRecord, CoverageHitRecord,
    CoveragePairStats, CoverageSummaryRecord, CoveringTestRecord, LatestTestRunRecord,
    ProductionArtefact, ProductionIngestionBatch, QueriedArtefactRecord,
    ResolvedTestScenarioRecord, StageBranchCoverageRecord, StageCoverageMetadataRecord,
    StageCoveringTestRecord, StageLineCoverageRecord, TestClassificationRecord,
    TestDiscoveryDiagnosticRecord, TestDiscoveryRunRecord, TestHarnessCommitCounts,
    TestLinkRecord, TestRunRecord, TestScenarioRecord, TestSuiteRecord,
    derive_test_classification,
};
use crate::storage::PostgresSyncConnection;

use self::helpers::{
    clear_existing_test_discovery_data, get, get_i64, get_opt_i64,
    load_listed_production_artefacts, load_listed_test_scenarios, load_listed_test_suites,
    upsert_test_classification, upsert_test_discovery_diagnostic, upsert_test_discovery_run,
    upsert_test_link, upsert_test_run, upsert_test_scenario, upsert_test_suite,
};

pub struct PostgresTestHarnessRepository {
    postgres: PostgresSyncConnection,
}

impl PostgresTestHarnessRepository {
    pub fn connect(dsn: impl Into<String>) -> Result<Self> {
        Ok(Self {
            postgres: PostgresSyncConnection::connect(dsn)?,
        })
    }

    pub fn initialise_schema(&self) -> Result<()> {
        self.postgres
            .execute_batch(postgres_test_domain_schema_sql())
            .context("initialising Postgres test-domain schema")
    }

    fn with_client<T>(
        &self,
        operation: impl for<'a> FnOnce(
            &'a mut tokio_postgres::Client,
        ) -> Pin<Box<dyn Future<Output = Result<T>> + 'a>>,
    ) -> Result<T> {
        self.postgres.with_client(operation)
    }
}

impl TestHarnessRepository for PostgresTestHarnessRepository {
    fn load_repo_id_for_commit(&self, commit_sha: &str) -> Result<String> {
        let commit_sha = commit_sha.to_string();
        self.with_client(move |client| {
            Box::pin(async move {
                let row = client
                    .query_opt(
                        "SELECT repo_id FROM commits WHERE commit_sha = $1 LIMIT 1",
                        &[&commit_sha],
                    )
                    .await
                    .context("failed preparing repo lookup query")?
                    .ok_or_else(|| {
                        anyhow!(
                            "no production artefacts found for commit {}; materialize production artefacts first (use `bitloops devql ingest` for Bitloops-backed stores or `testlens ingest-production-artefacts` in prototype mode)",
                            commit_sha
                        )
                    })?;
                get(&row, 0, "repo_id")
            })
        })
    }

    fn load_production_artefacts(&self, commit_sha: &str) -> Result<Vec<ProductionArtefact>> {
        let commit_sha = commit_sha.to_string();
        self.with_client(move |client| {
            Box::pin(async move {
                let rows = client
                    .query(
                        r#"
SELECT DISTINCT a.artefact_id, a.symbol_id, COALESCE(a.symbol_fqn, ''), a.path, a.start_line
FROM file_state fs
JOIN artefacts a
  ON a.repo_id = fs.repo_id
 AND a.blob_sha = fs.blob_sha
 AND a.path = fs.path
WHERE fs.commit_sha = $1
  AND a.canonical_kind IN ('function', 'method', 'class')
ORDER BY a.path ASC, a.start_line ASC
"#,
                        &[&commit_sha],
                    )
                    .await
                    .context("failed querying production artefacts")?;

                rows.into_iter()
                    .map(|row| {
                        Ok(ProductionArtefact {
                            artefact_id: get(&row, 0, "artefact_id")?,
                            symbol_id: get(&row, 1, "symbol_id")?,
                            symbol_fqn: get(&row, 2, "symbol_fqn")?,
                            path: get(&row, 3, "path")?,
                            start_line: get_i64(&row, 4, "start_line")?,
                        })
                    })
                    .collect()
            })
        })
    }

    fn load_test_scenarios(&self, commit_sha: &str) -> Result<Vec<ResolvedTestScenarioRecord>> {
        let commit_sha = commit_sha.to_string();
        self.with_client(move |client| {
            Box::pin(async move {
                let rows = client
                    .query(
                        r#"
SELECT ts.scenario_id, ts.path, COALESCE(s.name, ''), ts.name
FROM test_scenarios ts
LEFT JOIN test_suites s ON s.suite_id = ts.suite_id
WHERE ts.commit_sha = $1
ORDER BY ts.path ASC, ts.start_line ASC
"#,
                        &[&commit_sha],
                    )
                    .await
                    .context("failed querying test scenarios")?;

                rows.into_iter()
                    .map(|row| {
                        Ok(ResolvedTestScenarioRecord {
                            scenario_id: get(&row, 0, "scenario_id")?,
                            path: get(&row, 1, "path")?,
                            suite_name: get(&row, 2, "suite_name")?,
                            test_name: get(&row, 3, "test_name")?,
                        })
                    })
                    .collect()
            })
        })
    }

    fn load_artefacts_for_file_lines(
        &self,
        commit_sha: &str,
        file_path: &str,
    ) -> Result<Vec<(String, i64, i64)>> {
        let commit_sha = commit_sha.to_string();
        let file_path = file_path.to_string();
        self.with_client(move |client| {
            Box::pin(async move {
                let rows = client
                    .query(
                        r#"
SELECT DISTINCT a.artefact_id, a.path, a.start_line, a.end_line
FROM file_state fs
JOIN artefacts a
  ON a.repo_id = fs.repo_id
 AND a.blob_sha = fs.blob_sha
 AND a.path = fs.path
WHERE fs.commit_sha = $1
  AND a.canonical_kind != 'file'
  AND (fs.path = $2 OR $2 LIKE '%' || fs.path)
ORDER BY a.path ASC, a.start_line ASC
"#,
                        &[&commit_sha, &file_path],
                    )
                    .await
                    .context("failed querying artefacts for file")?;

                rows.into_iter()
                    .map(|row| {
                        Ok((
                            get(&row, 0, "artefact_id")?,
                            get_i64(&row, 2, "start_line")?,
                            get_i64(&row, 3, "end_line")?,
                        ))
                    })
                    .collect()
            })
        })
    }

    fn replace_production_artefacts(&mut self, _batch: &ProductionIngestionBatch) -> Result<()> {
        bail!(
            "production artefact replacement is not supported in the Bitloops-backed Postgres repository; use `bitloops devql ingest`"
        )
    }

    fn replace_test_discovery(
        &mut self,
        commit_sha: &str,
        suites: &[TestSuiteRecord],
        scenarios: &[TestScenarioRecord],
        links: &[TestLinkRecord],
        discovery_run: &TestDiscoveryRunRecord,
        diagnostics: &[TestDiscoveryDiagnosticRecord],
    ) -> Result<()> {
        let commit_sha = commit_sha.to_string();
        let suites = suites.to_vec();
        let scenarios = scenarios.to_vec();
        let links = links.to_vec();
        let discovery_run = discovery_run.clone();
        let diagnostics = diagnostics.to_vec();
        self.with_client(move |client| {
            Box::pin(async move {
                let tx = client
                    .transaction()
                    .await
                    .context("failed to start test discovery transaction")?;
                clear_existing_test_discovery_data(&tx, &commit_sha).await?;

                upsert_test_discovery_run(&tx, &discovery_run).await?;
                for diagnostic in diagnostics {
                    upsert_test_discovery_diagnostic(&tx, &diagnostic).await?;
                }
                for suite in suites {
                    upsert_test_suite(&tx, &suite).await?;
                }
                for scenario in scenarios {
                    upsert_test_scenario(&tx, &scenario).await?;
                }
                for link in links {
                    upsert_test_link(&tx, &link).await?;
                }

                tx.commit()
                    .await
                    .context("failed to commit test discovery transaction")
            })
        })
    }

    fn replace_test_runs(&mut self, commit_sha: &str, runs: &[TestRunRecord]) -> Result<()> {
        let commit_sha = commit_sha.to_string();
        let runs = runs.to_vec();
        self.with_client(move |client| {
            Box::pin(async move {
                let tx = client
                    .transaction()
                    .await
                    .context("failed to start test run transaction")?;

                tx.execute(
                    "DELETE FROM test_runs WHERE commit_sha = $1",
                    &[&commit_sha],
                )
                .await
                .context("failed clearing existing test_runs for commit")?;

                for run in runs {
                    upsert_test_run(&tx, &run).await?;
                }

                tx.commit()
                    .await
                    .context("failed to commit test run transaction")
            })
        })
    }

    fn insert_coverage_capture(&mut self, capture: &CoverageCaptureRecord) -> Result<()> {
        let capture = capture.clone();
        self.with_client(move |client| {
            Box::pin(async move {
                client
                    .execute(
                        r#"
INSERT INTO coverage_captures (
  capture_id, repo_id, commit_sha, tool, format, scope_kind,
  subject_test_scenario_id, line_truth, branch_truth, captured_at, status, metadata_json
) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
ON CONFLICT(capture_id) DO UPDATE SET
  repo_id = excluded.repo_id,
  commit_sha = excluded.commit_sha,
  tool = excluded.tool,
  format = excluded.format,
  scope_kind = excluded.scope_kind,
  subject_test_scenario_id = excluded.subject_test_scenario_id,
  line_truth = excluded.line_truth,
  branch_truth = excluded.branch_truth,
  captured_at = excluded.captured_at,
  status = excluded.status,
  metadata_json = excluded.metadata_json
"#,
                        &[
                            &capture.capture_id,
                            &capture.repo_id,
                            &capture.commit_sha,
                            &capture.tool,
                            &capture.format.as_str(),
                            &capture.scope_kind.as_str(),
                            &capture.subject_test_scenario_id,
                            &(capture.line_truth as i64),
                            &(capture.branch_truth as i64),
                            &capture.captured_at,
                            &capture.status,
                            &capture.metadata_json,
                        ],
                    )
                    .await
                    .with_context(|| {
                        format!("failed inserting coverage capture {}", capture.capture_id)
                    })?;
                Ok(())
            })
        })
    }

    fn insert_coverage_hits(&mut self, hits: &[CoverageHitRecord]) -> Result<()> {
        let hits = hits.to_vec();
        self.with_client(move |client| {
            Box::pin(async move {
                let tx = client
                    .transaction()
                    .await
                    .context("failed to start coverage hits transaction")?;

                for hit in hits {
                    tx.execute(
                        r#"
INSERT INTO coverage_hits (
  capture_id, production_artefact_id, file_path, line, branch_id, covered, hit_count
) VALUES ($1, $2, $3, $4, $5, $6, $7)
ON CONFLICT(capture_id, production_artefact_id, line, branch_id) DO UPDATE SET
  file_path = excluded.file_path,
  covered = excluded.covered,
  hit_count = excluded.hit_count
"#,
                        &[
                            &hit.capture_id,
                            &hit.production_artefact_id,
                            &hit.file_path,
                            &hit.line,
                            &hit.branch_id,
                            &(hit.covered as i64),
                            &hit.hit_count,
                        ],
                    )
                    .await
                    .with_context(|| {
                        format!(
                            "failed inserting coverage hit for capture {} artefact {} line {}",
                            hit.capture_id, hit.production_artefact_id, hit.line
                        )
                    })?;
                }

                tx.commit()
                    .await
                    .context("failed to commit coverage hits transaction")
            })
        })
    }

    fn insert_coverage_diagnostics(
        &mut self,
        diagnostics: &[CoverageDiagnosticRecord],
    ) -> Result<()> {
        if diagnostics.is_empty() {
            return Ok(());
        }

        let diagnostics = diagnostics.to_vec();
        self.with_client(move |client| {
            Box::pin(async move {
                let tx = client
                    .transaction()
                    .await
                    .context("failed to start coverage diagnostics transaction")?;

                for diag in diagnostics {
                    tx.execute(
                        r#"
INSERT INTO coverage_diagnostics (
  diagnostic_id, capture_id, repo_id, commit_sha, path, line,
  severity, code, message, metadata_json
) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
ON CONFLICT(diagnostic_id) DO UPDATE SET
  capture_id = excluded.capture_id,
  severity = excluded.severity,
  code = excluded.code,
  message = excluded.message,
  metadata_json = excluded.metadata_json
"#,
                        &[
                            &diag.diagnostic_id,
                            &diag.capture_id,
                            &diag.repo_id,
                            &diag.commit_sha,
                            &diag.path,
                            &diag.line,
                            &diag.severity,
                            &diag.code,
                            &diag.message,
                            &diag.metadata_json,
                        ],
                    )
                    .await
                    .with_context(|| {
                        format!(
                            "failed inserting coverage diagnostic {}",
                            diag.diagnostic_id
                        )
                    })?;
                }

                tx.commit()
                    .await
                    .context("failed to commit coverage diagnostics transaction")
            })
        })
    }

    fn rebuild_classifications_from_coverage(&mut self, commit_sha: &str) -> Result<usize> {
        let commit_sha = commit_sha.to_string();
        self.with_client(move |client| {
            Box::pin(async move {
                client
                    .execute(
                        "DELETE FROM test_classifications WHERE commit_sha = $1",
                        &[&commit_sha],
                    )
                    .await
                    .context("failed clearing prior classifications for commit")?;

                let rows = client
                    .query(
                        r#"
SELECT cc.repo_id, cc.subject_test_scenario_id, ch.production_artefact_id, ch.file_path
FROM coverage_hits ch
JOIN coverage_captures cc ON cc.capture_id = ch.capture_id
WHERE cc.commit_sha = $1
  AND cc.scope_kind = 'test_scenario'
  AND cc.subject_test_scenario_id IS NOT NULL
  AND ch.covered = 1
"#,
                        &[&commit_sha],
                    )
                    .await
                    .context("failed querying coverage rows for classification")?;

                let mut grouped: HashMap<String, (String, HashSet<String>, HashSet<String>)> =
                    HashMap::new();
                for row in rows {
                    let repo_id: String = get(&row, 0, "repo_id")?;
                    let test_scenario_id: String = get(&row, 1, "test_scenario_id")?;
                    let artefact_id: String = get(&row, 2, "artefact_id")?;
                    let path: String = get(&row, 3, "artefact_path")?;

                    let directory = std::path::Path::new(&path)
                        .parent()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default();

                    let entry = grouped
                        .entry(test_scenario_id)
                        .or_insert_with(|| (repo_id, HashSet::new(), HashSet::new()));
                    entry.1.insert(artefact_id);
                    entry.2.insert(directory);
                }

                let mut inserted = 0usize;
                for (test_scenario_id, (repo_id, artefacts, directories)) in grouped {
                    let fan_out = artefacts.len() as i64;
                    if fan_out == 0 {
                        continue;
                    }
                    let boundary_crossings = directories.len() as i64;
                    let classification = derive_test_classification(fan_out, boundary_crossings);
                    let record = TestClassificationRecord {
                        classification_id: format!("class:{commit_sha}:{test_scenario_id}"),
                        repo_id,
                        commit_sha: commit_sha.to_string(),
                        test_scenario_id,
                        classification: classification.to_string(),
                        classification_source: "coverage_derived".to_string(),
                        fan_out,
                        boundary_crossings,
                    };
                    upsert_test_classification(client, &record).await?;
                    inserted += 1;
                }

                Ok(inserted)
            })
        })
    }
}

impl TestHarnessQueryRepository for PostgresTestHarnessRepository {
    fn find_artefact(
        &self,
        commit_sha: &str,
        artefact_query: &str,
    ) -> Result<QueriedArtefactRecord> {
        let commit_sha = commit_sha.to_string();
        let artefact_query = artefact_query.to_string();
        self.with_client(move |client| {
            Box::pin(async move {
                let row = client
                    .query_opt(
                        r#"
SELECT
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
  AND (
    a.artefact_id = $2
    OR a.symbol_fqn = $2
    OR a.path = $2
    OR a.symbol_fqn LIKE '%' || $2
  )
ORDER BY
  CASE
    WHEN a.symbol_fqn = $2 THEN 0
    WHEN a.artefact_id = $2 THEN 1
    WHEN a.path = $2 THEN 2
    ELSE 3
  END ASC,
  a.start_line ASC
LIMIT 1
"#,
                        &[&commit_sha, &artefact_query],
                    )
                    .await
                    .context("failed querying artefact")?;

                let Some(row) = row else {
                    let indexed_for_commit = client
                        .query_opt(
                            "SELECT 1 FROM commits WHERE commit_sha = $1 LIMIT 1",
                            &[&commit_sha],
                        )
                        .await
                        .context("failed checking indexed state for commit")?;

                    if indexed_for_commit.is_some() {
                        bail!("Artefact not found");
                    }

                    bail!("Repository not indexed");
                };

                Ok(QueriedArtefactRecord {
                    artefact_id: get(&row, 0, "artefact_id")?,
                    symbol_fqn: get(&row, 1, "symbol_fqn")?,
                    canonical_kind: get(&row, 2, "canonical_kind")?,
                    path: get(&row, 3, "path")?,
                    start_line: get_i64(&row, 4, "start_line")?,
                    end_line: get_i64(&row, 5, "end_line")?,
                })
            })
        })
    }

    fn list_artefacts(
        &self,
        commit_sha: &str,
        kind: Option<&str>,
    ) -> Result<Vec<crate::models::ListedArtefactRecord>> {
        let commit_sha = commit_sha.to_string();
        let kind = kind.map(str::to_string);
        self.with_client(move |client| {
            Box::pin(async move {
                let mut output = Vec::new();

                if kind.is_none()
                    || !matches!(kind.as_deref(), Some("test_suite" | "test_scenario"))
                {
                    output.extend(
                        load_listed_production_artefacts(client, &commit_sha, kind.as_deref())
                            .await?,
                    );
                }
                if kind.is_none() || matches!(kind.as_deref(), Some("test_suite")) {
                    output.extend(load_listed_test_suites(client, &commit_sha).await?);
                }
                if kind.is_none() || matches!(kind.as_deref(), Some("test_scenario")) {
                    output.extend(load_listed_test_scenarios(client, &commit_sha).await?);
                }

                output.sort_by(|left, right| {
                    left.file_path
                        .cmp(&right.file_path)
                        .then(left.start_line.cmp(&right.start_line))
                        .then(left.kind.cmp(&right.kind))
                });
                Ok(output)
            })
        })
    }

    fn load_covering_tests(
        &self,
        commit_sha: &str,
        production_artefact_id: &str,
    ) -> Result<Vec<CoveringTestRecord>> {
        let commit_sha = commit_sha.to_string();
        let production_artefact_id = production_artefact_id.to_string();
        self.with_client(move |client| {
            Box::pin(async move {
                let rows = client
                    .query(
                        r#"
SELECT
  ts.scenario_id,
  ts.symbol_fqn,
  ts.signature,
  ts.path,
  s.name AS suite_name,
  tc.classification,
  tc.classification_source,
  tc.fan_out
FROM test_links tl
JOIN test_scenarios ts
  ON ts.scenario_id = tl.test_scenario_id
LEFT JOIN test_suites s
  ON s.suite_id = ts.suite_id
LEFT JOIN test_classifications tc
  ON tc.test_scenario_id = ts.scenario_id
  AND tc.commit_sha = $1
WHERE tl.commit_sha = $1
  AND tl.production_artefact_id = $2
ORDER BY ts.path ASC, ts.start_line ASC
"#,
                        &[&commit_sha, &production_artefact_id],
                    )
                    .await
                    .context("failed executing covering tests query")?;

                rows.into_iter()
                    .map(|row| {
                        Ok(CoveringTestRecord {
                            test_id: get(&row, 0, "test_id")?,
                            test_symbol_fqn: get(&row, 1, "test_symbol_fqn")?,
                            test_signature: get(&row, 2, "test_signature")?,
                            test_path: get(&row, 3, "test_path")?,
                            suite_name: get(&row, 4, "suite_name")?,
                            classification: get(&row, 5, "classification")?,
                            classification_source: get(&row, 6, "classification_source")?,
                            fan_out: get_opt_i64(&row, 7, "fan_out")?,
                        })
                    })
                    .collect()
            })
        })
    }

    fn load_linked_fan_out_by_test(&self, commit_sha: &str) -> Result<HashMap<String, i64>> {
        let commit_sha = commit_sha.to_string();
        self.with_client(move |client| {
            Box::pin(async move {
                let rows = client
                    .query(
                        r#"
SELECT test_scenario_id, COUNT(DISTINCT production_artefact_id)
FROM test_links
WHERE commit_sha = $1
GROUP BY test_scenario_id
"#,
                        &[&commit_sha],
                    )
                    .await
                    .context("failed executing linked fan-out query")?;

                let mut output = HashMap::new();
                for row in rows {
                    output.insert(get(&row, 0, "test_scenario_id")?, get(&row, 1, "fan_out")?);
                }
                Ok(output)
            })
        })
    }

    fn coverage_exists_for_commit(&self, commit_sha: &str) -> Result<bool> {
        let commit_sha = commit_sha.to_string();
        self.with_client(move |client| {
            Box::pin(async move {
                let row = client
                    .query_one(
                        "SELECT EXISTS(SELECT 1 FROM coverage_captures WHERE commit_sha = $1)",
                        &[&commit_sha],
                    )
                    .await
                    .context("failed querying coverage existence")?;
                get(&row, 0, "exists")
            })
        })
    }

    fn load_coverage_pair_stats(
        &self,
        commit_sha: &str,
        test_scenario_id: &str,
        artefact_id: &str,
    ) -> Result<CoveragePairStats> {
        let commit_sha = commit_sha.to_string();
        let test_scenario_id = test_scenario_id.to_string();
        let artefact_id = artefact_id.to_string();
        self.with_client(move |client| {
            Box::pin(async move {
                let row = client
                    .query_one(
                        r#"
SELECT
  COUNT(*) AS total_rows,
  COALESCE(SUM(CASE WHEN ch.covered = 1 THEN 1 ELSE 0 END), 0) AS covered_rows
FROM coverage_hits ch
JOIN coverage_captures cc ON cc.capture_id = ch.capture_id
WHERE cc.commit_sha = $1
  AND cc.scope_kind = 'test_scenario'
  AND cc.subject_test_scenario_id = $2
  AND ch.production_artefact_id = $3
"#,
                        &[&commit_sha, &test_scenario_id, &artefact_id],
                    )
                    .await
                    .context("failed querying pair coverage stats")?;

                Ok(CoveragePairStats {
                    total_rows: get(&row, 0, "total_rows")?,
                    covered_rows: get(&row, 1, "covered_rows")?,
                })
            })
        })
    }

    fn load_latest_test_run(
        &self,
        commit_sha: &str,
        test_scenario_id: &str,
    ) -> Result<Option<LatestTestRunRecord>> {
        let commit_sha = commit_sha.to_string();
        let test_scenario_id = test_scenario_id.to_string();
        self.with_client(move |client| {
            Box::pin(async move {
                let row = client
                    .query_opt(
                        r#"
SELECT status, duration_ms, commit_sha
FROM test_runs
WHERE test_scenario_id = $1
  AND commit_sha = $2
ORDER BY ran_at DESC
LIMIT 1
"#,
                        &[&test_scenario_id, &commit_sha],
                    )
                    .await
                    .context("failed querying last run")?;

                row.map(|row| {
                    Ok(LatestTestRunRecord {
                        status: get(&row, 0, "run_status")?,
                        duration_ms: get_opt_i64(&row, 1, "duration_ms")?,
                        commit_sha: get(&row, 2, "commit_sha")?,
                    })
                })
                .transpose()
            })
        })
    }

    fn load_coverage_summary(
        &self,
        commit_sha: &str,
        artefact_id: &str,
    ) -> Result<Option<CoverageSummaryRecord>> {
        let commit_sha = commit_sha.to_string();
        let artefact_id = artefact_id.to_string();
        self.with_client(move |client| {
            Box::pin(async move {
                let line_rows = client
                    .query(
                        r#"
SELECT ch.line, MAX(CASE WHEN ch.covered = 1 THEN 1 ELSE 0 END) AS covered_any
FROM coverage_hits ch
JOIN coverage_captures cc ON cc.capture_id = ch.capture_id
WHERE cc.commit_sha = $1
  AND ch.production_artefact_id = $2
  AND ch.branch_id = -1
GROUP BY ch.line
ORDER BY ch.line
"#,
                        &[&commit_sha, &artefact_id],
                    )
                    .await
                    .context("failed querying line coverage summary")?;

                let mut line_total = 0usize;
                let mut line_covered = 0usize;
                for row in line_rows {
                    let covered = get_i64(&row, 1, "covered_any")?;
                    line_total += 1;
                    if covered == 1 {
                        line_covered += 1;
                    }
                }

                let branch_rows = client
                    .query(
                        r#"
SELECT
  ch.line,
  ch.branch_id,
  MAX(CASE WHEN ch.covered = 1 THEN 1 ELSE 0 END) AS covered_any
FROM coverage_hits ch
JOIN coverage_captures cc ON cc.capture_id = ch.capture_id
WHERE cc.commit_sha = $1
  AND ch.production_artefact_id = $2
  AND ch.branch_id != -1
GROUP BY ch.line, ch.branch_id
ORDER BY ch.line, ch.branch_id
"#,
                        &[&commit_sha, &artefact_id],
                    )
                    .await
                    .context("failed querying branch coverage summary")?;

                let mut branch_total = 0usize;
                let mut branch_covered = 0usize;
                let mut branches = Vec::new();
                for row in branch_rows {
                    let covered = get_i64(&row, 2, "covered_any")?;
                    let branch = CoverageBranchRecord {
                        line: get_i64(&row, 0, "line")?,
                        branch_id: get_i64(&row, 1, "branch_id")?,
                        covered: covered == 1,
                        covering_test_ids: vec![],
                    };
                    branch_total += 1;
                    if branch.covered {
                        branch_covered += 1;
                    }
                    branches.push(branch);
                }

                if line_total == 0 && branch_total == 0 {
                    return Ok(None);
                }

                Ok(Some(CoverageSummaryRecord {
                    line_total,
                    line_covered,
                    branch_total,
                    branch_covered,
                    branches,
                }))
            })
        })
    }

    fn load_test_harness_commit_counts(&self, commit_sha: &str) -> Result<TestHarnessCommitCounts> {
        let commit_sha = commit_sha.to_string();
        self.with_client(move |client| {
            Box::pin(async move {
                commit_counts::load_test_harness_commit_counts(client, &commit_sha).await
            })
        })
    }

    fn load_stage_covering_tests(
        &self,
        repo_id: &str,
        production_artefact_id: &str,
        min_confidence: Option<f64>,
        linkage_source: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StageCoveringTestRecord>> {
        let repo_id = repo_id.to_string();
        let production_artefact_id = production_artefact_id.to_string();
        let linkage_source_owned = linkage_source.map(str::to_string);
        self.with_client(move |client| {
            Box::pin(async move {
                stage_serving::load_stage_covering_tests(
                    client,
                    repo_id,
                    production_artefact_id,
                    linkage_source_owned,
                    min_confidence,
                    limit,
                )
                .await
            })
        })
    }

    fn load_stage_line_coverage(
        &self,
        repo_id: &str,
        artefact_id: &str,
        commit_sha: Option<&str>,
    ) -> Result<Vec<StageLineCoverageRecord>> {
        let repo_id = repo_id.to_string();
        let artefact_id = artefact_id.to_string();
        let commit_sha = commit_sha.map(str::to_string);
        self.with_client(move |client| {
            Box::pin(async move {
                stage_serving::load_stage_line_coverage(client, repo_id, artefact_id, commit_sha)
                    .await
            })
        })
    }

    fn load_stage_branch_coverage(
        &self,
        repo_id: &str,
        artefact_id: &str,
        commit_sha: Option<&str>,
    ) -> Result<Vec<StageBranchCoverageRecord>> {
        let repo_id = repo_id.to_string();
        let artefact_id = artefact_id.to_string();
        let commit_sha = commit_sha.map(str::to_string);
        self.with_client(move |client| {
            Box::pin(async move {
                stage_serving::load_stage_branch_coverage(
                    client,
                    repo_id,
                    artefact_id,
                    commit_sha,
                )
                .await
            })
        })
    }

    fn load_stage_coverage_metadata(
        &self,
        repo_id: &str,
        commit_sha: Option<&str>,
    ) -> Result<Option<StageCoverageMetadataRecord>> {
        let repo_id = repo_id.to_string();
        let commit_sha = commit_sha.map(str::to_string);
        self.with_client(move |client| {
            Box::pin(async move {
                stage_serving::load_stage_coverage_metadata(client, repo_id, commit_sha).await
            })
        })
    }
}
