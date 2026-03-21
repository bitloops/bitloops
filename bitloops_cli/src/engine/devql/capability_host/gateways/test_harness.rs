use anyhow::Result;

use crate::domain::{CoverageCaptureRecord, CoverageDiagnosticRecord, CoverageHitRecord};

/// Relational test-harness writes (and reads needed for coverage ingest) scoped to the Test
/// Harness pack. Implemented by `BitloopsTestHarnessRepository` from the host runtime.
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
