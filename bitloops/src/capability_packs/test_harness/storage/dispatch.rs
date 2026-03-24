use std::path::Path;

use anyhow::{Context, Result, anyhow};

use super::{
    PostgresTestHarnessRepository, SqliteTestHarnessRepository, TestHarnessCoverageGateway,
    TestHarnessQueryRepository, TestHarnessRepository, init_test_domain_database,
};
use crate::config::resolve_store_backend_config_for_repo;
use crate::models::{
    CoverageCaptureRecord, CoverageDiagnosticRecord, CoverageHitRecord, CoveragePairStats,
    CoverageSummaryRecord, CoveringTestRecord, LatestTestRunRecord, ListedArtefactRecord,
    ProductionIngestionBatch, QueriedArtefactRecord, ResolvedTestScenarioRecord,
    StageBranchCoverageRecord, StageCoverageMetadataRecord, StageCoveringTestRecord,
    StageLineCoverageRecord, TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord,
    TestDiscoveryDiagnosticRecord, TestDiscoveryRunRecord, TestHarnessCommitCounts, TestRunRecord,
};
pub enum BitloopsTestHarnessRepository {
    Sqlite(SqliteTestHarnessRepository),
    Postgres(PostgresTestHarnessRepository),
}

pub fn init_schema_for_repo(repo_root: &Path) -> Result<()> {
    let backends = resolve_store_backend_config_for_repo(repo_root)
        .context("resolving Bitloops store config for test-harness schema init")?;

    if backends.relational.has_postgres() {
        let dsn = backends.relational.postgres_dsn.ok_or_else(|| {
            anyhow!("test-harness schema init requires stores.relational.postgres_dsn")
        })?;
        let repository = PostgresTestHarnessRepository::connect(dsn)?;
        repository.initialise_schema()?;
        println!("Postgres test-harness schema initialized");
        Ok(())
    } else {
        let sqlite_path = backends
            .relational
            .resolve_sqlite_db_path_for_repo(repo_root)
            .context("resolving SQLite path for test-harness schema init")?;
        init_test_domain_database(&sqlite_path)
    }
}

pub fn open_repository_for_repo(repo_root: &Path) -> Result<BitloopsTestHarnessRepository> {
    let backends = resolve_store_backend_config_for_repo(repo_root)
        .context("resolving Bitloops store config for `bitloops testlens`")?;

    if backends.relational.has_postgres() {
        let dsn = backends.relational.postgres_dsn.ok_or_else(|| {
            anyhow!("`bitloops testlens` requires stores.relational.postgres_dsn")
        })?;
        Ok(BitloopsTestHarnessRepository::Postgres(
            PostgresTestHarnessRepository::connect(dsn)?,
        ))
    } else {
        let sqlite_path = backends
            .relational
            .resolve_sqlite_db_path_for_repo(repo_root)
            .context("resolving SQLite path for `bitloops testlens`")?;
        Ok(BitloopsTestHarnessRepository::Sqlite(
            SqliteTestHarnessRepository::open_existing(&sqlite_path)?,
        ))
    }
}

impl TestHarnessRepository for BitloopsTestHarnessRepository {
    fn load_test_scenarios(&self, commit_sha: &str) -> Result<Vec<ResolvedTestScenarioRecord>> {
        match self {
            Self::Sqlite(repository) => repository.load_test_scenarios(commit_sha),
            Self::Postgres(repository) => repository.load_test_scenarios(commit_sha),
        }
    }

    fn replace_production_artefacts(&mut self, batch: &ProductionIngestionBatch) -> Result<()> {
        match self {
            Self::Sqlite(repository) => repository.replace_production_artefacts(batch),
            Self::Postgres(repository) => repository.replace_production_artefacts(batch),
        }
    }

    fn replace_test_discovery(
        &mut self,
        commit_sha: &str,
        test_artefacts: &[TestArtefactCurrentRecord],
        test_edges: &[TestArtefactEdgeCurrentRecord],
        discovery_run: &TestDiscoveryRunRecord,
        diagnostics: &[TestDiscoveryDiagnosticRecord],
    ) -> Result<()> {
        match self {
            Self::Sqlite(repository) => repository.replace_test_discovery(
                commit_sha,
                test_artefacts,
                test_edges,
                discovery_run,
                diagnostics,
            ),
            Self::Postgres(repository) => repository.replace_test_discovery(
                commit_sha,
                test_artefacts,
                test_edges,
                discovery_run,
                diagnostics,
            ),
        }
    }

