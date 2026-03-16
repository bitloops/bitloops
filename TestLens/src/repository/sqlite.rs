// SQLite repository implementation for command-side persistence. This module
// owns SQL statements, transactions, and row mapping for write workflows.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

use crate::db::open_existing_database;
use crate::domain::{
    ArtefactRecord, CoverageBranchRecord, CoveragePairStats, CoverageSummaryRecord, CoverageTarget,
    CoveringTestRecord, LatestTestRunRecord, ListedArtefactRecord, ProductionArtefact,
    QueriedArtefactRecord, TestCoverageRecord, TestLinkRecord, TestRunRecord, TestScenarioRecord,
    derive_test_classification,
};
use crate::repository::{TestHarnessQueryRepository, TestHarnessRepository};

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
    fn load_repo_id_for_commit(&self, commit_sha: &str) -> Result<String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT repo_id FROM artefacts WHERE commit_sha = ?1 AND canonical_kind NOT IN ('test_suite', 'test_scenario') LIMIT 1",
            )
            .context("failed preparing repo lookup query")?;
        let repo_id: String = stmt
            .query_row(params![commit_sha], |row| row.get(0))
            .with_context(|| {
                format!(
                    "no production artefacts found for commit {}; run `testlens ingest-production-artefacts` first",
                    commit_sha
                )
            })?;
        Ok(repo_id)
    }

    fn load_production_artefacts(&self, commit_sha: &str) -> Result<Vec<ProductionArtefact>> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
SELECT artefact_id, symbol_fqn, path, start_line
FROM artefacts
WHERE commit_sha = ?1
  AND canonical_kind IN ('function', 'method', 'class')
"#,
            )
            .context("failed preparing production artefact query")?;

        let rows = stmt
            .query_map(params![commit_sha], |row| {
                Ok(ProductionArtefact {
                    artefact_id: row.get(0)?,
                    symbol_fqn: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    path: row.get(2)?,
                    start_line: row.get(3)?,
                })
            })
            .context("failed querying production artefacts")?;

        let mut artefacts = Vec::new();
        for row in rows {
            artefacts.push(row.context("failed decoding production artefact row")?);
        }
        Ok(artefacts)
    }

    fn load_test_scenarios(&self, commit_sha: &str) -> Result<Vec<TestScenarioRecord>> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
SELECT t.artefact_id, t.path, COALESCE(s.symbol_fqn, ''), COALESCE(t.signature, '')
FROM artefacts t
LEFT JOIN artefacts s
  ON s.artefact_id = t.parent_artefact_id
WHERE t.commit_sha = ?1
  AND t.canonical_kind = 'test_scenario'
