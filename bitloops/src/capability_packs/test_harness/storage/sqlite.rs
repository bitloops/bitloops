// SQLite repository implementation for command-side persistence. This module
// owns SQL statements, transactions, and row mapping for write workflows.

mod stage_serving;
#[cfg(test)]
mod tests;
mod writes;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::capability_packs::test_harness::storage::{
    TestHarnessQueryRepository, TestHarnessRepository,
};
use crate::models::{
    CoverageBranchRecord, CoverageCaptureRecord, CoverageDiagnosticRecord, CoverageHitRecord,
    CoveragePairStats, CoverageSummaryRecord, CoveringTestRecord, LatestTestRunRecord,
    ResolvedTestScenarioRecord, StageBranchCoverageRecord, StageCoverageMetadataRecord,
    StageCoveringTestRecord, StageLineCoverageRecord, TestArtefactCurrentRecord,
    TestArtefactEdgeCurrentRecord, TestClassificationRecord, TestDiscoveryDiagnosticRecord,
    TestDiscoveryRunRecord, TestHarnessCommitCounts, TestRunRecord, derive_test_classification,
};
use crate::storage::init::open_existing_database;

use self::stage_serving::{
    load_stage_branch_coverage as load_stage_branch_coverage_conn,
    load_stage_coverage_metadata as load_stage_coverage_metadata_conn,
    load_stage_covering_tests as load_stage_covering_tests_conn,
    load_stage_line_coverage as load_stage_line_coverage_conn,
};
use self::writes::{
    clear_existing_production_data, clear_existing_test_discovery_data, table_exists,
    upsert_commit, upsert_current_file_state, upsert_current_production_artefact,
    upsert_current_production_edge, upsert_file_state, upsert_production_artefact,
    upsert_production_edge, upsert_repository, upsert_test_artefact_current,
    upsert_test_artefact_edge_current, upsert_test_classification,
    upsert_test_discovery_diagnostic, upsert_test_discovery_run, upsert_test_run,
};

pub struct SqliteTestHarnessRepository {
    conn: Connection,
}

impl SqliteTestHarnessRepository {
    pub fn open_existing(db_path: &Path) -> Result<Self> {
        Ok(Self {
            conn: open_existing_database(db_path)?,
        })
    }
}

