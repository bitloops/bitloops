use std::collections::{HashMap, HashSet};

use anyhow::Result;
use serde_json::Value;

use crate::capability_packs::test_harness::mapping;
use crate::capability_packs::test_harness::mapping::linker::build_production_index;
use crate::capability_packs::test_harness::mapping::materialize::{
    MaterializationContext, materialize_source_discovery,
};
use crate::capability_packs::test_harness::mapping::model::StructuralMappingStats;
use crate::host::capability_host::{
    CurrentStateConsumer, CurrentStateConsumerContext, CurrentStateConsumerFuture,
    CurrentStateConsumerRequest, CurrentStateConsumerResult, ReconcileMode,
};
use crate::host::devql::{RelationalStorage, esc_pg};
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
