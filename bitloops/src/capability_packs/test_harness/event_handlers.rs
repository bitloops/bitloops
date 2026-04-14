use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};
use serde_json::Value;

use crate::capability_packs::test_harness::mapping;
use crate::capability_packs::test_harness::mapping::linker::build_production_index;
use crate::capability_packs::test_harness::mapping::materialize::{
    MaterializationContext, materialize_enumerated_scenarios, materialize_source_discovery,
};
use crate::capability_packs::test_harness::mapping::model::StructuralMappingStats;
use crate::host::capability_host::{
    CurrentStateConsumer, CurrentStateConsumerContext, CurrentStateConsumerFuture,
    CurrentStateConsumerRequest, CurrentStateConsumerResult, ReconcileMode,
};
use crate::host::devql::{RelationalStorage, esc_pg};
use crate::host::language_adapter::{
    DiscoveredTestFile, EnumeratedTestScenario, LanguageAdapterContext,
};
use crate::models::{TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord};

use super::types::{TEST_HARNESS_CAPABILITY_ID, TEST_HARNESS_CURRENT_STATE_CONSUMER_ID};

pub struct TestHarnessCurrentStateConsumer;

impl CurrentStateConsumer for TestHarnessCurrentStateConsumer {
    fn capability_id(&self) -> &str {
        TEST_HARNESS_CAPABILITY_ID
    }

    fn consumer_id(&self) -> &str {
        TEST_HARNESS_CURRENT_STATE_CONSUMER_ID
    }

    fn reconcile<'a>(
        &'a self,
        request: &'a CurrentStateConsumerRequest,
        context: &'a CurrentStateConsumerContext,
    ) -> CurrentStateConsumerFuture<'a> {
        Box::pin(async move {
            match request.reconcile_mode {
                ReconcileMode::MergedDelta => reconcile_delta(request, context).await?,
                ReconcileMode::FullReconcile => reconcile_full(request, context).await?,
            }
            Ok(CurrentStateConsumerResult::applied(
                request.to_generation_seq_inclusive,
            ))
        })
    }
}

async fn reconcile_delta(
    request: &CurrentStateConsumerRequest,
    context: &CurrentStateConsumerContext,
) -> Result<()> {
    if requires_full_reconcile_for_delta(request) {
        return reconcile_full(request, context).await;
    }

    let mut discovered_files = Vec::new();
    let mut content_ids: HashMap<String, String> = HashMap::new();
    let mut processed_paths: HashSet<String> = HashSet::new();

    let supports = context.language_services.test_supports();
    for file in &request.file_upserts {
        let absolute_path = request.repo_root.join(&file.path);
        let support = context
            .language_services
            .resolve_test_support_for_path(&file.path)
            .or_else(|| {
                supports
                    .iter()
                    .find(|support| support.supports_path(&absolute_path, &file.path))
                    .cloned()
            });
        let Some(support) = support else {
            continue;
        };

        match support.discover_tests(&absolute_path, &file.path) {
            Ok(discovered) => {
                content_ids.insert(file.path.clone(), file.content_id.clone());
                processed_paths.insert(file.path.clone());
                discovered_files.push(discovered);
            }
            Err(err) => {
                log::warn!(
                    "test_harness current-state reconcile: failed discovering tests for {}: {err}",
                    file.path
                );
            }
        }
    }

    if !discovered_files.is_empty() || !processed_paths.is_empty() {
        let enumerated_scenarios = match enumerate_delta_scenarios(
            request,
            context,
            &discovered_files,
            &processed_paths,
        ) {
            DeltaEnumerationDecision::Incremental(scenarios) => scenarios,
            DeltaEnumerationDecision::RequiresFullReconcile => {
                return reconcile_full(request, context).await;
            }
        };
        let production = context
            .relational
            .load_current_production_artefacts(&request.repo_id)?;
        let production_index = build_production_index(&production);
        let mut test_artefacts = Vec::new();
        let mut test_edges = Vec::new();
        let mut link_keys = HashSet::new();
        let mut stats = StructuralMappingStats::default();

        let mut materialization = MaterializationContext {
            repo_id: &request.repo_id,
            content_ids: &content_ids,
            production: &production,
            production_index: &production_index,
            test_artefacts: &mut test_artefacts,
            test_edges: &mut test_edges,
            link_keys: &mut link_keys,
            stats: &mut stats,
        };
        materialize_source_discovery(&mut materialization, &discovered_files);
        materialize_enumerated_scenarios(&mut materialization, &enumerated_scenarios);

        persist_discovered_files(
            &context.storage,
            &request.repo_id,
            &processed_paths,
            &test_artefacts,
            &test_edges,
        )
        .await?;
    }

    if !request.file_removals.is_empty() {
        let removed_paths = request
            .file_removals
            .iter()
            .map(|file| file.path.clone())
            .collect::<HashSet<_>>();
        delete_paths(&context.storage, &request.repo_id, &removed_paths).await?;
    }

    if !request.artefact_removals.is_empty() {
        let removed_symbol_ids = request
            .artefact_removals
            .iter()
            .map(|artefact| artefact.symbol_id.clone())
            .collect::<Vec<_>>();
        delete_edges_to_removed_symbols(&context.storage, &request.repo_id, &removed_symbol_ids)
            .await?;
    }

    Ok(())
}