impl TestHarnessRepository for SqliteTestHarnessRepository {
    fn load_test_scenarios(&self, commit_sha: &str) -> Result<Vec<ResolvedTestScenarioRecord>> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
SELECT ts.symbol_id, ts.path, COALESCE(parent.name, ''), ts.name
FROM test_artefacts_current ts
LEFT JOIN test_artefacts_current parent
  ON parent.repo_id = ts.repo_id
 AND parent.symbol_id = ts.parent_symbol_id
WHERE ts.commit_sha = ?1
  AND ts.canonical_kind = 'test_scenario'
ORDER BY ts.path ASC, ts.start_line ASC
"#,
            )
            .context("failed preparing scenario lookup query")?;

        let rows = stmt
            .query_map(params![commit_sha], |row| {
                Ok(ResolvedTestScenarioRecord {
                    scenario_id: row.get(0)?,
                    path: row.get(1)?,
                    suite_name: row.get(2)?,
                    test_name: row.get(3)?,
                })
            })
            .context("failed querying test scenarios")?;

        let mut scenarios = Vec::new();
        for row in rows {
            scenarios.push(row.context("failed decoding test scenario row")?);
        }
        Ok(scenarios)
    }

    fn replace_production_artefacts(
        &mut self,
        batch: &crate::models::ProductionIngestionBatch,
    ) -> Result<()> {
        let tx = self
            .conn
            .transaction()
            .context("failed to start production artefact transaction")?;
        let has_current_file_state = table_exists(&tx, "current_file_state")?;
        clear_existing_production_data(&tx, &batch.commit.commit_sha)?;

        upsert_repository(&tx, &batch.repository)?;
        upsert_commit(&tx, &batch.commit)?;
        for row in &batch.file_states {
            upsert_file_state(&tx, row)?;
        }
        if has_current_file_state {
            for row in &batch.current_file_states {
                upsert_current_file_state(&tx, row)?;
            }
        }
        for artefact in &batch.artefacts {
            upsert_production_artefact(&tx, artefact)?;
        }
        for artefact in &batch.current_artefacts {
            upsert_current_production_artefact(&tx, artefact)?;
        }
        for edge in &batch.edges {
            upsert_production_edge(&tx, edge)?;
        }
        for edge in &batch.current_edges {
            upsert_current_production_edge(&tx, edge)?;
        }

        tx.commit()
            .context("failed to commit production artefact transaction")?;
        Ok(())
    }

    fn replace_test_discovery(
        &mut self,
        commit_sha: &str,
        test_artefacts: &[TestArtefactCurrentRecord],
        test_edges: &[TestArtefactEdgeCurrentRecord],
        discovery_run: &TestDiscoveryRunRecord,
        diagnostics: &[TestDiscoveryDiagnosticRecord],
    ) -> Result<()> {
        let tx = self
            .conn
            .transaction()
            .context("failed to start test discovery transaction")?;
        clear_existing_test_discovery_data(&tx, commit_sha)?;

        upsert_test_discovery_run(&tx, discovery_run)?;
        for diagnostic in diagnostics {
            upsert_test_discovery_diagnostic(&tx, diagnostic)?;
        }
        for artefact in test_artefacts {
            upsert_test_artefact_current(&tx, artefact)?;
        }
        for edge in test_edges {
            upsert_test_artefact_edge_current(&tx, edge)?;
        }

        tx.commit()
            .context("failed to commit test discovery transaction")?;
        Ok(())
    }

    fn replace_test_runs(&mut self, commit_sha: &str, runs: &[TestRunRecord]) -> Result<()> {
        let tx = self
            .conn
            .transaction()
            .context("failed to start test run transaction")?;

        tx.execute(
            "DELETE FROM test_runs WHERE commit_sha = ?1",
            params![commit_sha],
        )
        .context("failed clearing existing test_runs for commit")?;

        for run in runs {
            upsert_test_run(&tx, run)?;
        }

        tx.commit()
            .context("failed to commit test run transaction")?;
        Ok(())
    }

    fn insert_coverage_capture(&mut self, capture: &CoverageCaptureRecord) -> Result<()> {
        self.conn
            .execute(
                r#"
INSERT INTO coverage_captures (
  capture_id, repo_id, commit_sha, tool, format, scope_kind,
  subject_test_symbol_id, line_truth, branch_truth, captured_at, status, metadata_json
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
ON CONFLICT(capture_id) DO UPDATE SET
  repo_id = excluded.repo_id,
  commit_sha = excluded.commit_sha,
  tool = excluded.tool,
  format = excluded.format,
  scope_kind = excluded.scope_kind,
  subject_test_symbol_id = excluded.subject_test_symbol_id,
  line_truth = excluded.line_truth,
  branch_truth = excluded.branch_truth,
  captured_at = excluded.captured_at,
  status = excluded.status,
  metadata_json = excluded.metadata_json
"#,
                params![
                    capture.capture_id,
                    capture.repo_id,
                    capture.commit_sha,
                    capture.tool,
                    capture.format.as_str(),
                    capture.scope_kind.as_str(),
                    capture.subject_test_symbol_id,
                    capture.line_truth as i64,
                    capture.branch_truth as i64,
                    capture.captured_at,
                    capture.status,
                    capture.metadata_json,
                ],
            )
            .with_context(|| format!("failed inserting coverage capture {}", capture.capture_id))?;
        Ok(())
    }

    fn insert_coverage_hits(&mut self, hits: &[CoverageHitRecord]) -> Result<()> {
        let tx = self
            .conn
            .transaction()
            .context("failed to start coverage hits transaction")?;

        for hit in hits {
            tx.execute(
                r#"
INSERT INTO coverage_hits (
  capture_id, production_symbol_id, file_path, line, branch_id, covered, hit_count
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
ON CONFLICT(capture_id, production_symbol_id, line, branch_id) DO UPDATE SET
  file_path = excluded.file_path,
  covered = excluded.covered,
  hit_count = excluded.hit_count
"#,
                params![
                    hit.capture_id,
                    hit.production_symbol_id,
                    hit.file_path,
                    hit.line,
                    hit.branch_id,
                    hit.covered as i64,
                    hit.hit_count,
                ],
            )
            .with_context(|| {
                format!(
                    "failed inserting coverage hit for capture {} symbol {} line {}",
                    hit.capture_id, hit.production_symbol_id, hit.line
                )
            })?;
        }

        tx.commit()
            .context("failed to commit coverage hits transaction")?;
        Ok(())
    }

    fn insert_coverage_diagnostics(
        &mut self,
        diagnostics: &[CoverageDiagnosticRecord],
    ) -> Result<()> {
        if diagnostics.is_empty() {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction()
            .context("failed to start coverage diagnostics transaction")?;

        for diag in diagnostics {
            tx.execute(
                r#"
INSERT INTO coverage_diagnostics (
  diagnostic_id, capture_id, repo_id, commit_sha, path, line,
  severity, code, message, metadata_json
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
ON CONFLICT(diagnostic_id) DO UPDATE SET
  capture_id = excluded.capture_id,
  severity = excluded.severity,
  code = excluded.code,
  message = excluded.message,
  metadata_json = excluded.metadata_json
"#,
                params![
                    diag.diagnostic_id,
                    diag.capture_id,
                    diag.repo_id,
                    diag.commit_sha,
                    diag.path,
                    diag.line,
                    diag.severity,
                    diag.code,
                    diag.message,
                    diag.metadata_json,
                ],
            )
            .with_context(|| {
                format!(
                    "failed inserting coverage diagnostic {}",
                    diag.diagnostic_id
                )
            })?;
        }

        tx.commit()
            .context("failed to commit coverage diagnostics transaction")?;
        Ok(())
    }

    fn rebuild_classifications_from_coverage(&mut self, commit_sha: &str) -> Result<usize> {
        self.conn
            .execute(
                "DELETE FROM test_classifications WHERE commit_sha = ?1",
                params![commit_sha],
            )
            .context("failed clearing prior classifications for commit")?;

        let mut stmt = self
            .conn
            .prepare(
                r#"
SELECT cc.repo_id, cc.subject_test_symbol_id, ch.production_symbol_id, ch.file_path
FROM coverage_hits ch
JOIN coverage_captures cc ON cc.capture_id = ch.capture_id
WHERE cc.commit_sha = ?1
  AND cc.scope_kind = 'test_scenario'
  AND cc.subject_test_symbol_id IS NOT NULL
  AND ch.covered = 1
"#,
            )
            .context("failed preparing classification source query")?;

        let mut rows = stmt
            .query(params![commit_sha])
            .context("failed querying coverage rows for classification")?;

        let mut grouped: HashMap<String, (String, HashSet<String>, HashSet<String>)> =
            HashMap::new();
        while let Some(row) = rows
            .next()
            .context("failed reading classification source row")?
        {
            let repo_id: String = row.get(0).context("missing repo_id")?;
            let test_symbol_id: String = row.get(1).context("missing test_symbol_id")?;
            let production_symbol_id: String =
                row.get(2).context("missing production_symbol_id")?;
            let path: String = row.get(3).context("missing artefact path")?;

            let directory = Path::new(&path)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();

            let entry = grouped
                .entry(test_symbol_id)
                .or_insert_with(|| (repo_id, HashSet::new(), HashSet::new()));
            entry.1.insert(production_symbol_id);
            entry.2.insert(directory);
        }

        let mut inserted = 0usize;
        for (test_symbol_id, (repo_id, artefacts, directories)) in grouped {
            let fan_out = artefacts.len() as i64;
            if fan_out == 0 {
                continue;
            }
            let boundary_crossings = directories.len() as i64;
            let classification = derive_test_classification(fan_out, boundary_crossings);
            let record = TestClassificationRecord {
                classification_id: format!("class:{commit_sha}:{test_symbol_id}"),
                repo_id,
                commit_sha: commit_sha.to_string(),
                test_symbol_id,
                classification: classification.to_string(),
                classification_source: "coverage_derived".to_string(),
                fan_out,
                boundary_crossings,
            };
            upsert_test_classification(&self.conn, &record)?;
            inserted += 1;
        }

        Ok(inserted)
    }
}

impl TestHarnessQueryRepository for SqliteTestHarnessRepository {
    fn load_covering_tests(
        &self,
        commit_sha: &str,
        production_symbol_id: &str,
    ) -> Result<Vec<CoveringTestRecord>> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
SELECT DISTINCT
  ts.symbol_id,
  ts.symbol_fqn,
  ts.signature,
  ts.path,
  parent.name AS suite_name,
  tc.classification,
  tc.classification_source,
  tc.fan_out
FROM test_artefact_edges_current te
JOIN test_artefacts_current ts
  ON ts.repo_id = te.repo_id
 AND ts.symbol_id = te.from_symbol_id
LEFT JOIN test_artefacts_current parent
  ON parent.repo_id = ts.repo_id
 AND parent.symbol_id = ts.parent_symbol_id
LEFT JOIN test_classifications tc
  ON tc.test_symbol_id = ts.symbol_id
  AND tc.commit_sha = ?1
WHERE te.commit_sha = ?1
  AND (te.to_symbol_id = ?2 OR te.to_artefact_id = ?2)
ORDER BY ts.path ASC, ts.start_line ASC
"#,
            )
            .context("failed preparing covering tests query")?;

        let rows = stmt
            .query_map(params![commit_sha, production_symbol_id], |row| {
                Ok(CoveringTestRecord {
                    test_id: row.get(0)?,
                    test_symbol_fqn: row.get(1)?,
                    test_signature: row.get(2)?,
                    test_path: row.get(3)?,
                    suite_name: row.get(4)?,
                    classification: row.get(5)?,
                    classification_source: row.get(6)?,
                    fan_out: row.get(7)?,
                })
            })
            .context("failed executing covering tests query")?;

        let mut tests = Vec::new();
        for row in rows {
            tests.push(row.context("failed decoding covering test row")?);
        }
        Ok(tests)
    }

    fn load_linked_fan_out_by_test(&self, commit_sha: &str) -> Result<HashMap<String, i64>> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
SELECT from_symbol_id, COUNT(DISTINCT COALESCE(to_symbol_id, to_symbol_ref))
FROM test_artefact_edges_current
WHERE commit_sha = ?1
GROUP BY from_symbol_id
"#,
            )
            .context("failed preparing linked fan-out query")?;

        let mut rows = stmt
            .query(params![commit_sha])
            .context("failed executing linked fan-out query")?;

        let mut output = HashMap::new();
        while let Some(row) = rows.next().context("failed reading linked fan-out row")? {
            let test_symbol_id: String = row.get(0).context("missing test_symbol_id")?;
            let fan_out: i64 = row.get(1).context("missing fan_out")?;
            output.insert(test_symbol_id, fan_out);
        }
        Ok(output)
    }

    fn coverage_exists_for_commit(&self, commit_sha: &str) -> Result<bool> {
        let mut stmt = self
            .conn
            .prepare("SELECT EXISTS(SELECT 1 FROM coverage_captures WHERE commit_sha = ?1)")
            .context("failed preparing coverage existence query")?;
        let exists: i64 = stmt
            .query_row(params![commit_sha], |row| row.get(0))
            .context("failed querying coverage existence")?;
        Ok(exists == 1)
    }

    fn load_coverage_pair_stats(
        &self,
        commit_sha: &str,
        test_symbol_id: &str,
        production_symbol_id: &str,
    ) -> Result<CoveragePairStats> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