"#,
            )
            .context("failed preparing scenario lookup query")?;

        let rows = stmt
            .query_map(params![commit_sha], |row| {
                Ok(TestScenarioRecord {
                    artefact_id: row.get(0)?,
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

    fn load_test_links_by_production_artefact(
        &self,
        commit_sha: &str,
    ) -> Result<HashMap<String, Vec<String>>> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
SELECT production_artefact_id, test_artefact_id
FROM test_links
WHERE commit_sha = ?1
"#,
            )
            .context("failed preparing test link query")?;

        let mut rows = stmt
            .query(params![commit_sha])
            .context("failed querying test links")?;

        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        while let Some(row) = rows.next().context("failed reading test link row")? {
            let production_artefact_id: String =
                row.get(0).context("missing production_artefact_id")?;
            let test_artefact_id: String = row.get(1).context("missing test_artefact_id")?;
            map.entry(production_artefact_id)
                .or_default()
                .push(test_artefact_id);
        }

        Ok(map)
    }

    fn load_coverage_targets_for_file(
        &self,
        commit_sha: &str,
        lcov_source_file: &str,
    ) -> Result<Vec<CoverageTarget>> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
SELECT DISTINCT a.artefact_id, a.repo_id, a.start_line, a.end_line
FROM artefacts a
JOIN test_links tl
  ON tl.production_artefact_id = a.artefact_id
WHERE a.commit_sha = ?1
  AND tl.commit_sha = ?1
  AND a.canonical_kind NOT IN ('test_suite', 'test_scenario', 'file')
  AND (a.path = ?2 OR ?2 LIKE '%' || a.path)
"#,
            )
            .context("failed preparing coverage target query")?;

        let rows = stmt
            .query_map(params![commit_sha, lcov_source_file], |row| {
                Ok(CoverageTarget {
                    artefact_id: row.get(0)?,
                    repo_id: row.get(1)?,
                    start_line: row.get(2)?,
                    end_line: row.get(3)?,
                })
            })
            .context("failed querying coverage targets")?;

        let mut targets = Vec::new();
        for row in rows {
            targets.push(row.context("failed mapping coverage target row")?);
        }
        Ok(targets)
    }

    fn replace_production_artefacts(
        &mut self,
        commit_sha: &str,
        artefacts: &[ArtefactRecord],
    ) -> Result<()> {
        let tx = self
            .conn
            .transaction()
            .context("failed to start production artefact transaction")?;
        clear_existing_production_data(&tx, commit_sha)?;

        for artefact in artefacts {
            upsert_artefact(&tx, artefact)?;
        }

        tx.commit()
            .context("failed to commit production artefact transaction")?;
        Ok(())
    }

    fn replace_test_discovery(
        &mut self,
        commit_sha: &str,
        artefacts: &[ArtefactRecord],
        links: &[TestLinkRecord],
    ) -> Result<()> {
        let tx = self
            .conn
            .transaction()
            .context("failed to start test discovery transaction")?;
        clear_existing_test_discovery_data(&tx, commit_sha)?;

        for artefact in artefacts {
            upsert_artefact(&tx, artefact)?;
        }
        for link in links {
            upsert_test_link(&tx, link)?;
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

    fn replace_test_coverage(
        &mut self,
        commit_sha: &str,
        coverage_rows: &[TestCoverageRecord],
    ) -> Result<()> {
        let tx = self
            .conn
            .transaction()
            .context("failed to start test coverage transaction")?;

        tx.execute(
            "DELETE FROM test_coverage WHERE commit_sha = ?1",
            params![commit_sha],
        )
        .context("failed to clear existing coverage rows for commit")?;

        for row in coverage_rows {
            upsert_test_coverage(&tx, row)?;
        }

        tx.commit()
            .context("failed to commit test coverage transaction")?;
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
SELECT tc.test_artefact_id, tc.artefact_id, a.path
FROM test_coverage tc
JOIN artefacts a ON a.artefact_id = tc.artefact_id
WHERE tc.commit_sha = ?1
  AND tc.covered = 1
"#,
            )
            .context("failed preparing classification source query")?;

        let mut rows = stmt
            .query(params![commit_sha])
            .context("failed querying coverage rows for classification")?;

        let mut grouped: HashMap<String, (HashSet<String>, HashSet<String>)> = HashMap::new();
        while let Some(row) = rows
            .next()
            .context("failed reading classification source row")?
        {
            let test_artefact_id: String = row.get(0).context("missing test_artefact_id")?;
            let artefact_id: String = row.get(1).context("missing artefact_id")?;
            let path: String = row.get(2).context("missing artefact path")?;

            let directory = Path::new(&path)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();

            let entry = grouped
                .entry(test_artefact_id)
                .or_insert_with(|| (HashSet::new(), HashSet::new()));
            entry.0.insert(artefact_id);
            entry.1.insert(directory);
        }

        let mut inserted = 0usize;
        for (test_artefact_id, (artefacts, directories)) in grouped {
            let fan_out = artefacts.len() as i64;
            if fan_out == 0 {
                continue;
            }
            let boundary_crossings = directories.len() as i64;
            let classification = derive_test_classification(fan_out, boundary_crossings);
            let classification_id = format!("class:{commit_sha}:{test_artefact_id}");

            self.conn
                .execute(
                    r#"
INSERT INTO test_classifications (
  classification_id,
  test_artefact_id,
  commit_sha,
  classification,
  classification_source,
  fan_out,
  boundary_crossings
) VALUES (?1, ?2, ?3, ?4, 'coverage_derived', ?5, ?6)
ON CONFLICT(classification_id) DO UPDATE SET
  test_artefact_id = excluded.test_artefact_id,
  commit_sha = excluded.commit_sha,
  classification = excluded.classification,
  classification_source = excluded.classification_source,
  fan_out = excluded.fan_out,
  boundary_crossings = excluded.boundary_crossings;
"#,
                    params![
                        classification_id,
                        test_artefact_id,
                        commit_sha,
                        classification,
                        fan_out,
                        boundary_crossings
                    ],
                )
                .with_context(|| {
                    format!("failed writing classification for test artefact {test_artefact_id}")
                })?;
            inserted += 1;
        }

        Ok(inserted)
    }
}

