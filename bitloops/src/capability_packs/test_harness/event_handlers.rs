use anyhow::Result;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use crate::capability_packs::test_harness::mapping::linker::build_production_index;
use crate::capability_packs::test_harness::mapping::materialize::{
    MaterializationContext, materialize_source_discovery,
};
use crate::capability_packs::test_harness::mapping::model::StructuralMappingStats;
use crate::host::capability_host::{
    EventHandlerContext, EventHandlerFuture, HostEvent, HostEventHandler, HostEventKind,
};
use crate::host::devql::{RelationalStorage, esc_pg};
use crate::models::{TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord};

use super::types::TEST_HARNESS_CAPABILITY_ID;

pub struct TestHarnessSyncHandler;

impl HostEventHandler for TestHarnessSyncHandler {
    fn event_kind(&self) -> HostEventKind {
        HostEventKind::SyncCompleted
    }

    fn capability_id(&self) -> &str {
        TEST_HARNESS_CAPABILITY_ID
    }

    fn handle<'a>(
        &'a self,
        event: &'a HostEvent,
        context: &'a EventHandlerContext,
    ) -> EventHandlerFuture<'a> {
        Box::pin(async move {
            let HostEvent::SyncCompleted(payload) = event;
            log::info!(
                "test_harness sync event received (repo_id={}, mode={}, branch={}, files_added={}, files_changed={}, files_removed={}, artefacts_added={}, artefacts_changed={}, artefacts_removed={})",
                payload.repo_id,
                payload.sync_mode,
                payload.active_branch.as_deref().unwrap_or("unknown"),
                payload.files.added.len(),
                payload.files.changed.len(),
                payload.files.removed.len(),
                payload.artefacts.added.len(),
                payload.artefacts.changed.len(),
                payload.artefacts.removed.len()
            );
            handle_sync_completed(payload, context).await?;
            Ok(())
        })
    }
}

async fn handle_sync_completed(
    payload: &crate::host::capability_host::SyncCompletedPayload,
    context: &EventHandlerContext,
) -> Result<()> {
    let mut discovered_files = Vec::new();
    let mut content_ids: HashMap<String, String> = HashMap::new();
    let mut processed_paths: HashSet<String> = HashSet::new();

    let supports = context.language_services.test_supports();
    for file in payload
        .files
        .added
        .iter()
        .chain(payload.files.changed.iter())
    {
        let absolute_path = payload.repo_root.join(&file.path);
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
                    "test_harness sync handler: failed discovering tests for path {}: {err}",
                    file.path
                );
            }
        }
    }

    if !discovered_files.is_empty() || !processed_paths.is_empty() {
        let production = context
            .relational
            .load_current_production_artefacts(&payload.repo_id)?;
        let production_index = build_production_index(&production);
        let mut test_artefacts = Vec::new();
        let mut test_edges = Vec::new();
        let mut link_keys = HashSet::new();
        let mut stats = StructuralMappingStats::default();

        let mut materialization = MaterializationContext {
            repo_id: &payload.repo_id,
            content_ids: &content_ids,
            production: &production,
            production_index: &production_index,
            test_artefacts: &mut test_artefacts,
            test_edges: &mut test_edges,
            link_keys: &mut link_keys,
            stats: &mut stats,
        };
        materialize_source_discovery(&mut materialization, &discovered_files);

        persist_discovered_files(
            &context.storage,
            &payload.repo_id,
            &processed_paths,
            &test_artefacts,
            &test_edges,
        )
        .await?;
    }

    if !payload.files.removed.is_empty() {
        let removed_paths: HashSet<String> = payload
            .files
            .removed
            .iter()
            .map(|file| file.path.clone())
            .collect();
        delete_paths(&context.storage, &payload.repo_id, &removed_paths).await?;
    }

    if !payload.artefacts.removed.is_empty() {
        let removed_symbol_ids: Vec<String> = payload
            .artefacts
            .removed
            .iter()
            .map(|artefact| artefact.symbol_id.clone())
            .collect();
        delete_edges_to_removed_symbols(&context.storage, &payload.repo_id, &removed_symbol_ids)
            .await?;
    }

    Ok(())
}

