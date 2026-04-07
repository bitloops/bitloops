use std::path::Path;

use anyhow::{Context, Result, anyhow};

use super::{
    PostgresTestHarnessRepository, SqliteTestHarnessRepository, TestHarnessCoverageGateway,
    TestHarnessQueryRepository, TestHarnessRepository, init_test_domain_database,
};
use crate::config::resolve_store_backend_config_for_repo;
use crate::host::relational_store::DefaultRelationalStore;
use crate::models::{
    CoverageCaptureRecord, CoverageDiagnosticRecord, CoverageHitRecord, CoveragePairStats,
    CoverageSummaryRecord, CoveringTestRecord, LatestTestRunRecord, ResolvedTestScenarioRecord,
    StageBranchCoverageRecord, StageCoverageMetadataRecord, StageCoveringTestRecord,
    StageLineCoverageRecord, TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord,
    TestHarnessCommitCounts, TestRunRecord,
};
pub enum BitloopsTestHarnessRepository {
    Sqlite(SqliteTestHarnessRepository),
    Postgres(PostgresTestHarnessRepository),
}

pub fn init_schema_for_repo(repo_root: &Path) -> Result<()> {
    let backends = resolve_store_backend_config_for_repo(repo_root)
        .context("resolving Bitloops store config for test-harness schema init")?;

    init_schema_for_backends(repo_root, backends)
}

fn init_schema_for_backends(
    repo_root: &Path,
    backends: crate::config::StoreBackendConfig,
) -> Result<()> {
    if backends.relational.has_postgres() {
        let dsn = backends.relational.postgres_dsn.ok_or_else(|| {
            anyhow!("test-harness schema init requires stores.relational.postgres_dsn")
        })?;
        let repository = PostgresTestHarnessRepository::connect(dsn)?;
        repository.initialise_schema()?;
        log::info!("Postgres test-harness schema initialized");
        Ok(())
    } else {
        let relational = DefaultRelationalStore::open_local_for_repo_root(repo_root)
            .context("opening local relational store for test-harness schema init")?;
        init_test_domain_database(relational.sqlite_path())
    }
}

pub fn open_repository_for_repo(repo_root: &Path) -> Result<BitloopsTestHarnessRepository> {
    let backends = resolve_store_backend_config_for_repo(repo_root)
        .context("resolving Bitloops store config for `bitloops devql test-harness`")?;

    open_repository_for_backends(repo_root, backends)
}

