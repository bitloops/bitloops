use std::collections::HashMap;

use anyhow::Result;
use serde_json::Value;

use crate::host::devql::{RelationalStorage, esc_pg, sql_string_list_pg};

use super::types::{SearchDocumentCandidate, SearchFeatureCandidate};

pub(super) async fn load_search_features(
    relational: &RelationalStorage,
    repo_id: &str,
    artefact_ids: &[String],
) -> Result<HashMap<String, SearchFeatureCandidate>> {
    if artefact_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let sql = format!(
        "SELECT artefact_id, normalized_name, normalized_signature, identifier_tokens, normalized_body_tokens \
         FROM symbol_features_current \
         WHERE repo_id = '{repo_id}' AND artefact_id IN ({artefact_ids})",
        repo_id = esc_pg(repo_id),
        artefact_ids = sql_string_list_pg(artefact_ids),
    );
    let rows = query_rows_all_safe(relational, &sql).await?;
    let mut features_by_id = HashMap::new();
    for row in rows {
        let Some(artefact_id) = string_field(&row, "artefact_id") else {
            continue;
        };
        let candidate = SearchFeatureCandidate {
            normalized_name: string_field(&row, "normalized_name"),
            normalized_signature: string_field(&row, "normalized_signature")
                .map(|value| value.to_ascii_lowercase()),
            identifier_tokens: parse_string_array_field(&row, "identifier_tokens")
                .into_iter()
                .collect(),
            normalized_body_tokens: parse_string_array_field(&row, "normalized_body_tokens")
                .into_iter()
                .collect(),
        };
        features_by_id.insert(artefact_id, candidate);
    }
    Ok(features_by_id)
}

pub(super) async fn load_search_documents(
    relational: &RelationalStorage,
    repo_id: &str,
    artefact_ids: &[String],
) -> Result<HashMap<String, SearchDocumentCandidate>> {
    if artefact_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let sql = format!(
        "SELECT artefact_id, signature_text, summary_text, body_text \
         FROM symbol_search_documents_current \
         WHERE repo_id = '{repo_id}' AND artefact_id IN ({artefact_ids})",
        repo_id = esc_pg(repo_id),
        artefact_ids = sql_string_list_pg(artefact_ids),
    );
    let rows = query_rows_all_safe(relational, &sql).await?;
    let mut documents_by_id = HashMap::new();
    for row in rows {
        let Some(artefact_id) = string_field(&row, "artefact_id") else {
            continue;
        };
        documents_by_id.insert(
            artefact_id,
            SearchDocumentCandidate {
                signature_text: string_field(&row, "signature_text"),
                summary_text: string_field(&row, "summary_text"),
                body_text: string_field(&row, "body_text"),
            },
        );
    }
    Ok(documents_by_id)
}

pub(super) async fn query_rows_all_safe(
    relational: &RelationalStorage,
    sql: &str,
) -> Result<Vec<Value>> {
    let mut rows = match relational.query_rows(sql).await {
        Ok(rows) => rows,
        Err(err) if is_missing_relation_error(&err) => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    if relational.remote_client().is_some() {
        match relational.query_rows_remote(sql).await {
            Ok(remote_rows) => rows.extend(remote_rows),
            Err(err) if is_missing_relation_error(&err) => {}
            Err(err) => return Err(err),
        }
    }
    Ok(rows)
}

fn is_missing_relation_error(err: &anyhow::Error) -> bool {
    let message = format!("{err:#}");
    message.contains("no such table")
        || message.contains("relation") && message.contains("does not exist")
        || message.contains("symbol_search_documents_current_fts")
}

fn parse_string_array_field(row: &Value, key: &str) -> Vec<String> {
    let Some(value) = row.get(key) else {
        return Vec::new();
    };
    if let Some(items) = value.as_array() {
        return items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
    }
    value
        .as_str()
        .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .unwrap_or_default()
}

pub(super) fn string_field(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn numeric_field(row: &Value, key: &str) -> Option<f64> {
    row.get(key)
        .and_then(Value::as_f64)
        .or_else(|| {
            row.get(key)
                .and_then(Value::as_i64)
                .map(|value| value as f64)
        })
        .or_else(|| {
            row.get(key)
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<f64>().ok())
        })
}