impl TestHarnessQueryRepository for SqliteTestHarnessRepository {
    fn find_artefact(
        &self,
        commit_sha: &str,
        artefact_query: &str,
    ) -> Result<QueriedArtefactRecord> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
SELECT artefact_id, symbol_fqn, canonical_kind, path, start_line, end_line
FROM artefacts
WHERE commit_sha = ?1
  AND (
    artefact_id = ?2
    OR symbol_fqn = ?2
    OR path = ?2
    OR symbol_fqn LIKE '%' || ?2
  )
ORDER BY
  CASE
    WHEN symbol_fqn = ?2 THEN 0
    WHEN artefact_id = ?2 THEN 1
    WHEN path = ?2 THEN 2
    ELSE 3
  END ASC,
  start_line ASC
LIMIT 1
"#,
            )
            .context("failed preparing artefact lookup query")?;

        let mut rows = stmt
            .query(params![commit_sha, artefact_query])
            .context("failed querying artefact")?;
        let Some(row) = rows.next().context("failed reading artefact row")? else {
            let indexed_for_commit: Option<i64> = self
                .conn
                .query_row(
                    "SELECT 1 FROM artefacts WHERE commit_sha = ?1 LIMIT 1",
                    params![commit_sha],
                    |row| row.get(0),
                )
                .optional()
                .context("failed checking indexed state for commit")?;

            if indexed_for_commit.is_some() {
                anyhow::bail!("Artefact not found");
            }

            anyhow::bail!("Repository not indexed");
        };

        Ok(QueriedArtefactRecord {
            artefact_id: row.get(0).context("missing artefact_id")?,
            symbol_fqn: row.get(1).context("missing symbol_fqn")?,
            canonical_kind: row.get(2).context("missing canonical_kind")?,
            path: row.get(3).context("missing path")?,
            start_line: row.get(4).context("missing start_line")?,
            end_line: row.get(5).context("missing end_line")?,
        })
    }

    fn list_artefacts(
        &self,
        commit_sha: &str,
        kind: Option<&str>,
    ) -> Result<Vec<ListedArtefactRecord>> {
        if let Some(kind) = kind {
            let mut stmt = self
                .conn
                .prepare(
                    r#"
SELECT artefact_id, symbol_fqn, canonical_kind, path, start_line, end_line
FROM artefacts
WHERE commit_sha = ?1
  AND canonical_kind = ?2
ORDER BY path ASC, start_line ASC
"#,
                )
                .context("failed preparing list query")?;

            let rows = stmt
                .query_map(params![commit_sha, kind], |row| {
                    Ok(ListedArtefactRecord {
                        artefact_id: row.get(0)?,
                        symbol_fqn: row.get(1)?,
                        kind: row.get(2)?,
                        file_path: row.get(3)?,
                        start_line: row.get(4)?,
                        end_line: row.get(5)?,
                    })
                })
                .context("failed querying artefacts")?;

            let mut output = Vec::new();
            for row in rows {
                output.push(row.context("failed decoding list row")?);
            }
            Ok(output)
        } else {
            let mut stmt = self
                .conn
                .prepare(
                    r#"
SELECT artefact_id, symbol_fqn, canonical_kind, path, start_line, end_line
FROM artefacts
WHERE commit_sha = ?1
ORDER BY path ASC, start_line ASC
"#,
                )
                .context("failed preparing list query")?;

            let rows = stmt
                .query_map(params![commit_sha], |row| {
                    Ok(ListedArtefactRecord {
                        artefact_id: row.get(0)?,
                        symbol_fqn: row.get(1)?,
                        kind: row.get(2)?,
                        file_path: row.get(3)?,
                        start_line: row.get(4)?,
                        end_line: row.get(5)?,
                    })
                })
                .context("failed querying artefacts")?;

            let mut output = Vec::new();
            for row in rows {
                output.push(row.context("failed decoding list row")?);
            }
            Ok(output)
        }
    }

    fn load_covering_tests(
        &self,
        commit_sha: &str,
        production_artefact_id: &str,
    ) -> Result<Vec<CoveringTestRecord>> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
SELECT DISTINCT
  t.artefact_id,
  t.symbol_fqn,
  t.signature,
  t.path,
  s.symbol_fqn AS suite_name,
  tc.classification,
  tc.classification_source,
  tc.fan_out
FROM test_links tl
JOIN artefacts t
  ON t.artefact_id = tl.test_artefact_id
  AND t.commit_sha = ?1
LEFT JOIN artefacts s
  ON s.artefact_id = t.parent_artefact_id