async fn persist_discovered_files(
    storage: &RelationalStorage,
    repo_id: &str,
    processed_paths: &HashSet<String>,
    test_artefacts: &[TestArtefactCurrentRecord],
    test_edges: &[TestArtefactEdgeCurrentRecord],
) -> Result<()> {
    let mut statements = Vec::new();
    for path in processed_paths {
        statements.push(delete_test_edges_for_path_sql(repo_id, path));
        statements.push(delete_test_artefacts_for_path_sql(repo_id, path));
    }
    for artefact in test_artefacts {
        statements.push(insert_test_artefact_sql(storage, artefact));
    }
    for edge in test_edges {
        statements.push(insert_test_edge_sql(storage, edge));
    }
    storage.exec_batch_transactional(&statements).await
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
    use super::*;
    use std::path::Path;
    use std::sync::Arc;
    use tempfile::TempDir;

    use crate::capability_packs::test_harness::storage::init_test_domain_database;
    use crate::host::capability_host::gateways::{
        HostServicesGateway, LanguageServicesGateway, SymbolIdentityInput,
    };
    use crate::host::capability_host::{
        ChangedArtefact, ChangedFile, RemovedArtefact, RemovedFile, SyncArtefactDiff,
        SyncCompletedPayload, SyncFileDiff,
    };
    use crate::host::language_adapter::{
        DiscoveredTestFile, DiscoveredTestScenario, DiscoveredTestSuite, LanguageTestSupport,
        ReferenceCandidate, ScenarioDiscoverySource,
    };

    struct FakeTestSupport;

    impl LanguageTestSupport for FakeTestSupport {
        fn language_id(&self) -> &'static str {
            "rust"
        }

        fn priority(&self) -> u8 {
            1
        }

        fn supports_path(&self, _absolute_path: &Path, relative_path: &str) -> bool {
            relative_path.ends_with("_test.rs")
        }

        fn discover_tests(
            &self,
            _absolute_path: &Path,
            relative_path: &str,
        ) -> Result<DiscoveredTestFile> {
            Ok(DiscoveredTestFile {
                relative_path: relative_path.to_string(),
                language: "rust".to_string(),
                reference_candidates: vec![ReferenceCandidate::SourcePath(
                    "src/user/service.rs".to_string(),
                )],
                suites: vec![DiscoveredTestSuite {
                    name: "user_service_tests".to_string(),
                    start_line: 1,
                    end_line: 20,
                    scenarios: vec![DiscoveredTestScenario {
                        name: "test_create_user".to_string(),
                        start_line: 3,
                        end_line: 10,
                        reference_candidates: vec![ReferenceCandidate::SymbolName(
                            "create_user".to_string(),
                        )],
                        discovery_source: ScenarioDiscoverySource::Source,
                    }],
                }],
            })
        }
    }

    struct FakeLanguageGateway {
        support: Arc<dyn LanguageTestSupport>,
    }

    impl LanguageServicesGateway for FakeLanguageGateway {
        fn test_supports(&self) -> Vec<Arc<dyn LanguageTestSupport>> {
            vec![self.support.clone()]
        }

        fn resolve_test_support_for_path(
            &self,
            relative_path: &str,
        ) -> Option<Arc<dyn LanguageTestSupport>> {
            if relative_path.ends_with("_test.rs") {
                Some(self.support.clone())
            } else {
                None
            }
        }
    }

    struct NoopHostServices;

    impl HostServicesGateway for NoopHostServices {
        fn derive_symbol_id(&self, _input: &SymbolIdentityInput<'_>) -> String {
            String::new()
        }

        fn derive_artefact_id(&self, _content_id: &str, _symbol_id: &str) -> String {
            String::new()
        }

        fn derive_edge_id(
            &self,
            _repo_id: &str,
            _from_symbol_id: &str,
            _edge_kind: &str,
            _to_symbol_id_or_ref: &str,
        ) -> String {
            String::new()
        }
    }

    fn test_event() -> HostEvent {
        HostEvent::SyncCompleted(SyncCompletedPayload {
            repo_id: "repo-1".to_string(),
            repo_root: std::env::temp_dir(),
            active_branch: Some("main".to_string()),
            head_commit_sha: Some("abc123".to_string()),
            sync_mode: "full".to_string(),
            sync_completed_at: "2026-04-06T00:00:00Z".to_string(),
            files: SyncFileDiff {
                added: vec![ChangedFile {
                    path: "src/new.rs".to_string(),
                    language: "rust".to_string(),
                    content_id: "blob-new".to_string(),
                }],
                changed: vec![ChangedFile {
                    path: "src/changed.rs".to_string(),
                    language: "rust".to_string(),
                    content_id: "blob-changed".to_string(),
                }],
                removed: vec![RemovedFile {
                    path: "src/old.rs".to_string(),
                }],
            },
            artefacts: SyncArtefactDiff {
                added: vec![ChangedArtefact {
                    artefact_id: "aid-add".to_string(),
                    symbol_id: "sid-add".to_string(),
                    path: "src/new.rs".to_string(),
                    canonical_kind: Some("function".to_string()),
                    name: "new_fn".to_string(),
                }],
                changed: vec![ChangedArtefact {
                    artefact_id: "aid-changed".to_string(),
                    symbol_id: "sid-changed".to_string(),
                    path: "src/changed.rs".to_string(),
                    canonical_kind: Some("function".to_string()),
                    name: "changed_fn".to_string(),
                }],
                removed: vec![RemovedArtefact {
                    artefact_id: "aid-removed".to_string(),
                    symbol_id: "sid-removed".to_string(),
                    path: "src/old.rs".to_string(),
                }],
            },
        })
    }

    #[test]
    fn handler_subscribes_to_sync_completed() {
        let handler = TestHarnessSyncHandler;
        assert_eq!(handler.event_kind(), HostEventKind::SyncCompleted);
        assert_eq!(handler.capability_id(), TEST_HARNESS_CAPABILITY_ID);
    }

    #[tokio::test]
    async fn handler_accepts_sync_completed_payload() {
        let temp = TempDir::new().expect("tempdir");
        let db_path = temp
            .path()
            .join("bitloops-test-harness-sync-handler-tests.sqlite");
        init_test_domain_database(&db_path).expect("init test harness schema");
        let handler = TestHarnessSyncHandler;
        let event = test_event();
        let pool = crate::storage::SqliteConnectionPool::connect(db_path.clone())
            .expect("open sqlite pool");
        let context = EventHandlerContext {
            storage: std::sync::Arc::new(crate::host::devql::RelationalStorage::local_only(
                db_path,
            )),
            relational: Arc::new(
                crate::host::capability_host::gateways::SqliteRelationalGateway::new(pool),
            ),
            language_services: std::sync::Arc::new(
                crate::host::capability_host::gateways::EmptyLanguageServicesGateway,
            ),
            host_services: std::sync::Arc::new(
                crate::host::capability_host::gateways::DefaultHostServicesGateway::new("repo-1"),
            ),
        };

        handler
            .handle(&event, &context)
            .await
            .expect("test harness sync handler should accept SyncCompleted payload");
    }

    #[tokio::test]
    async fn handler_materializes_changed_test_file_and_cleans_removed_edges() {
        let temp = TempDir::new().expect("tempdir");
        let db_path = temp.path().join("handler.sqlite");
        init_test_domain_database(&db_path).expect("init test harness schema");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(repo_root.join("tests")).expect("repo tests dir");
        std::fs::write(repo_root.join("tests").join("user_test.rs"), "mod tests {}")
            .expect("write test file");

        let storage = crate::host::devql::RelationalStorage::local_only(db_path.clone());
        storage
            .exec(
                "CREATE TABLE IF NOT EXISTS artefacts_current (
                    repo_id TEXT NOT NULL,
                    path TEXT NOT NULL,
                    content_id TEXT NOT NULL,
                    symbol_id TEXT NOT NULL,
                    artefact_id TEXT NOT NULL,
                    language TEXT NOT NULL,
                    canonical_kind TEXT,
                    language_kind TEXT,
                    symbol_fqn TEXT NOT NULL,
                    parent_symbol_id TEXT,
                    parent_artefact_id TEXT,
                    start_line INTEGER NOT NULL,
                    end_line INTEGER NOT NULL,
                    start_byte INTEGER NOT NULL,
                    end_byte INTEGER NOT NULL,
                    signature TEXT,
                    modifiers TEXT NOT NULL DEFAULT '[]',
                    docstring TEXT,
                    updated_at TEXT
                )",
            )
            .await
            .expect("create artefacts_current table");
        storage
            .exec(
                "INSERT INTO artefacts_current (repo_id, path, content_id, symbol_id, artefact_id, language, canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, updated_at) \
                 VALUES ('repo-1','src/user/service.rs','blob-prod','sym::create_user','aid::create_user','rust','function','fn','src/user/service.rs::create_user',NULL,NULL,1,10,0,100,'fn create_user()','[]',NULL,datetime('now'))"
            )
            .await
            .expect("insert production artefact");
        storage
            .exec(
                "INSERT INTO test_artefact_edges_current (repo_id, path, content_id, edge_id, from_artefact_id, from_symbol_id, to_artefact_id, to_symbol_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata, updated_at) \
                 VALUES ('repo-1','tests/legacy_test.rs','blob-legacy','edge-legacy','from-aid','from-sid','to-aid','sym::removed',NULL,'tests','rust',1,1,'{}',datetime('now'))",
            )
            .await
            .expect("insert stale edge");

        let handler = TestHarnessSyncHandler;
        let event = HostEvent::SyncCompleted(SyncCompletedPayload {
            repo_id: "repo-1".to_string(),
            repo_root: repo_root.clone(),
            active_branch: Some("main".to_string()),
            head_commit_sha: Some("abc123".to_string()),
            sync_mode: "incremental".to_string(),
            sync_completed_at: "2026-04-06T00:00:00Z".to_string(),
            files: SyncFileDiff {
                added: vec![],
                changed: vec![ChangedFile {
                    path: "tests/user_test.rs".to_string(),
                    language: "rust".to_string(),
                    content_id: "blob-test-1".to_string(),
                }],
                removed: vec![],
            },
            artefacts: SyncArtefactDiff {
                added: vec![],
                changed: vec![],
                removed: vec![RemovedArtefact {
                    artefact_id: "aid::removed".to_string(),
                    symbol_id: "sym::removed".to_string(),
                    path: "src/old.rs".to_string(),
                }],
            },
        });
        let pool =
            crate::storage::SqliteConnectionPool::connect(db_path).expect("open sqlite pool");
        let context = EventHandlerContext {
            storage: Arc::new(storage),
            relational: Arc::new(
                crate::host::capability_host::gateways::SqliteRelationalGateway::new(pool),
            ),
            language_services: Arc::new(FakeLanguageGateway {
                support: Arc::new(FakeTestSupport),
            }),
            host_services: Arc::new(NoopHostServices),
        };

        handler
            .handle(&event, &context)
            .await
            .expect("handler succeeds");

        let artefacts = context
            .storage
            .query_rows(
                "SELECT symbol_id, canonical_kind FROM test_artefacts_current WHERE repo_id = 'repo-1' ORDER BY canonical_kind",
            )
            .await
            .expect("query test artefacts");
        assert_eq!(
            artefacts.len(),
            2,
            "suite + scenario should be materialized"
        );

        let edges = context
            .storage
            .query_rows(
                "SELECT edge_id, to_symbol_id FROM test_artefact_edges_current WHERE repo_id = 'repo-1'",
            )
            .await
            .expect("query edges");
        assert_eq!(edges.len(), 1, "stale edge should be removed");
        assert_eq!(
            edges[0].get("to_symbol_id").and_then(Value::as_str),
            Some("sym::create_user")
        );
    }
}