SELECT
  COUNT(*) AS total_rows,
  COALESCE(SUM(CASE WHEN ch.covered = 1 THEN 1 ELSE 0 END), 0) AS covered_rows
FROM coverage_hits ch
JOIN coverage_captures cc ON cc.capture_id = ch.capture_id
WHERE cc.commit_sha = ?1
  AND cc.scope_kind = 'test_scenario'
  AND cc.subject_test_symbol_id = ?2
  AND ch.production_symbol_id = ?3
"#,
            )
            .context("failed preparing pair coverage query")?;

        let stats = stmt
            .query_row(
                params![commit_sha, test_symbol_id, production_symbol_id],
                |row| {
                    Ok(CoveragePairStats {
                        total_rows: row.get(0)?,
                        covered_rows: row.get(1)?,
                    })
                },
            )
            .context("failed querying pair coverage stats")?;
        Ok(stats)
    }

    fn load_latest_test_run(
        &self,
        commit_sha: &str,
        test_symbol_id: &str,
    ) -> Result<Option<LatestTestRunRecord>> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
SELECT status, duration_ms, commit_sha
FROM test_runs
WHERE test_symbol_id = ?1
  AND commit_sha = ?2
ORDER BY ran_at DESC
LIMIT 1
"#,
            )
            .context("failed preparing last run query")?;

        let mut rows = stmt
            .query(params![test_symbol_id, commit_sha])
            .context("failed querying last run")?;

        let Some(row) = rows.next().context("failed reading last run row")? else {
            return Ok(None);
        };

        Ok(Some(LatestTestRunRecord {
            status: row.get(0).context("missing run status")?,
            duration_ms: row.get(1).context("missing run duration")?,
            commit_sha: row.get(2).context("missing run commit")?,
        }))
    }

    fn load_coverage_summary(
        &self,
        commit_sha: &str,
        production_symbol_id: &str,
    ) -> Result<Option<CoverageSummaryRecord>> {
        let mut line_stmt = self
            .conn
            .prepare(
                r#"
SELECT ch.line, MAX(CASE WHEN ch.covered = 1 THEN 1 ELSE 0 END) AS covered_any
FROM coverage_hits ch
JOIN coverage_captures cc ON cc.capture_id = ch.capture_id
WHERE cc.commit_sha = ?1
  AND ch.production_symbol_id = ?2
  AND ch.branch_id = -1
GROUP BY ch.line
ORDER BY ch.line
"#,
            )
            .context("failed preparing line coverage summary query")?;

        let line_rows = line_stmt
            .query_map(params![commit_sha, production_symbol_id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)? == 1))
            })
            .context("failed querying line coverage summary")?;

        let mut line_total = 0usize;
        let mut line_covered = 0usize;
        for row in line_rows {
            let (_line, covered) = row.context("failed decoding line coverage row")?;
            line_total += 1;
            if covered {
                line_covered += 1;
            }
        }

        let mut branch_stmt = self
            .conn
            .prepare(
                r#"
SELECT
  ch.line,
  ch.branch_id,
  MAX(CASE WHEN ch.covered = 1 THEN 1 ELSE 0 END) AS covered_any
FROM coverage_hits ch
JOIN coverage_captures cc ON cc.capture_id = ch.capture_id
WHERE cc.commit_sha = ?1
  AND ch.production_symbol_id = ?2
  AND ch.branch_id != -1
GROUP BY ch.line, ch.branch_id
ORDER BY ch.line, ch.branch_id
"#,
            )
            .context("failed preparing branch coverage summary query")?;

        let branch_rows = branch_stmt
            .query_map(params![commit_sha, production_symbol_id], |row| {
                Ok(CoverageBranchRecord {
                    line: row.get(0)?,
                    branch_id: row.get(1)?,
                    covered: row.get::<_, i64>(2)? == 1,
                    covering_test_ids: vec![],
                })
            })
            .context("failed querying branch coverage summary")?;

        let mut branch_total = 0usize;
        let mut branch_covered = 0usize;
        let mut branches = Vec::new();
        for row in branch_rows {
            let branch = row.context("failed decoding branch coverage row")?;
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
    }

    fn load_test_harness_commit_counts(&self, commit_sha: &str) -> Result<TestHarnessCommitCounts> {
        fn count(conn: &Connection, sql: &str, commit_sha: &str) -> Result<u64> {
            let n: i64 = conn
                .query_row(sql, params![commit_sha], |row| row.get(0))
                .context("test harness commit count query")?;
            Ok(n.max(0) as u64)
        }

        let conn = &self.conn;
        Ok(TestHarnessCommitCounts {
            test_artefacts: count(
                conn,
                "SELECT COUNT(*) FROM test_artefacts_current WHERE commit_sha = ?1",
                commit_sha,
            )?,
            test_artefact_edges: count(
                conn,
                "SELECT COUNT(*) FROM test_artefact_edges_current WHERE commit_sha = ?1",
                commit_sha,
            )?,
            test_classifications: count(
                conn,
                "SELECT COUNT(*) FROM test_classifications WHERE commit_sha = ?1",
                commit_sha,
            )?,
            coverage_captures: count(
                conn,
                "SELECT COUNT(*) FROM coverage_captures WHERE commit_sha = ?1",
                commit_sha,
            )?,
            coverage_hits: count(
                conn,
                r#"
SELECT COUNT(*) FROM coverage_hits ch
JOIN coverage_captures cc ON cc.capture_id = ch.capture_id
WHERE cc.commit_sha = ?1
"#,
                commit_sha,
            )?,
        })
    }

    fn load_stage_covering_tests(
        &self,
        repo_id: &str,
        production_symbol_id: &str,
        commit_sha: Option<&str>,
        min_confidence: Option<f64>,
        linkage_source: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StageCoveringTestRecord>> {
        load_stage_covering_tests_conn(
            &self.conn,
            repo_id,
            production_symbol_id,
            commit_sha,
            min_confidence,
            linkage_source,
            limit,
        )
    }

    fn load_stage_line_coverage(
        &self,
        repo_id: &str,
        production_symbol_id: &str,
        commit_sha: Option<&str>,
    ) -> Result<Vec<StageLineCoverageRecord>> {
        load_stage_line_coverage_conn(&self.conn, repo_id, production_symbol_id, commit_sha)
    }

    fn load_stage_branch_coverage(
        &self,
        repo_id: &str,
        production_symbol_id: &str,
        commit_sha: Option<&str>,
    ) -> Result<Vec<StageBranchCoverageRecord>> {
        load_stage_branch_coverage_conn(&self.conn, repo_id, production_symbol_id, commit_sha)
    }

    fn load_stage_coverage_metadata(
        &self,
        repo_id: &str,
        commit_sha: Option<&str>,
    ) -> Result<Option<StageCoverageMetadataRecord>> {
        load_stage_coverage_metadata_conn(&self.conn, repo_id, commit_sha)
    }
}
