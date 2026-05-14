use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use rusqlite::Connection;
use serde_json::json;
use tempfile::TempDir;

use super::{TestHarnessCurrentStateConsumer, ensure_unique_test_artefact_ids};
use crate::capability_packs::test_harness::storage::init_test_domain_database;
use crate::host::capability_host::gateways::{
    CapabilityMailboxStatus, CapabilityWorkplaneEnqueueResult, CapabilityWorkplaneGateway,
    DefaultHostServicesGateway, HostServicesGateway, LanguageServicesGateway, RelationalGateway,
};
use crate::host::capability_host::{
    ChangedArtefact, ChangedFile, CurrentStateConsumer, CurrentStateConsumerContext,
    CurrentStateConsumerRequest, ReconcileMode,
};
use crate::host::devql::RelationalStorage;
use crate::host::language_adapter::{
    DiscoveredTestFile, DiscoveredTestScenario, DiscoveredTestSuite, EnumeratedTestScenario,
    EnumerationMode, EnumerationResult, LanguageAdapterContext, LanguageTestSupport,
    ReconciledDiscovery, ReferenceCandidate, ScenarioDiscoverySource,
};
use crate::models::ProductionArtefact;
use crate::models::TestArtefactCurrentRecord;

#[derive(Default)]
struct NoopWorkplaneGateway;

impl CapabilityWorkplaneGateway for NoopWorkplaneGateway {
    fn enqueue_jobs(
        &self,
        _jobs: Vec<crate::host::capability_host::gateways::CapabilityWorkplaneJob>,
    ) -> Result<CapabilityWorkplaneEnqueueResult> {
        Ok(CapabilityWorkplaneEnqueueResult::default())
    }

    fn mailbox_status(&self) -> Result<BTreeMap<String, CapabilityMailboxStatus>> {
        Ok(BTreeMap::new())
    }
}

#[derive(Clone)]
struct FakeRelationalGateway {
    production: Vec<ProductionArtefact>,
}

impl RelationalGateway for FakeRelationalGateway {
    fn resolve_checkpoint_id(&self, _repo_id: &str, _checkpoint_ref: &str) -> Result<String> {
        bail!("resolve_checkpoint_id is not used in test_harness event handler tests")
    }

    fn artefact_exists(&self, _repo_id: &str, _artefact_id: &str) -> Result<bool> {
        Ok(false)
    }

    fn load_repo_id_for_commit(&self, _commit_sha: &str) -> Result<String> {
        bail!("load_repo_id_for_commit is not used in test_harness event handler tests")
    }

    fn load_current_production_artefacts(&self, _repo_id: &str) -> Result<Vec<ProductionArtefact>> {
        Ok(self.production.clone())
    }

    fn load_production_artefacts(&self, _commit_sha: &str) -> Result<Vec<ProductionArtefact>> {
        Ok(self.production.clone())
    }

    fn load_artefacts_for_file_lines(
        &self,
        _commit_sha: &str,
        _file_path: &str,
    ) -> Result<Vec<(String, i64, i64)>> {
        Ok(Vec::new())
    }
}

#[derive(Clone)]
struct FakeLanguageServicesGateway {
    support: Arc<FakeLanguageTestSupport>,
}

impl LanguageServicesGateway for FakeLanguageServicesGateway {
    fn test_supports(&self) -> Vec<Arc<dyn LanguageTestSupport>> {
        vec![self.support.clone()]
    }

    fn resolve_test_support_for_path(
        &self,
        relative_path: &str,
    ) -> Option<Arc<dyn LanguageTestSupport>> {
        self.support
            .supports_any(relative_path)
            .then(|| self.support.clone() as Arc<dyn LanguageTestSupport>)
    }
}

#[derive(Clone)]
struct FakeLanguageTestSupport {
    language_id: &'static str,
    discovered_by_path: HashMap<String, DiscoveredTestFile>,
    enumeration: EnumerationResult,
}

impl FakeLanguageTestSupport {
    fn supports_any(&self, relative_path: &str) -> bool {
        self.discovered_by_path.contains_key(relative_path)
    }
}