    fn replace_test_runs(&mut self, commit_sha: &str, runs: &[TestRunRecord]) -> Result<()> {
        match self {
            Self::Sqlite(repository) => repository.replace_test_runs(commit_sha, runs),
            Self::Postgres(repository) => repository.replace_test_runs(commit_sha, runs),
        }
    }

    fn insert_coverage_capture(&mut self, capture: &CoverageCaptureRecord) -> Result<()> {
        match self {
            Self::Sqlite(repository) => repository.insert_coverage_capture(capture),
            Self::Postgres(repository) => repository.insert_coverage_capture(capture),
        }
    }

    fn insert_coverage_hits(&mut self, hits: &[CoverageHitRecord]) -> Result<()> {
        match self {
            Self::Sqlite(repository) => repository.insert_coverage_hits(hits),
            Self::Postgres(repository) => repository.insert_coverage_hits(hits),
        }
    }

    fn insert_coverage_diagnostics(
        &mut self,
        diagnostics: &[CoverageDiagnosticRecord],
    ) -> Result<()> {
        match self {
            Self::Sqlite(repository) => repository.insert_coverage_diagnostics(diagnostics),
            Self::Postgres(repository) => repository.insert_coverage_diagnostics(diagnostics),
        }
    }

    fn rebuild_classifications_from_coverage(&mut self, commit_sha: &str) -> Result<usize> {
        match self {
            Self::Sqlite(repository) => {
                repository.rebuild_classifications_from_coverage(commit_sha)
            }
            Self::Postgres(repository) => {
                repository.rebuild_classifications_from_coverage(commit_sha)
            }
        }
    }
}

impl TestHarnessCoverageGateway for BitloopsTestHarnessRepository {
    fn insert_coverage_capture(&mut self, capture: &CoverageCaptureRecord) -> Result<()> {
        TestHarnessRepository::insert_coverage_capture(self, capture)
    }

    fn insert_coverage_hits(&mut self, hits: &[CoverageHitRecord]) -> Result<()> {
        TestHarnessRepository::insert_coverage_hits(self, hits)
    }

    fn insert_coverage_diagnostics(
        &mut self,
        diagnostics: &[CoverageDiagnosticRecord],
    ) -> Result<()> {
        TestHarnessRepository::insert_coverage_diagnostics(self, diagnostics)
    }

    fn rebuild_classifications_from_coverage(&mut self, commit_sha: &str) -> Result<usize> {
        TestHarnessRepository::rebuild_classifications_from_coverage(self, commit_sha)
    }
}

impl TestHarnessQueryRepository for BitloopsTestHarnessRepository {
    fn find_artefact(
        &self,
        commit_sha: &str,
        artefact_query: &str,
    ) -> Result<QueriedArtefactRecord> {
        match self {
            Self::Sqlite(repository) => repository.find_artefact(commit_sha, artefact_query),
            Self::Postgres(repository) => repository.find_artefact(commit_sha, artefact_query),
        }
    }

    fn list_artefacts(
        &self,
        commit_sha: &str,
        kind: Option<&str>,
    ) -> Result<Vec<ListedArtefactRecord>> {
        match self {
            Self::Sqlite(repository) => repository.list_artefacts(commit_sha, kind),
            Self::Postgres(repository) => repository.list_artefacts(commit_sha, kind),
        }
    }

    fn load_covering_tests(
        &self,
        commit_sha: &str,
        production_symbol_id: &str,
    ) -> Result<Vec<CoveringTestRecord>> {
        match self {
            Self::Sqlite(repository) => {
                repository.load_covering_tests(commit_sha, production_symbol_id)
            }
            Self::Postgres(repository) => {
                repository.load_covering_tests(commit_sha, production_symbol_id)
            }
        }
    }

    fn load_linked_fan_out_by_test(
        &self,
        commit_sha: &str,
    ) -> Result<std::collections::HashMap<String, i64>> {
        match self {
            Self::Sqlite(repository) => repository.load_linked_fan_out_by_test(commit_sha),
            Self::Postgres(repository) => repository.load_linked_fan_out_by_test(commit_sha),
        }
    }

    fn coverage_exists_for_commit(&self, commit_sha: &str) -> Result<bool> {
        match self {
            Self::Sqlite(repository) => repository.coverage_exists_for_commit(commit_sha),
            Self::Postgres(repository) => repository.coverage_exists_for_commit(commit_sha),
        }
    }

    fn load_coverage_pair_stats(
        &self,
        commit_sha: &str,
        test_symbol_id: &str,
        production_symbol_id: &str,
    ) -> Result<CoveragePairStats> {
        match self {
            Self::Sqlite(repository) => repository.load_coverage_pair_stats(
                commit_sha,
                test_symbol_id,
                production_symbol_id,
            ),
            Self::Postgres(repository) => repository.load_coverage_pair_stats(
                commit_sha,
                test_symbol_id,
                production_symbol_id,
            ),
        }
    }