fn requires_full_reconcile_for_delta(request: &CurrentStateConsumerRequest) -> bool {
    !request.artefact_upserts.is_empty() || !request.artefact_removals.is_empty()
}

enum DeltaEnumerationDecision {
    Incremental(Vec<EnumeratedTestScenario>),
    RequiresFullReconcile,
}

fn enumerate_delta_scenarios(
    request: &CurrentStateConsumerRequest,
    context: &CurrentStateConsumerContext,
    discovered_files: &[DiscoveredTestFile],
    processed_paths: &HashSet<String>,
) -> DeltaEnumerationDecision {
    if discovered_files.is_empty() || processed_paths.is_empty() {
        return DeltaEnumerationDecision::Incremental(Vec::new());
    }

    let language_context = LanguageAdapterContext::new(
        request.repo_root.clone(),
        request.repo_id.clone(),
        request.head_commit_sha.clone(),
    );
    let mut enumerated = Vec::new();

    for support in context.language_services.test_supports() {
        let source_files = discovered_files
            .iter()
            .filter(|file| file.language == support.language_id())
            .cloned()
            .collect::<Vec<_>>();
        if source_files.is_empty() {
            continue;
        }

        let enumeration = support.enumerate_tests(&language_context);
        let reconciled = support.reconcile(&source_files, enumeration);
        if reconciled
            .enumerated_scenarios
            .iter()
            .any(|scenario| scenario.relative_path.starts_with("__synthetic_tests__/"))
        {
            return DeltaEnumerationDecision::RequiresFullReconcile;
        }
        enumerated.extend(
            reconciled
                .enumerated_scenarios
                .into_iter()
                .filter(|scenario| processed_paths.contains(&scenario.relative_path)),
        );
    }

    DeltaEnumerationDecision::Incremental(enumerated)
}

async fn reconcile_full(
    request: &CurrentStateConsumerRequest,
    context: &CurrentStateConsumerContext,
) -> Result<()> {
    let production = context
        .relational
        .load_current_production_artefacts(&request.repo_id)?;
    let mapping = mapping::execute(
        &request.repo_id,
        &request.repo_root,
        request.head_commit_sha.as_deref().unwrap_or("current"),
        &production,
        context.language_services.as_ref(),
    )?;

    replace_repo_state(
        &context.storage,
        &request.repo_id,
        &mapping.test_artefacts,
        &mapping.test_edges,
    )
    .await
}

async fn replace_repo_state(
    storage: &RelationalStorage,
    repo_id: &str,
    test_artefacts: &[TestArtefactCurrentRecord],
    test_edges: &[TestArtefactEdgeCurrentRecord],
) -> Result<()> {
    ensure_unique_test_artefact_ids(test_artefacts)?;
    let mut statements = vec![
        delete_repo_test_edges_sql(repo_id),
        delete_repo_test_artefacts_sql(repo_id),
    ];
    statements.extend(
        test_artefacts
            .iter()
            .map(|artefact| insert_test_artefact_sql(storage, artefact)),
    );
    statements.extend(
        test_edges
            .iter()
            .map(|edge| insert_test_edge_sql(storage, edge)),
    );
    storage.exec_batch_transactional(&statements).await
}