impl LanguageTestSupport for FakeLanguageTestSupport {
    fn language_id(&self) -> &'static str {
        self.language_id
    }

    fn priority(&self) -> u8 {
        0
    }

    fn supports_path(&self, _absolute_path: &Path, relative_path: &str) -> bool {
        self.supports_any(relative_path)
    }

    fn discover_tests(
        &self,
        _absolute_path: &Path,
        relative_path: &str,
    ) -> Result<DiscoveredTestFile> {
        self.discovered_by_path
            .get(relative_path)
            .cloned()
            .ok_or_else(|| anyhow!("unexpected test discovery request for {relative_path}"))
    }

    fn enumerate_tests(&self, _ctx: &LanguageAdapterContext) -> EnumerationResult {
        self.enumeration.clone()
    }

    fn reconcile(
        &self,
        _source_files: &[DiscoveredTestFile],
        enumeration: EnumerationResult,
    ) -> ReconciledDiscovery {
        ReconciledDiscovery {
            enumerated_scenarios: enumeration.scenarios,
        }
    }
}

#[tokio::test]
async fn merged_delta_materializes_enumerated_scenarios_only_for_changed_paths() -> Result<()> {
    let fixture = TestFixture::new()?;
    fixture.write_file("tests/changed.fake", "changed")?;

    let changed_discovery = DiscoveredTestFile {
        relative_path: "tests/changed.fake".to_string(),
        language: "fake".to_string(),
        reference_candidates: Vec::new(),
        suites: Vec::new(),
    };
    let unchanged_discovery = DiscoveredTestFile {
        relative_path: "tests/unchanged.fake".to_string(),
        language: "fake".to_string(),
        reference_candidates: Vec::new(),
        suites: Vec::new(),
    };

    let support = Arc::new(FakeLanguageTestSupport {
        language_id: "fake",
        discovered_by_path: HashMap::from([
            ("tests/changed.fake".to_string(), changed_discovery),
            ("tests/unchanged.fake".to_string(), unchanged_discovery),
        ]),
        enumeration: EnumerationResult {
            mode: EnumerationMode::Full,
            scenarios: vec![
                enumerated_scenario("tests/changed.fake", "generated_changed_case"),
                enumerated_scenario("tests/unchanged.fake", "generated_unchanged_case"),
            ],
            notes: Vec::new(),
        },
    });
    let context = fixture.context(support, vec![production_artefact()])?;
    let request = CurrentStateConsumerRequest {
        run_id: None,
        repo_id: "repo-1".to_string(),
        repo_root: fixture.repo_root(),
        active_branch: Some("main".to_string()),
        head_commit_sha: Some("HEAD".to_string()),
        from_generation_seq_exclusive: 0,
        to_generation_seq_inclusive: 1,
        reconcile_mode: ReconcileMode::MergedDelta,
        file_upserts: vec![ChangedFile {
            path: "tests/changed.fake".to_string(),
            language: "fake".to_string(),
            content_id: "changed-content".to_string(),
        }],
        file_removals: Vec::new(),
        affected_paths: Vec::new(),
        artefact_upserts: Vec::new(),
        artefact_removals: Vec::new(),
    };

    TestHarnessCurrentStateConsumer
        .reconcile(&request, &context)
        .await?;

    assert_eq!(
        load_test_scenarios(fixture.db_path())?,
        vec![(
            "tests/changed.fake".to_string(),
            "generated_changed_case".to_string(),
            "enumeration".to_string(),
        )]
    );
    assert_eq!(count_test_edges(fixture.db_path())?, 1);

    Ok(())
}

#[tokio::test]
async fn merged_delta_materializes_synthetic_enumerated_scenarios() -> Result<()> {
    let fixture = TestFixture::new()?;
    fixture.write_file("tests/changed.fake", "changed")?;

    let changed_discovery = DiscoveredTestFile {
        relative_path: "tests/changed.fake".to_string(),
        language: "fake".to_string(),
        reference_candidates: Vec::new(),
        suites: Vec::new(),
    };

    let support = Arc::new(FakeLanguageTestSupport {
        language_id: "fake",
        discovered_by_path: HashMap::from([("tests/changed.fake".to_string(), changed_discovery)]),
        enumeration: EnumerationResult {
            mode: EnumerationMode::Full,
            scenarios: vec![enumerated_scenario(
                "__synthetic_tests__/target_debug_deps_changed",
                "generated_changed_case",
            )],
            notes: Vec::new(),
        },
    });
    let context = fixture.context(support, vec![production_artefact()])?;
    let request = CurrentStateConsumerRequest {
        run_id: None,
        repo_id: "repo-1".to_string(),
        repo_root: fixture.repo_root(),
        active_branch: Some("main".to_string()),
        head_commit_sha: Some("HEAD".to_string()),
        from_generation_seq_exclusive: 0,
        to_generation_seq_inclusive: 1,
        reconcile_mode: ReconcileMode::MergedDelta,
        file_upserts: vec![ChangedFile {
            path: "tests/changed.fake".to_string(),
            language: "fake".to_string(),
            content_id: "changed-content".to_string(),
        }],
        file_removals: Vec::new(),
        affected_paths: Vec::new(),
        artefact_upserts: Vec::new(),
        artefact_removals: Vec::new(),
    };

    TestHarnessCurrentStateConsumer
        .reconcile(&request, &context)
        .await?;

    assert_eq!(
        load_test_scenarios(fixture.db_path())?,
        vec![(
            "__synthetic_tests__/target_debug_deps_changed".to_string(),
            "generated_changed_case".to_string(),
            "enumeration".to_string(),
        )]
    );
    assert_eq!(count_test_edges(fixture.db_path())?, 1);

    Ok(())
}