LEFT JOIN test_classifications tc
  ON tc.test_artefact_id = t.artefact_id
  AND tc.commit_sha = ?1
WHERE tl.commit_sha = ?1
  AND tl.production_artefact_id = ?2
ORDER BY t.path ASC, t.start_line ASC
"#,
            )
            .context("failed preparing covering tests query")?;

        let rows = stmt
            .query_map(params![commit_sha, production_artefact_id], |row| {
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
SELECT test_artefact_id, COUNT(DISTINCT production_artefact_id)
FROM test_links
WHERE commit_sha = ?1
GROUP BY test_artefact_id
"#,
            )
            .context("failed preparing linked fan-out query")?;

        let mut rows = stmt
            .query(params![commit_sha])
            .context("failed executing linked fan-out query")?;

        let mut output = HashMap::new();
        while let Some(row) = rows.next().context("failed reading linked fan-out row")? {
            let test_artefact_id: String = row.get(0).context("missing test_artefact_id")?;
            let fan_out: i64 = row.get(1).context("missing fan_out")?;
            output.insert(test_artefact_id, fan_out);
        }
        Ok(output)
    }

    fn coverage_exists_for_commit(&self, commit_sha: &str) -> Result<bool> {
        let mut stmt = self
            .conn
            .prepare("SELECT EXISTS(SELECT 1 FROM test_coverage WHERE commit_sha = ?1)")
            .context("failed preparing coverage existence query")?;
        let exists: i64 = stmt
            .query_row(params![commit_sha], |row| row.get(0))
            .context("failed querying coverage existence")?;
        Ok(exists == 1)
    }

    fn load_coverage_pair_stats(
        &self,
        commit_sha: &str,
        test_artefact_id: &str,
        artefact_id: &str,
    ) -> Result<CoveragePairStats> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
SELECT
  COUNT(*) AS total_rows,
  COALESCE(SUM(CASE WHEN covered = 1 THEN 1 ELSE 0 END), 0) AS covered_rows
FROM test_coverage
WHERE commit_sha = ?1
  AND test_artefact_id = ?2
  AND artefact_id = ?3
