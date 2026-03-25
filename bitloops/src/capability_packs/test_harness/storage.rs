// Repository traits for command-side persistence. Command handlers build domain
// records and delegate raw SQL and transaction details to an implementation.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::models::{
    CoverageCaptureRecord, CoverageDiagnosticRecord, CoverageHitRecord, CoveragePairStats,
    CoverageSummaryRecord, CoveringTestRecord, LatestTestRunRecord, ListedArtefactRecord,
    ProductionIngestionBatch, QueriedArtefactRecord, ResolvedTestScenarioRecord,
    StageBranchCoverageRecord, StageCoverageMetadataRecord, StageCoveringTestRecord,
    StageLineCoverageRecord, TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord,
    TestDiscoveryDiagnosticRecord, TestDiscoveryRunRecord, TestHarnessCommitCounts, TestRunRecord,
};

pub mod dispatch;
pub mod postgres;
pub mod schema;
pub mod sqlite;

/// Narrow coverage gateway: write-side subset of TestHarnessRepository used by coverage ingest paths.
/// Host-owned reads (repo resolution, artefact lookup) have moved to `RelationalGateway`.
pub trait TestHarnessCoverageGateway: Send {
    fn insert_coverage_capture(&mut self, capture: &CoverageCaptureRecord) -> Result<()>;
    fn insert_coverage_hits(&mut self, hits: &[CoverageHitRecord]) -> Result<()>;
    fn insert_coverage_diagnostics(
        &mut self,
        diagnostics: &[CoverageDiagnosticRecord],
    ) -> Result<()>;
    fn rebuild_classifications_from_coverage(&mut self, commit_sha: &str) -> Result<usize>;
}

pub trait TestHarnessRepository {
    fn load_test_scenarios(&self, commit_sha: &str) -> Result<Vec<ResolvedTestScenarioRecord>>;

    fn replace_production_artefacts(&mut self, batch: &ProductionIngestionBatch) -> Result<()>;
    fn replace_test_discovery(
        &mut self,
        commit_sha: &str,
        test_artefacts: &[TestArtefactCurrentRecord],
        test_edges: &[TestArtefactEdgeCurrentRecord],
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
        production_symbol_id: &str,
    ) -> Result<Vec<CoveringTestRecord>>;
    fn load_linked_fan_out_by_test(&self, commit_sha: &str) -> Result<HashMap<String, i64>>;
    fn coverage_exists_for_commit(&self, commit_sha: &str) -> Result<bool>;
    fn load_coverage_pair_stats(
        &self,
        commit_sha: &str,
        test_symbol_id: &str,
        production_symbol_id: &str,
    ) -> Result<CoveragePairStats>;
    fn load_latest_test_run(
        &self,
        commit_sha: &str,
        test_symbol_id: &str,
    ) -> Result<Option<LatestTestRunRecord>>;
    fn load_coverage_summary(
        &self,
        commit_sha: &str,
        production_symbol_id: &str,
    ) -> Result<Option<CoverageSummaryRecord>>;

    fn load_test_harness_commit_counts(&self, commit_sha: &str) -> Result<TestHarnessCommitCounts>;

    fn load_stage_covering_tests(
        &self,
        repo_id: &str,
        production_symbol_id: &str,
        min_confidence: Option<f64>,
        linkage_source: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StageCoveringTestRecord>>;

    fn load_stage_line_coverage(
        &self,
        repo_id: &str,
        production_symbol_id: &str,
        commit_sha: Option<&str>,
    ) -> Result<Vec<StageLineCoverageRecord>>;

    fn load_stage_branch_coverage(
        &self,
        repo_id: &str,
        production_symbol_id: &str,
        commit_sha: Option<&str>,
    ) -> Result<Vec<StageBranchCoverageRecord>>;

    fn load_stage_coverage_metadata(
        &self,
        repo_id: &str,
        commit_sha: Option<&str>,
    ) -> Result<Option<StageCoverageMetadataRecord>>;
}

pub use dispatch::{BitloopsTestHarnessRepository, init_schema_for_repo, open_repository_for_repo};
pub use postgres::PostgresTestHarnessRepository;
pub use sqlite::SqliteTestHarnessRepository;

pub fn init_test_domain_database(db_path: &Path) -> Result<()> {
    if let Some(parent) = db_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create parent directory for db path {}",
                db_path.display()
            )
        })?;
    }

    let conn = Connection::open(db_path).with_context(|| {
        format!(
            "failed to open or create sqlite database at {}",
            db_path.display()
        )
    })?;

    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .context("failed to enable foreign keys")?;
    conn.execute_batch(schema::sqlite_test_domain_schema_sql())
        .context("failed to create test-harness test-domain schema")?;

    println!(
        "test-harness test-domain schema initialized at {}",
        db_path.display()
    );
    Ok(())
}

pub fn open_sqlite_repository(db_path: &Path) -> Result<SqliteTestHarnessRepository> {
    SqliteTestHarnessRepository::open_existing(db_path)
}