async fn persist_discovered_files(
    storage: &RelationalStorage,
    repo_id: &str,
    processed_paths: &HashSet<String>,
    test_artefacts: &[TestArtefactCurrentRecord],
    test_edges: &[TestArtefactEdgeCurrentRecord],
) -> Result<()> {
    ensure_unique_test_artefact_ids(test_artefacts)?;
    let mut statements = Vec::new();
    for path in processed_paths {
        statements.push(delete_test_edges_for_path_sql(repo_id, path));
        statements.push(delete_test_artefacts_for_path_sql(repo_id, path));
    }
    statements.extend(
        test_artefacts
            .iter()
            .map(|artefact| insert_test_artefact_sql(storage, artefact)),
    );
    statements.extend(
        test_edges
            .iter()
            .map(|edge| insert_test_edge_sql(storage, edge)),
    );
    storage.exec_batch_transactional(&statements).await
}

fn ensure_unique_test_artefact_ids(test_artefacts: &[TestArtefactCurrentRecord]) -> Result<()> {
    let mut by_artefact_id: HashMap<&str, Vec<&TestArtefactCurrentRecord>> = HashMap::new();
    for artefact in test_artefacts {
        by_artefact_id
            .entry(artefact.artefact_id.as_str())
            .or_default()
            .push(artefact);
    }

    let duplicates = by_artefact_id
        .into_iter()
        .filter_map(|(artefact_id, artefacts)| {
            (artefacts.len() > 1).then(|| {
                let details = artefacts
                    .iter()
                    .map(|artefact| {
                        format!(
                            "path={}, kind={}, name={}, discovery_source={}",
                            artefact.path,
                            artefact.canonical_kind,
                            artefact.name,
                            artefact.discovery_source
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" ; ");
                format!("{artefact_id} => {details}")
            })
        })
        .take(5)
        .collect::<Vec<_>>();

    if !duplicates.is_empty() {
        bail!(
            "duplicate test artefact ids detected before persistence: {}",
            duplicates.join(" | ")
        );
    }

    Ok(())
}

async fn delete_paths(
    storage: &RelationalStorage,
    repo_id: &str,
    paths: &HashSet<String>,
) -> Result<()> {
    let mut statements = Vec::new();
    for path in paths {
        statements.push(delete_test_edges_for_path_sql(repo_id, path));
        statements.push(delete_test_artefacts_for_path_sql(repo_id, path));
    }
    storage.exec_batch_transactional(&statements).await
}

async fn delete_edges_to_removed_symbols(
    storage: &RelationalStorage,
    repo_id: &str,
    symbol_ids: &[String],
) -> Result<()> {
    if symbol_ids.is_empty() {
        return Ok(());
    }
    let in_list = symbol_ids
        .iter()
        .map(|symbol_id| format!("'{}'", esc_pg(symbol_id)))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "DELETE FROM test_artefact_edges_current \
         WHERE repo_id = '{}' AND to_symbol_id IN ({})",
        esc_pg(repo_id),
        in_list
    );
    storage.exec(&sql).await
}

fn delete_repo_test_artefacts_sql(repo_id: &str) -> String {
    format!(
        "DELETE FROM test_artefacts_current WHERE repo_id = '{}'",
        esc_pg(repo_id)
    )
}

fn delete_repo_test_edges_sql(repo_id: &str) -> String {
    format!(
        "DELETE FROM test_artefact_edges_current WHERE repo_id = '{}'",
        esc_pg(repo_id)
    )
}

fn delete_test_artefacts_for_path_sql(repo_id: &str, path: &str) -> String {
    format!(
        "DELETE FROM test_artefacts_current WHERE repo_id = '{}' AND path = '{}'",
        esc_pg(repo_id),
        esc_pg(path)
    )
}

fn delete_test_edges_for_path_sql(repo_id: &str, path: &str) -> String {
    format!(
        "DELETE FROM test_artefact_edges_current WHERE repo_id = '{}' AND path = '{}'",
        esc_pg(repo_id),
        esc_pg(path)
    )
}

fn insert_test_artefact_sql(
    storage: &RelationalStorage,
    artefact: &TestArtefactCurrentRecord,
) -> String {
    let language_kind_sql = nullable_text_sql(artefact.language_kind.as_deref());
    let symbol_fqn_sql = nullable_text_sql(artefact.symbol_fqn.as_deref());
    let parent_symbol_id_sql = nullable_text_sql(artefact.parent_symbol_id.as_deref());
    let parent_artefact_id_sql = nullable_text_sql(artefact.parent_artefact_id.as_deref());
    let start_byte_sql = nullable_i64_sql(artefact.start_byte);
    let end_byte_sql = nullable_i64_sql(artefact.end_byte);
    let signature_sql = nullable_text_sql(artefact.signature.as_deref());
    let docstring_sql = nullable_text_sql(artefact.docstring.as_deref());
    let modifiers_sql = crate::host::devql::sql_json_value(
        storage,
        &serde_json::from_str(&artefact.modifiers).unwrap_or(Value::Array(Vec::new())),
    );

    format!(
        "INSERT INTO test_artefacts_current \
         (repo_id, path, content_id, symbol_id, artefact_id, language, canonical_kind, language_kind, symbol_fqn, name, parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, discovery_source, updated_at) \
         VALUES ('{}', '{}', '{}', '{}', '{}', '{}', '{}', {}, {}, '{}', {}, {}, {}, {}, {}, {}, {}, {}, {}, '{}', datetime('now'))",
        esc_pg(&artefact.repo_id),
        esc_pg(&artefact.path),
        esc_pg(&artefact.content_id),
        esc_pg(&artefact.symbol_id),
        esc_pg(&artefact.artefact_id),
        esc_pg(&artefact.language),
        esc_pg(&artefact.canonical_kind),
        language_kind_sql,
        symbol_fqn_sql,
        esc_pg(&artefact.name),
        parent_symbol_id_sql,
        parent_artefact_id_sql,
        artefact.start_line,
        artefact.end_line,
        start_byte_sql,
        end_byte_sql,
        signature_sql,
        modifiers_sql,
        docstring_sql,
        esc_pg(&artefact.discovery_source),
    )
}

fn insert_test_edge_sql(
    storage: &RelationalStorage,
    edge: &TestArtefactEdgeCurrentRecord,
) -> String {
    let to_artefact_id_sql = nullable_text_sql(edge.to_artefact_id.as_deref());
    let to_symbol_id_sql = nullable_text_sql(edge.to_symbol_id.as_deref());
    let to_symbol_ref_sql = nullable_text_sql(edge.to_symbol_ref.as_deref());
    let start_line_sql = nullable_i64_sql(edge.start_line);
    let end_line_sql = nullable_i64_sql(edge.end_line);
    let metadata_sql = crate::host::devql::sql_json_value(
        storage,
        &serde_json::from_str(&edge.metadata).unwrap_or(Value::Object(serde_json::Map::new())),
    );

    format!(
        "INSERT INTO test_artefact_edges_current \
         (repo_id, path, content_id, edge_id, from_artefact_id, from_symbol_id, to_artefact_id, to_symbol_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata, updated_at) \
         VALUES ('{}', '{}', '{}', '{}', '{}', '{}', {}, {}, {}, '{}', '{}', {}, {}, {}, datetime('now'))",
        esc_pg(&edge.repo_id),
        esc_pg(&edge.path),
        esc_pg(&edge.content_id),
        esc_pg(&edge.edge_id),
        esc_pg(&edge.from_artefact_id),
        esc_pg(&edge.from_symbol_id),
        to_artefact_id_sql,
        to_symbol_id_sql,
        to_symbol_ref_sql,
        esc_pg(&edge.edge_kind),
        esc_pg(&edge.language),
        start_line_sql,
        end_line_sql,
        metadata_sql,
    )
}

fn nullable_text_sql(value: Option<&str>) -> String {
    value
        .map(|value| format!("'{}'", esc_pg(value)))
        .unwrap_or_else(|| "NULL".to_string())
}

fn nullable_i64_sql(value: Option<i64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NULL".to_string())
}

#[cfg(test)]
mod tests {
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
        DefaultHostServicesGateway, HostServicesGateway, LanguageServicesGateway,
        RelationalGateway,
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

        fn load_current_production_artefacts(
            &self,
            _repo_id: &str,
        ) -> Result<Vec<ProductionArtefact>> {
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
            discovered_by_path: HashMap::from([(
                "tests/changed.fake".to_string(),
                changed_discovery,
            )]),
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
    async fn merged_delta_promotes_to_full_reconcile_when_production_artefacts_change() -> Result<()>
    {
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
            repo_id: "repo-1".to_string(),
            repo_root: fixture.repo_root(),
            active_branch: Some("main".to_string()),
            head_commit_sha: Some("HEAD".to_string()),
            from_generation_seq_exclusive: 1,
            to_generation_seq_inclusive: 2,
            reconcile_mode: ReconcileMode::MergedDelta,
            file_upserts: Vec::new(),
            file_removals: Vec::new(),
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
                host_services: Arc::new(DefaultHostServicesGateway::new("repo-1"))
                    as Arc<dyn HostServicesGateway>,
                workplane: Arc::new(NoopWorkplaneGateway),
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
}