    fn load_latest_test_run(
        &self,
        commit_sha: &str,
        test_symbol_id: &str,
    ) -> Result<Option<LatestTestRunRecord>> {
        match self {
            Self::Sqlite(repository) => repository.load_latest_test_run(commit_sha, test_symbol_id),
            Self::Postgres(repository) => {
                repository.load_latest_test_run(commit_sha, test_symbol_id)
            }
        }
    }

    fn load_coverage_summary(
        &self,
        commit_sha: &str,
        production_symbol_id: &str,
    ) -> Result<Option<CoverageSummaryRecord>> {
        match self {
            Self::Sqlite(repository) => {
                repository.load_coverage_summary(commit_sha, production_symbol_id)
            }
            Self::Postgres(repository) => {
                repository.load_coverage_summary(commit_sha, production_symbol_id)
            }
        }
    }

    fn load_test_harness_commit_counts(&self, commit_sha: &str) -> Result<TestHarnessCommitCounts> {
        match self {
            Self::Sqlite(repository) => repository.load_test_harness_commit_counts(commit_sha),
            Self::Postgres(repository) => repository.load_test_harness_commit_counts(commit_sha),
        }
    }

    fn load_stage_covering_tests(
        &self,
        repo_id: &str,
        production_symbol_id: &str,
        min_confidence: Option<f64>,
        linkage_source: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StageCoveringTestRecord>> {
        match self {
            Self::Sqlite(repository) => repository.load_stage_covering_tests(
                repo_id,
                production_symbol_id,
                min_confidence,
                linkage_source,
                limit,
            ),
            Self::Postgres(repository) => repository.load_stage_covering_tests(
                repo_id,
                production_symbol_id,
                min_confidence,
                linkage_source,
                limit,
            ),
        }
    }

    fn load_stage_line_coverage(
        &self,
        repo_id: &str,
        production_symbol_id: &str,
        commit_sha: Option<&str>,
    ) -> Result<Vec<StageLineCoverageRecord>> {
        match self {
            Self::Sqlite(repository) => {
                repository.load_stage_line_coverage(repo_id, production_symbol_id, commit_sha)
            }
            Self::Postgres(repository) => {
                repository.load_stage_line_coverage(repo_id, production_symbol_id, commit_sha)
            }
        }
    }

    fn load_stage_branch_coverage(
        &self,
        repo_id: &str,
        production_symbol_id: &str,
        commit_sha: Option<&str>,
    ) -> Result<Vec<StageBranchCoverageRecord>> {
        match self {
            Self::Sqlite(repository) => {
                repository.load_stage_branch_coverage(repo_id, production_symbol_id, commit_sha)
            }
            Self::Postgres(repository) => {
                repository.load_stage_branch_coverage(repo_id, production_symbol_id, commit_sha)
            }
        }
    }

    fn load_stage_coverage_metadata(
        &self,
        repo_id: &str,
        commit_sha: Option<&str>,
    ) -> Result<Option<StageCoverageMetadataRecord>> {
        match self {
            Self::Sqlite(repository) => {
                repository.load_stage_coverage_metadata(repo_id, commit_sha)
            }
            Self::Postgres(repository) => {
                repository.load_stage_coverage_metadata(repo_id, commit_sha)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::init_schema_for_repo;
    use anyhow::Result;
    use tempfile::TempDir;

    #[test]
    fn init_schema_for_repo_initialises_sqlite_test_harness_tables() -> Result<()> {
        let temp = TempDir::new()?;
        let repo_root = temp.path();
        let sqlite_path = repo_root.join("stores").join("relational.sqlite");
        let config_dir = repo_root.join(".bitloops");
        std::fs::create_dir_all(&config_dir)?;
        std::fs::write(
            config_dir.join("config.json"),
            format!(
                r#"{{
  "version": "1.0",
  "scope": "project",
  "settings": {{
    "stores": {{
      "relational": {{
        "sqlite_path": "{}"
      }}
    }}
  }}
}}"#,
                sqlite_path.display()
            ),
        )?;

        init_schema_for_repo(repo_root)?;

        let conn = rusqlite::Connection::open(&sqlite_path)?;
        for table in [
            "test_artefacts_current",
            "test_artefact_edges_current",
            "coverage_captures",
            "coverage_hits",
            "test_discovery_runs",
        ] {
            let exists: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )?;
            assert_eq!(exists, 1, "expected SQLite test-domain table `{table}`");
        }

        Ok(())
    }
}