#[tokio::test]
async fn merged_delta_promotes_to_full_reconcile_when_production_artefacts_change() -> Result<()> {
    let fixture = TestFixture::new()?;
    fixture.write_file("tests/cases.fake", "cases")?;

    let source_discovery = DiscoveredTestFile {
        relative_path: "tests/cases.fake".to_string(),
        language: "fake".to_string(),
        reference_candidates: Vec::new(),
        suites: vec![DiscoveredTestSuite {
            name: "suite".to_string(),
            start_line: 1,
            end_line: 3,
            scenarios: vec![DiscoveredTestScenario {
                name: "source_case".to_string(),
                start_line: 2,
                end_line: 2,
                reference_candidates: vec![ReferenceCandidate::ExplicitTarget {
                    path: "src/prod.fake".to_string(),
                    start_line: 10,
                }],
                discovery_source: ScenarioDiscoverySource::Source,
            }],
        }],
    };

    let support = Arc::new(FakeLanguageTestSupport {
        language_id: "fake",
        discovered_by_path: HashMap::from([("tests/cases.fake".to_string(), source_discovery)]),
        enumeration: EnumerationResult::default(),
    });
    let context = fixture.context(support, vec![production_artefact()])?;
    let request = CurrentStateConsumerRequest {
        run_id: None,
        repo_id: "repo-1".to_string(),
        repo_root: fixture.repo_root(),
        active_branch: Some("main".to_string()),
        head_commit_sha: Some("HEAD".to_string()),
        from_generation_seq_exclusive: 1,
        to_generation_seq_inclusive: 2,
        reconcile_mode: ReconcileMode::MergedDelta,
        file_upserts: Vec::new(),
        file_removals: Vec::new(),
        affected_paths: Vec::new(),
        artefact_upserts: vec![ChangedArtefact {
            artefact_id: "prod-artefact".to_string(),
            symbol_id: "prod-symbol".to_string(),
            path: "src/prod.fake".to_string(),
            canonical_kind: Some("function".to_string()),
            name: "foo".to_string(),
        }],
        artefact_removals: Vec::new(),
    };

    TestHarnessCurrentStateConsumer
        .reconcile(&request, &context)
        .await?;

    assert_eq!(
        load_test_scenarios(fixture.db_path())?,
        vec![(
            "tests/cases.fake".to_string(),
            "source_case".to_string(),
            "source".to_string(),
        )]
    );
    assert_eq!(count_test_edges(fixture.db_path())?, 1);

    Ok(())
}

