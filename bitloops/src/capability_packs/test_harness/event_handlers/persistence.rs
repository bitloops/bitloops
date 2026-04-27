use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};
use serde_json::Value;

use crate::host::devql::{RelationalStorage, esc_pg};
use crate::models::{TestArtefactCurrentRecord, TestArtefactEdgeCurrentRecord};

pub(super) async fn replace_repo_state(
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

pub(super) async fn persist_discovered_files(
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

pub(super) fn ensure_unique_test_artefact_ids(
    test_artefacts: &[TestArtefactCurrentRecord],
) -> Result<()> {
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

pub(super) async fn delete_paths(
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

pub(super) async fn delete_edges_to_removed_symbols(
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