"#,
            )
            .context("failed preparing pair coverage query")?;

        let stats = stmt
            .query_row(params![commit_sha, test_artefact_id, artefact_id], |row| {
                Ok(CoveragePairStats {
                    total_rows: row.get(0)?,
                    covered_rows: row.get(1)?,
                })
            })
            .context("failed querying pair coverage stats")?;
        Ok(stats)
    }

    fn load_latest_test_run(
        &self,
        commit_sha: &str,
        test_artefact_id: &str,
    ) -> Result<Option<LatestTestRunRecord>> {
        let mut stmt = self
            .conn
            .prepare(
                r#"
SELECT status, duration_ms, commit_sha
FROM test_runs
WHERE test_artefact_id = ?1
  AND commit_sha = ?2
ORDER BY ran_at DESC
LIMIT 1
"#,
            )
            .context("failed preparing last run query")?;

        let mut rows = stmt
            .query(params![test_artefact_id, commit_sha])
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
        artefact_id: &str,
    ) -> Result<Option<CoverageSummaryRecord>> {
        let mut line_stmt = self
            .conn
            .prepare(
                r#"
SELECT line, MAX(CASE WHEN covered = 1 THEN 1 ELSE 0 END) AS covered_any
FROM test_coverage
WHERE commit_sha = ?1
  AND artefact_id = ?2
  AND branch_id IS NULL
GROUP BY line
ORDER BY line
"#,
            )
            .context("failed preparing line coverage summary query")?;

        let line_rows = line_stmt
            .query_map(params![commit_sha, artefact_id], |row| {
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
  line,
  branch_id,
  MAX(CASE WHEN covered = 1 THEN 1 ELSE 0 END) AS covered_any,
  GROUP_CONCAT(DISTINCT CASE WHEN covered = 1 THEN test_artefact_id END) AS covering_test_ids
FROM test_coverage
WHERE commit_sha = ?1
  AND artefact_id = ?2
  AND branch_id IS NOT NULL
GROUP BY line, branch_id
ORDER BY line, branch_id
"#,
            )
            .context("failed preparing branch coverage summary query")?;

        let branch_rows = branch_stmt
            .query_map(params![commit_sha, artefact_id], |row| {
                let covering_test_ids_raw: Option<String> = row.get(3)?;
                Ok(CoverageBranchRecord {
                    line: row.get(0)?,
                    branch_id: row.get(1)?,
                    covered: row.get::<_, i64>(2)? == 1,
                    covering_test_ids: parse_grouped_ids(covering_test_ids_raw.as_deref()),
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
}

fn clear_existing_production_data(conn: &Connection, commit_sha: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM test_links WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing test_links for commit")?;
    conn.execute(
        "DELETE FROM test_coverage WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing test_coverage for commit")?;
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
        r#"
DELETE FROM artefacts
WHERE commit_sha = ?1
  AND canonical_kind NOT IN ('test_suite', 'test_scenario')
  AND NOT (
    canonical_kind = 'file'
    AND (
      path LIKE 'tests/%'
      OR path LIKE '%/__tests__/%'
      OR path LIKE '%.test.ts'
      OR path LIKE '%.spec.ts'
      OR path LIKE '%.test.rs'
      OR path LIKE '%.spec.rs'
    )
  )
"#,
        params![commit_sha],
    )
    .context("failed clearing existing production artefacts for commit")?;
    Ok(())
}

fn clear_existing_test_discovery_data(conn: &Connection, commit_sha: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM test_classifications WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing existing test classifications for commit")?;
    conn.execute(
        "DELETE FROM test_coverage WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing existing test coverage for commit")?;
    conn.execute(
        "DELETE FROM test_links WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing existing test links for commit")?;
    conn.execute(
        "DELETE FROM test_runs WHERE commit_sha = ?1",
        params![commit_sha],
    )
    .context("failed clearing existing test runs for commit")?;
    conn.execute(
        r#"
DELETE FROM artefacts
WHERE commit_sha = ?1
  AND (
    canonical_kind IN ('test_suite', 'test_scenario')
    OR (canonical_kind = 'file' AND (
      path LIKE 'tests/%'
      OR path LIKE '%/tests/%'
      OR path LIKE '%/__tests__/%'
      OR path LIKE '%.test.ts'
      OR path LIKE '%.spec.ts'
      OR path LIKE '%.test.rs'
      OR path LIKE '%.spec.rs'
    ))
  )
"#,
        params![commit_sha],
    )
    .context("failed clearing existing test artefacts for commit")?;
    Ok(())
}

fn upsert_artefact(conn: &Connection, artefact: &ArtefactRecord) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO artefacts (
  artefact_id, symbol_id, repo_id, blob_sha, commit_sha, path, language, canonical_kind,
  language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, content_hash
) VALUES (
  ?1, NULL, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, NULL, ?12, NULL
)
ON CONFLICT(artefact_id) DO UPDATE SET
  repo_id = excluded.repo_id,
  commit_sha = excluded.commit_sha,
  path = excluded.path,
  language = excluded.language,
  canonical_kind = excluded.canonical_kind,
  language_kind = excluded.language_kind,
  symbol_fqn = excluded.symbol_fqn,
  parent_artefact_id = excluded.parent_artefact_id,
  start_line = excluded.start_line,
  end_line = excluded.end_line,
  signature = excluded.signature
"#,
        params![
            artefact.artefact_id,
            artefact.repo_id,
            artefact.commit_sha,
            artefact.path,
            artefact.language,
            artefact.canonical_kind,
            artefact.language_kind,
            artefact.symbol_fqn,
            artefact.parent_artefact_id,
            artefact.start_line,
            artefact.end_line,
            artefact.signature
        ],
    )
    .with_context(|| format!("failed upserting artefact {}", artefact.artefact_id))?;
    Ok(())
}

fn upsert_test_link(conn: &Connection, link: &TestLinkRecord) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO test_links (test_link_id, test_artefact_id, production_artefact_id, link_source, commit_sha)
VALUES (?1, ?2, ?3, 'static_analysis', ?4)
ON CONFLICT(test_link_id) DO UPDATE SET
  test_artefact_id = excluded.test_artefact_id,
  production_artefact_id = excluded.production_artefact_id,
  link_source = excluded.link_source,
  commit_sha = excluded.commit_sha
"#,
        params![
            link.test_link_id,
            link.test_artefact_id,
            link.production_artefact_id,
            link.commit_sha
        ],
    )
    .with_context(|| format!("failed upserting test link {}", link.test_link_id))?;
    Ok(())
}