#[test]
fn duplicate_test_artefact_ids_are_reported_before_sqlite_insert() {
    let duplicate_a = TestArtefactCurrentRecord {
        artefact_id: "duplicate-artefact-id".to_string(),
        symbol_id: "symbol-a".to_string(),
        repo_id: "repo-1".to_string(),
        content_id: "content-a".to_string(),
        path: "tests/a.rs".to_string(),
        language: "rust".to_string(),
        canonical_kind: "test_scenario".to_string(),
        language_kind: None,
        symbol_fqn: Some("suite.a".to_string()),
        name: "a".to_string(),
        parent_artefact_id: None,
        parent_symbol_id: None,
        start_line: 10,
        end_line: 10,
        start_byte: None,
        end_byte: None,
        signature: Some("a".to_string()),
        modifiers: "[]".to_string(),
        docstring: None,
        discovery_source: "enumeration".to_string(),
    };
    let duplicate_b = TestArtefactCurrentRecord {
        artefact_id: "duplicate-artefact-id".to_string(),
        symbol_id: "symbol-b".to_string(),
        repo_id: "repo-1".to_string(),
        content_id: "content-b".to_string(),
        path: "tests/b.rs".to_string(),
        language: "rust".to_string(),
        canonical_kind: "test_scenario".to_string(),
        language_kind: None,
        symbol_fqn: Some("suite.b".to_string()),
        name: "b".to_string(),
        parent_artefact_id: None,
        parent_symbol_id: None,
        start_line: 20,
        end_line: 20,
        start_byte: None,
        end_byte: None,
        signature: Some("b".to_string()),
        modifiers: "[]".to_string(),
        docstring: None,
        discovery_source: "source".to_string(),
    };

    let error = ensure_unique_test_artefact_ids(&[duplicate_a, duplicate_b]).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("duplicate test artefact ids detected before persistence"));
    assert!(message.contains("duplicate-artefact-id"));
    assert!(message.contains("tests/a.rs"));
    assert!(message.contains("tests/b.rs"));
}

struct TestFixture {
    temp: TempDir,
    db_path: PathBuf,
}

impl TestFixture {
    fn new() -> Result<Self> {
        let temp = TempDir::new()?;
        let db_path = temp.path().join("stores").join("relational.sqlite");
        init_test_domain_database(&db_path)?;
        Ok(Self { temp, db_path })
    }

    fn repo_root(&self) -> PathBuf {
        self.temp.path().to_path_buf()
    }

    fn db_path(&self) -> &Path {
        &self.db_path
    }

    fn write_file(&self, relative_path: &str, contents: &str) -> Result<()> {
        let absolute_path = self.temp.path().join(relative_path);
        if let Some(parent) = absolute_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(absolute_path, contents)?;
        Ok(())
    }

    fn context(
        &self,
        support: Arc<FakeLanguageTestSupport>,
        production: Vec<ProductionArtefact>,
    ) -> Result<CurrentStateConsumerContext> {
        Ok(CurrentStateConsumerContext {
            config_root: json!({}),
            storage: Arc::new(RelationalStorage::local_only(self.db_path.clone())),
            relational: Arc::new(FakeRelationalGateway { production }),
            language_services: Arc::new(FakeLanguageServicesGateway { support }),
            git_history: Arc::new(crate::host::capability_host::gateways::EmptyGitHistoryGateway),
            inference: Arc::new(crate::host::inference::EmptyInferenceGateway),
            host_services: Arc::new(DefaultHostServicesGateway::new("repo-1"))
                as Arc<dyn HostServicesGateway>,
            workplane: Arc::new(NoopWorkplaneGateway),
            test_harness: None,
            init_session_id: None,
            parent_pid: None,
        })
    }
}

fn production_artefact() -> ProductionArtefact {
    ProductionArtefact {
        artefact_id: "prod-artefact".to_string(),
        symbol_id: "prod-symbol".to_string(),
        symbol_fqn: "crate::foo".to_string(),
        path: "src/prod.fake".to_string(),
        start_line: 10,
    }
}

fn enumerated_scenario(relative_path: &str, scenario_name: &str) -> EnumeratedTestScenario {
    EnumeratedTestScenario {
        language: "fake".to_string(),
        suite_name: "generated_suite".to_string(),
        scenario_name: scenario_name.to_string(),
        relative_path: relative_path.to_string(),
        start_line: 1,
        reference_candidates: vec![ReferenceCandidate::ExplicitTarget {
            path: "src/prod.fake".to_string(),
            start_line: 10,
        }],
        discovery_source: ScenarioDiscoverySource::Enumeration,
    }
}

fn load_test_scenarios(db_path: &Path) -> Result<Vec<(String, String, String)>> {
    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare(
        "SELECT path, name, discovery_source
         FROM test_artefacts_current
         WHERE canonical_kind = 'test_scenario'
         ORDER BY path, name",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    let mut scenarios = Vec::new();
    for row in rows {
        scenarios.push(row?);
    }
    Ok(scenarios)
}

fn count_test_edges(db_path: &Path) -> Result<i64> {
    let conn = Connection::open(db_path)?;
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM test_artefact_edges_current",
        [],
        |row| row.get(0),
    )?)
}