fn open_repository_for_backends(
    repo_root: &Path,
    backends: crate::config::StoreBackendConfig,
) -> Result<BitloopsTestHarnessRepository> {
    if backends.relational.has_postgres() {
        let dsn = backends.relational.postgres_dsn.ok_or_else(|| {
            anyhow!("`bitloops devql test-harness` requires stores.relational.postgres_dsn")
        })?;
        Ok(BitloopsTestHarnessRepository::Postgres(
            PostgresTestHarnessRepository::connect(dsn)?,
        ))
    } else {
        let relational = DefaultRelationalStore::open_local_for_repo_root(repo_root)
            .context("opening local relational store for `bitloops devql test-harness`")?;
        Ok(BitloopsTestHarnessRepository::Sqlite(
            SqliteTestHarnessRepository::open_existing(relational.sqlite_path())?,
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

    fn replace_test_discovery(
        &mut self,
        commit_sha: &str,
        test_artefacts: &[TestArtefactCurrentRecord],
        test_edges: &[TestArtefactEdgeCurrentRecord],
    ) -> Result<()> {
        match self {
            Self::Sqlite(repository) => {
                repository.replace_test_discovery(commit_sha, test_artefacts, test_edges)
            }
            Self::Postgres(repository) => {
                repository.replace_test_discovery(commit_sha, test_artefacts, test_edges)
            }
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
    use super::{
        BitloopsTestHarnessRepository, init_schema_for_backends, init_schema_for_repo,
        open_repository_for_backends, open_repository_for_repo,
    };
    use anyhow::Result;
    use serde_json::json;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;
    use toml_edit::{Array, DocumentMut, Item, Table, Value as TomlValue};

    fn json_value_to_toml_item(value: &serde_json::Value) -> Item {
        match value {
            serde_json::Value::Null => Item::Value(TomlValue::from("")),
            serde_json::Value::Bool(value) => Item::Value(TomlValue::from(*value)),
            serde_json::Value::Number(value) => {
                if let Some(value) = value.as_i64() {
                    Item::Value(TomlValue::from(value))
                } else if let Some(value) = value.as_u64() {
                    Item::Value(TomlValue::from(value as i64))
                } else if let Some(value) = value.as_f64() {
                    Item::Value(TomlValue::from(value))
                } else {
                    Item::Value(TomlValue::from(value.to_string()))
                }
            }
            serde_json::Value::String(value) => Item::Value(TomlValue::from(value.clone())),
            serde_json::Value::Array(values) => {
                let mut array = Array::new();
                for value in values {
                    let Item::Value(value) = json_value_to_toml_item(value) else {
                        panic!("nested TOML table arrays are not supported in test config");
                    };
                    array.push(value);
                }
                Item::Value(TomlValue::Array(array))
            }
            serde_json::Value::Object(map) => {
                let mut table = Table::new();
                for (key, value) in map {
                    table[key] = json_value_to_toml_item(value);
                }
                Item::Table(table)
            }
        }
    }

    fn write_repo_config(repo_root: &Path, value: serde_json::Value) -> Result<()> {
        let value = value.get("settings").cloned().unwrap_or(value);
        let mut doc = DocumentMut::new();
        let serde_json::Value::Object(map) = value else {
            panic!("expected object config value");
        };
        for (key, value) in map {
            doc[key.as_str()] = json_value_to_toml_item(&value);
        }
        fs::write(
            repo_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH),
            doc.to_string(),
        )?;
        Ok(())
    }

    #[test]
    fn init_schema_for_repo_initialises_sqlite_test_harness_tables() -> Result<()> {
        let temp = TempDir::new()?;
        let repo_root = temp.path();
        let sqlite_path = repo_root.join("stores").join("relational.sqlite");
        write_repo_config(
            repo_root,
            json!({
                "version": "1.0",
                "scope": "project",
                "settings": {
                    "stores": {
                        "relational": {
                            "sqlite_path": sqlite_path.display().to_string()
                        }
                    }
                }
            }),
        )?;

        init_schema_for_repo(repo_root)?;

        let conn = rusqlite::Connection::open(&sqlite_path)?;
        for table in [
            "test_artefacts_current",
            "test_artefact_edges_current",
            "coverage_captures",
            "coverage_hits",
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

    #[test]
    fn init_schema_for_repo_falls_back_to_sqlite_when_postgres_dsn_missing() -> Result<()> {
        let temp = TempDir::new()?;
        let repo_root = temp.path();
        let sqlite_path = repo_root.join("stores").join("relational.sqlite");
        write_repo_config(
            repo_root,
            json!({
                "version": "1.0",
                "scope": "project",
                "settings": {
                    "stores": {
                        "relational": {
                            "sqlite_path": sqlite_path.display().to_string(),
                            "postgres_dsn": "postgres://user:pass@localhost:5432/bitloops"
                        }
                    }
                }
            }),
        )?;

        let mut backends = crate::config::resolve_store_backend_config_for_repo(repo_root)?;
        assert!(backends.relational.has_postgres());
        backends.relational.postgres_dsn = None;

        init_schema_for_backends(repo_root, backends)?;
        assert!(sqlite_path.exists(), "expected sqlite fallback schema init");

        Ok(())
    }

    #[test]
    fn open_repository_for_repo_returns_sqlite_variant_when_sqlite_configured() -> Result<()> {
        let temp = TempDir::new()?;
        let repo_root = temp.path();
        let sqlite_path = repo_root.join("stores").join("relational.sqlite");
        write_repo_config(
            repo_root,
            json!({
                "version": "1.0",
                "scope": "project",
                "settings": {
                    "stores": {
                        "relational": {
                            "sqlite_path": sqlite_path.display().to_string()
                        }
                    }
                }
            }),
        )?;

        init_schema_for_repo(repo_root)?;
        let repository = open_repository_for_repo(repo_root)?;

        assert!(
            matches!(repository, BitloopsTestHarnessRepository::Sqlite(_)),
            "expected sqlite variant"
        );

        Ok(())
    }

    #[test]
    fn open_repository_for_backends_returns_postgres_variant_when_dsn_is_present() -> Result<()> {
        let temp = TempDir::new()?;
        let repo_root = temp.path();
        let sqlite_path = repo_root.join("stores").join("relational.sqlite");
        write_repo_config(
            repo_root,
            json!({
                "version": "1.0",
                "scope": "project",
                "settings": {
                    "stores": {
                        "relational": {
                            "sqlite_path": sqlite_path.display().to_string(),
                            "postgres_dsn": "postgres://127.0.0.1:1/bitloops"
                        }
                    }
                }
            }),
        )?;

        let backends = crate::config::resolve_store_backend_config_for_repo(repo_root)?;
        assert!(backends.relational.has_postgres());

        assert!(
            matches!(
                open_repository_for_backends(repo_root, backends)?,
                BitloopsTestHarnessRepository::Postgres(_)
            ),
            "expected postgres variant when dsn is configured"
        );
        Ok(())
    }
}
