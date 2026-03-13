// Repository traits for command-side persistence. Command handlers build domain
// records and delegate raw SQL and transaction details to an implementation.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use crate::domain::{
    ArtefactRecord, CoveragePairStats, CoverageSummaryRecord, CoverageTarget, CoveringTestRecord,
    LatestTestRunRecord, ListedArtefactRecord, ProductionArtefact, QueriedArtefactRecord,
    TestCoverageRecord, TestLinkRecord, TestRunRecord, TestScenarioRecord,
};

pub mod sqlite;

pub trait TestHarnessRepository {
    fn load_repo_id_for_commit(&self, commit_sha: &str) -> Result<String>;
    fn load_production_artefacts(&self, commit_sha: &str) -> Result<Vec<ProductionArtefact>>;
    fn load_test_scenarios(&self, commit_sha: &str) -> Result<Vec<TestScenarioRecord>>;
    fn load_test_links_by_production_artefact(
        &self,
        commit_sha: &str,
    ) -> Result<HashMap<String, Vec<String>>>;
    fn load_coverage_targets_for_file(
        &self,
        commit_sha: &str,
        lcov_source_file: &str,
    ) -> Result<Vec<CoverageTarget>>;

    fn replace_production_artefacts(
        &mut self,
        commit_sha: &str,
        artefacts: &[ArtefactRecord],
    ) -> Result<()>;
    fn replace_test_discovery(
        &mut self,
        commit_sha: &str,
        artefacts: &[ArtefactRecord],
        links: &[TestLinkRecord],
    ) -> Result<()>;
    fn replace_test_runs(&mut self, commit_sha: &str, runs: &[TestRunRecord]) -> Result<()>;
    fn replace_test_coverage(
        &mut self,
        commit_sha: &str,
        coverage_rows: &[TestCoverageRecord],
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
        test_artefact_id: &str,
        artefact_id: &str,
    ) -> Result<CoveragePairStats>;
    fn load_latest_test_run(
        &self,
        commit_sha: &str,
        test_artefact_id: &str,
    ) -> Result<Option<LatestTestRunRecord>>;
    fn load_coverage_summary(
        &self,
        commit_sha: &str,
        artefact_id: &str,
    ) -> Result<Option<CoverageSummaryRecord>>;
}

pub use sqlite::SqliteTestHarnessRepository;

pub fn open_sqlite_repository(db_path: &Path) -> Result<SqliteTestHarnessRepository> {
    SqliteTestHarnessRepository::open_existing(db_path)
}
