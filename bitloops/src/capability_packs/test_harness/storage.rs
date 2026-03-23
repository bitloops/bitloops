// Repository traits for command-side persistence. Command handlers build domain
// records and delegate raw SQL and transaction details to an implementation.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use crate::models::{
    CoverageCaptureRecord, CoverageDiagnosticRecord, CoverageHitRecord, CoveragePairStats,
    CoverageSummaryRecord, CoveringTestRecord, LatestTestRunRecord, ListedArtefactRecord,
    ProductionArtefact, ProductionIngestionBatch, QueriedArtefactRecord,
    ResolvedTestScenarioRecord, TestDiscoveryDiagnosticRecord, TestDiscoveryRunRecord,
    TestHarnessCommitCounts, TestLinkRecord, TestRunRecord, TestScenarioRecord, TestSuiteRecord,
};

pub mod dispatch;
pub mod postgres;
pub mod schema;
pub mod sqlite;

/// Narrow coverage gateway: subset of TestHarnessRepository used by coverage ingest paths.
pub trait TestHarnessCoverageGateway: Send {
    fn load_repo_id_for_commit(&self, commit_sha: &str) -> Result<String>;
    fn load_artefacts_for_file_lines(
        &self,
        commit_sha: &str,
        file_path: &str,
    ) -> Result<Vec<(String, i64, i64)>>;
    fn insert_coverage_capture(&mut self, capture: &CoverageCaptureRecord) -> Result<()>;
    fn insert_coverage_hits(&mut self, hits: &[CoverageHitRecord]) -> Result<()>;
    fn insert_coverage_diagnostics(
        &mut self,
        diagnostics: &[CoverageDiagnosticRecord],
    ) -> Result<()>;
    fn rebuild_classifications_from_coverage(&mut self, commit_sha: &str) -> Result<usize>;
}

pub trait TestHarnessRepository {
    fn load_repo_id_for_commit(&self, commit_sha: &str) -> Result<String>;
    fn load_production_artefacts(&self, commit_sha: &str) -> Result<Vec<ProductionArtefact>>;
    fn load_test_scenarios(&self, commit_sha: &str) -> Result<Vec<ResolvedTestScenarioRecord>>;
    fn load_artefacts_for_file_lines(
        &self,
        commit_sha: &str,
        file_path: &str,
    ) -> Result<Vec<(String, i64, i64)>>;

    fn replace_production_artefacts(&mut self, batch: &ProductionIngestionBatch) -> Result<()>;
    fn replace_test_discovery(
        &mut self,
        commit_sha: &str,
        suites: &[TestSuiteRecord],
        scenarios: &[TestScenarioRecord],
        links: &[TestLinkRecord],
        discovery_run: &TestDiscoveryRunRecord,
        diagnostics: &[TestDiscoveryDiagnosticRecord],
    ) -> Result<()>;
    fn replace_test_runs(&mut self, commit_sha: &str, runs: &[TestRunRecord]) -> Result<()>;
    fn insert_coverage_capture(&mut self, capture: &CoverageCaptureRecord) -> Result<()>;
    fn insert_coverage_hits(&mut self, hits: &[CoverageHitRecord]) -> Result<()>;
    fn insert_coverage_diagnostics(
        &mut self,
        diagnostics: &[CoverageDiagnosticRecord],
    ) -> Result<()>;
    fn rebuild_classifications_from_coverage(&mut self, commit_sha: &str) -> Result<usize>;
}

pub trait TestHarnessQueryRepository {
    fn find_artefact(
        &self,
        commit_sha: &str,
        artefact_query: &str,
    ) -> Result<QueriedArtefactRecord>;
    fn list_artefacts(
        &self,
        commit_sha: &str,
        kind: Option<&str>,
    ) -> Result<Vec<ListedArtefactRecord>>;
    fn load_covering_tests(
        &self,
        commit_sha: &str,
        production_artefact_id: &str,
    ) -> Result<Vec<CoveringTestRecord>>;
    fn load_linked_fan_out_by_test(&self, commit_sha: &str) -> Result<HashMap<String, i64>>;
    fn coverage_exists_for_commit(&self, commit_sha: &str) -> Result<bool>;
    fn load_coverage_pair_stats(
        &self,
        commit_sha: &str,
        test_scenario_id: &str,
        artefact_id: &str,
    ) -> Result<CoveragePairStats>;
    fn load_latest_test_run(
        &self,
        commit_sha: &str,
        test_scenario_id: &str,
    ) -> Result<Option<LatestTestRunRecord>>;
    fn load_coverage_summary(
        &self,
        commit_sha: &str,
        artefact_id: &str,
    ) -> Result<Option<CoverageSummaryRecord>>;

    fn load_test_harness_commit_counts(&self, commit_sha: &str) -> Result<TestHarnessCommitCounts>;
}

pub use dispatch::{BitloopsTestHarnessRepository, init_schema_for_repo, open_repository_for_repo};
pub use postgres::PostgresTestHarnessRepository;
pub use sqlite::SqliteTestHarnessRepository;

pub fn open_sqlite_repository(db_path: &Path) -> Result<SqliteTestHarnessRepository> {
    SqliteTestHarnessRepository::open_existing(db_path)
}