fn upsert_test_run(conn: &Connection, run: &TestRunRecord) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO test_runs (run_id, repo_id, commit_sha, test_artefact_id, status, duration_ms, ran_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
ON CONFLICT(run_id) DO UPDATE SET
  repo_id = excluded.repo_id,
  commit_sha = excluded.commit_sha,
  test_artefact_id = excluded.test_artefact_id,
  status = excluded.status,
  duration_ms = excluded.duration_ms,
  ran_at = excluded.ran_at
"#,
        params![
            run.run_id,
            run.repo_id,
            run.commit_sha,
            run.test_artefact_id,
            run.status,
            run.duration_ms,
            run.ran_at
        ],
    )
    .with_context(|| format!("failed inserting test run {}", run.run_id))?;
    Ok(())
}

fn upsert_test_coverage(conn: &Connection, row: &TestCoverageRecord) -> Result<()> {
    conn.execute(
        r#"
INSERT INTO test_coverage (
  coverage_id,
  repo_id,
  commit_sha,
  test_artefact_id,
  artefact_id,
  line,
  branch_id,
  covered,
  hit_count
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
ON CONFLICT(coverage_id) DO UPDATE SET
  repo_id = excluded.repo_id,
  commit_sha = excluded.commit_sha,
  test_artefact_id = excluded.test_artefact_id,
  artefact_id = excluded.artefact_id,
  line = excluded.line,
  branch_id = excluded.branch_id,
  covered = excluded.covered,
  hit_count = excluded.hit_count;
"#,
        params![
            row.coverage_id,
            row.repo_id,
            row.commit_sha,
            row.test_artefact_id,
            row.artefact_id,
            row.line,
            row.branch_id,
            row.covered,
            row.hit_count
        ],
    )
    .with_context(|| format!("failed upserting test coverage {}", row.coverage_id))?;
    Ok(())
}

fn parse_grouped_ids(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or("")
        .split(',')
        .filter(|item| !item.is_empty())
        .map(|item| item.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::SqliteTestHarnessRepository;
    use crate::db::init_database;
    use crate::domain::ArtefactRecord;
    use crate::repository::TestHarnessRepository;

    #[test]
    fn load_repo_id_for_commit_supports_workspace_crate_paths() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let db_path = temp_dir.path().join("workspace-layout.db");
        init_database(&db_path, false, "seed").expect("failed to initialize db");

        let mut repository =
            SqliteTestHarnessRepository::open_existing(&db_path).expect("open db");
        repository
            .replace_production_artefacts(
                "commit-workspace",
                &[ArtefactRecord {
                    artefact_id: "file:workspace".to_string(),
                    repo_id: "ruff-workspace".to_string(),
                    commit_sha: "commit-workspace".to_string(),
                    path: "crates/ruff/src/lib.rs".to_string(),
                    language: "rust".to_string(),
                    canonical_kind: "file".to_string(),
                    language_kind: Some("source_file".to_string()),
                    symbol_fqn: Some("crates/ruff/src/lib.rs".to_string()),
                    parent_artefact_id: None,
                    start_line: 1,
                    end_line: 10,
                    signature: None,
                }],
            )
            .expect("replace production artefacts");

        let repo_id = repository
            .load_repo_id_for_commit("commit-workspace")
            .expect("load repo id");
        assert_eq!(repo_id, "ruff-workspace");
    }

    #[test]
    fn load_production_artefacts_includes_workspace_crate_functions() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let db_path = temp_dir.path().join("workspace-functions.db");
        init_database(&db_path, false, "seed").expect("failed to initialize db");

        let mut repository =
            SqliteTestHarnessRepository::open_existing(&db_path).expect("open db");
        repository
            .replace_production_artefacts(
                "commit-workspace",
                &[ArtefactRecord {
                    artefact_id: "function:workspace".to_string(),
                    repo_id: "ruff-workspace".to_string(),
                    commit_sha: "commit-workspace".to_string(),
                    path: "crates/ruff/src/version.rs".to_string(),
                    language: "rust".to_string(),
                    canonical_kind: "function".to_string(),
                    language_kind: Some("function_item".to_string()),
                    symbol_fqn: Some("crates/ruff/src/version.rs::version".to_string()),
                    parent_artefact_id: None,
                    start_line: 1,
                    end_line: 5,
                    signature: Some("pub fn version() -> String".to_string()),
                }],
            )
            .expect("replace production artefacts");

        let artefacts = repository
            .load_production_artefacts("commit-workspace")
            .expect("load production artefacts");
        assert_eq!(artefacts.len(), 1);
        assert_eq!(artefacts[0].artefact_id, "function:workspace");
    }
}
