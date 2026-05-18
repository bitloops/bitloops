use std::collections::{BTreeSet, HashMap, HashSet};

use anyhow::Result;
use serde_json::Value;

use crate::capability_packs::semantic_clones::RepoEmbeddingSyncAction;
use crate::capability_packs::semantic_clones::embeddings;
use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::devql::{RelationalStorage, RelationalStorageRole};

use super::ensure_semantic_embeddings_schema;
use super::sql::{
    build_active_embedding_setup_lookup_sql, build_current_repo_artefact_snapshots_sql,
    build_current_repo_semantic_clone_coverage_sql, build_repo_embedding_state_candidates_sql,
    build_semantic_summary_lookup_sql, build_symbol_embedding_index_state_sql,
    build_symbol_embedding_index_states_sql, representation_kind_sql_predicate,
};

async fn query_current_rows(relational: &RelationalStorage, sql: &str) -> Result<Vec<Value>> {
    relational
        .query_rows_for_role(RelationalStorageRole::CurrentProjection, sql)
        .await
}

async fn query_shared_rows(relational: &RelationalStorage, sql: &str) -> Result<Vec<Value>> {
    relational
        .query_rows_for_role(RelationalStorageRole::SharedRelational, sql)
        .await
}

fn build_current_local_semantic_summary_lookup_sql(artefact_ids: &[String]) -> String {
    build_semantic_summary_lookup_sql(artefact_ids, "symbol_semantics_current")
}

fn build_current_summary_embedding_coverage_rows_sql(
    repo_id: &str,
    setup: &embeddings::EmbeddingSetup,
) -> String {
    format!(
        "SELECT DISTINCT \
            a.artefact_id AS artefact_id, \
            a.content_id AS content_id, \
            CASE WHEN sc.artefact_id IS NOT NULL THEN 1 ELSE 0 END AS has_current_summary, \
            CASE WHEN sf.artefact_id IS NOT NULL THEN 1 ELSE 0 END AS has_current_features, \
            CASE WHEN e.artefact_id IS NOT NULL THEN 1 ELSE 0 END AS has_current_embedding \
         FROM artefacts_current a \
         JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path \
         LEFT JOIN symbol_semantics_current sc \
           ON sc.repo_id = a.repo_id \
          AND sc.artefact_id = a.artefact_id \
          AND sc.content_id = a.content_id \
         LEFT JOIN symbol_features_current sf \
           ON sf.repo_id = a.repo_id \
          AND sf.artefact_id = a.artefact_id \
          AND sf.content_id = a.content_id \
         LEFT JOIN symbol_embeddings_current e \
           ON e.repo_id = a.repo_id \
          AND e.artefact_id = a.artefact_id \
          AND e.content_id = a.content_id \
          AND {representation_predicate} \
          AND e.provider = '{provider}' \
          AND e.model = '{model}' \
          AND e.dimension = {dimension} \
         WHERE a.repo_id = '{repo_id}' \
           AND cfs.analysis_mode = 'code' \
           AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import' \
         ORDER BY a.artefact_id",
        repo_id = crate::host::devql::esc_pg(repo_id),
        representation_predicate = representation_kind_sql_predicate(
            "e.representation_kind",
            embeddings::EmbeddingRepresentationKind::Summary,
        ),
        provider = crate::host::devql::esc_pg(&setup.provider),
        model = crate::host::devql::esc_pg(&setup.model),
        dimension = setup.dimension,
    )
}

fn build_shared_historical_summary_presence_sql(
    repo_id: &str,
    artefact_blob_pairs: &[(String, String)],
) -> Option<String> {
    if artefact_blob_pairs.is_empty() {
        return None;
    }
    let pair_predicate = artefact_blob_pairs
        .iter()
        .map(|(artefact_id, blob_sha)| {
            format!(
                "(artefact_id = '{artefact_id}' AND blob_sha = '{blob_sha}')",
                artefact_id = crate::host::devql::esc_pg(artefact_id),
                blob_sha = crate::host::devql::esc_pg(blob_sha),
            )
        })
        .collect::<Vec<_>>()
        .join(" OR ");
    Some(format!(
        "SELECT artefact_id, blob_sha AS content_id \
         FROM symbol_semantics \
         WHERE repo_id = '{repo_id}' AND ({pair_predicate})",
        repo_id = crate::host::devql::esc_pg(repo_id),
        pair_predicate = pair_predicate,
    ))
}

pub(crate) async fn load_active_embedding_setup(
    relational: &RelationalStorage,
    repo_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
) -> Result<Option<embeddings::ActiveEmbeddingRepresentationState>> {
    ensure_semantic_embeddings_schema(relational).await?;
    let rows = query_current_rows(
        relational,
        &build_active_embedding_setup_lookup_sql(repo_id, representation_kind),
    )
    .await?;
    Ok(parse_active_embedding_state_rows(&rows).into_iter().next())
}

pub(crate) async fn determine_repo_embedding_sync_action(
    relational: &RelationalStorage,
    repo_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    setup: &embeddings::EmbeddingSetup,
) -> Result<RepoEmbeddingSyncAction> {
    let current_coverage_complete = current_repo_semantic_clone_rows_are_complete(
        relational,
        repo_id,
        representation_kind,
        setup,
    )
    .await?;
    if let Some(active) =
        load_active_embedding_setup(relational, repo_id, representation_kind).await?
        && active.setup == *setup
    {
        return Ok(if current_coverage_complete {
            RepoEmbeddingSyncAction::Incremental
        } else {
            RepoEmbeddingSyncAction::RefreshCurrentRepo
        });
    }

    let current_states =
        load_current_repo_embedding_states(relational, repo_id, Some(representation_kind)).await?;
    Ok(
        if current_states.iter().any(|state| state.setup == *setup) {
            if current_coverage_complete {
                RepoEmbeddingSyncAction::AdoptExisting
            } else {
                RepoEmbeddingSyncAction::RefreshCurrentRepo
            }
        } else {
            RepoEmbeddingSyncAction::RefreshCurrentRepo
        },
    )
}

pub(crate) async fn load_current_repo_embedding_states(
    relational: &RelationalStorage,
    repo_id: &str,
    representation_kind: Option<embeddings::EmbeddingRepresentationKind>,
) -> Result<Vec<embeddings::ActiveEmbeddingRepresentationState>> {
    let current_snapshots = load_current_repo_artefact_snapshots(relational, repo_id).await?;
    if current_snapshots.is_empty() {
        return Ok(Vec::new());
    }

    let current_rows = match query_current_rows(
        relational,
        &build_repo_embedding_state_candidates_sql(
            "symbol_embeddings_current",
            "content_id",
            repo_id,
            representation_kind,
        ),
    )
    .await
    {
        Ok(rows) => filter_rows_for_current_snapshots(rows, &current_snapshots),
        Err(err) if missing_current_embedding_projection_table(&err) => Vec::new(),
        Err(err) => return Err(err),
    };
    let historical_rows = match query_shared_rows(
        relational,
        &build_repo_embedding_state_candidates_sql(
            "symbol_embeddings",
            "blob_sha",
            repo_id,
            representation_kind,
        ),
    )
    .await
    {
        Ok(rows) => filter_rows_for_current_snapshots(rows, &current_snapshots),
        Err(err) if missing_shared_embedding_table(&err) => Vec::new(),
        Err(err) => return Err(err),
    };
    let mut rows = current_rows;
    rows.extend(historical_rows);
    Ok(parse_active_embedding_state_rows(&rows))
}

pub(crate) async fn load_symbol_embedding_index_state(
    relational: &RelationalStorage,
    artefact_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    setup_fingerprint: &str,
) -> Result<embeddings::SymbolEmbeddingIndexState> {
    let rows = query_shared_rows(
        relational,
        &build_symbol_embedding_index_state_sql(
            artefact_id,
            "symbol_embeddings",
            representation_kind,
            setup_fingerprint,
        ),
    )
    .await?;
    Ok(parse_symbol_embedding_index_state_rows(&rows))
}

pub(crate) async fn load_symbol_embedding_index_states(
    relational: &RelationalStorage,
    artefact_ids: &[String],
    representation_kind: embeddings::EmbeddingRepresentationKind,
    setup_fingerprint: &str,
) -> Result<HashMap<String, embeddings::SymbolEmbeddingIndexState>> {
    if artefact_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = query_shared_rows(
        relational,
        &build_symbol_embedding_index_states_sql(
            artefact_ids,
            "symbol_embeddings",
            representation_kind,
            setup_fingerprint,
        ),
    )
    .await?;
    Ok(parse_symbol_embedding_index_state_map_rows(&rows))
}

pub(crate) async fn load_current_symbol_embedding_index_states(
    relational: &RelationalStorage,
    artefact_ids: &[String],
    representation_kind: embeddings::EmbeddingRepresentationKind,
    setup_fingerprint: &str,
) -> Result<HashMap<String, embeddings::SymbolEmbeddingIndexState>> {
    if artefact_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = query_current_rows(
        relational,
        &build_symbol_embedding_index_states_sql(
            artefact_ids,
            "symbol_embeddings_current",
            representation_kind,
            setup_fingerprint,
        ),
    )
    .await?;
    Ok(parse_symbol_embedding_index_state_map_rows(&rows))
}

pub(crate) async fn load_current_symbol_embedding_index_state(
    relational: &RelationalStorage,
    artefact_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    setup_fingerprint: &str,
) -> Result<embeddings::SymbolEmbeddingIndexState> {
    let rows = query_current_rows(
        relational,
        &build_symbol_embedding_index_state_sql(
            artefact_id,
            "symbol_embeddings_current",
            representation_kind,
            setup_fingerprint,
        ),
    )
    .await?;
    Ok(parse_symbol_embedding_index_state_rows(&rows))
}

pub(crate) async fn load_semantic_summary_map(
    relational: &RelationalStorage,
    artefact_ids: &[String],
    representation_kind: embeddings::EmbeddingRepresentationKind,
) -> Result<HashMap<String, String>> {
    load_semantic_summary_map_from_table(
        relational,
        artefact_ids,
        "symbol_semantics",
        representation_kind,
    )
    .await
}

#[allow(dead_code)]
pub(crate) async fn load_current_semantic_summary_map(
    relational: &RelationalStorage,
    artefact_ids: &[String],
    representation_kind: embeddings::EmbeddingRepresentationKind,
) -> Result<HashMap<String, String>> {
    if representation_kind != embeddings::EmbeddingRepresentationKind::Summary {
        return Ok(HashMap::new());
    }

    let current = query_current_rows(
        relational,
        &build_current_local_semantic_summary_lookup_sql(artefact_ids),
    )
    .await
    .map(|rows| {
        let mut out = HashMap::with_capacity(rows.len());
        for row in rows {
            let Some(artefact_id) = row.get("artefact_id").and_then(Value::as_str) else {
                continue;
            };
            if let Some(summary) = resolve_embedding_summary(&row) {
                out.insert(artefact_id.to_string(), summary);
            }
        }
        out
    });
    match current {
        Ok(mut summary_map) => {
            let missing_ids = artefact_ids
                .iter()
                .filter(|artefact_id| !summary_map.contains_key(*artefact_id))
                .cloned()
                .collect::<Vec<_>>();
            if !missing_ids.is_empty() {
                summary_map.extend(
                    load_semantic_summary_map_from_table(
                        relational,
                        &missing_ids,
                        "symbol_semantics",
                        representation_kind,
                    )
                    .await?,
                );
            }
            Ok(summary_map)
        }
        Err(err) if missing_current_summary_projection_table(&err) => {
            load_semantic_summary_map_from_table(
                relational,
                artefact_ids,
                "symbol_semantics_current",
                representation_kind,
            )
            .await
        }
        Err(err) => Err(err),
    }
}

pub(crate) fn parse_symbol_embedding_index_state_rows(
    rows: &[Value],
) -> embeddings::SymbolEmbeddingIndexState {
    let Some(row) = rows.first() else {
        return embeddings::SymbolEmbeddingIndexState::default();
    };

    embeddings::SymbolEmbeddingIndexState {
        embedding_hash: row
            .get("embedding_hash")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

fn parse_symbol_embedding_index_state_map_rows(
    rows: &[Value],
) -> HashMap<String, embeddings::SymbolEmbeddingIndexState> {
    let mut out = HashMap::with_capacity(rows.len());
    for row in rows {
        let Some(artefact_id) = row.get("artefact_id").and_then(Value::as_str) else {
            continue;
        };
        out.insert(
            artefact_id.to_string(),
            embeddings::SymbolEmbeddingIndexState {
                embedding_hash: row
                    .get("embedding_hash")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            },
        );
    }
    out
}

async fn current_repo_semantic_clone_rows_are_complete(
    relational: &RelationalStorage,
    repo_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    setup: &embeddings::EmbeddingSetup,
) -> Result<bool> {
    if representation_kind == embeddings::EmbeddingRepresentationKind::Summary {
        return current_summary_embedding_rows_are_complete(relational, repo_id, setup).await;
    }
    let rows = query_current_rows(
        relational,
        &build_current_repo_semantic_clone_coverage_sql(repo_id, representation_kind, setup),
    )
    .await?;
    let Some(row) = rows.first() else {
        return Ok(true);
    };
    let eligible_current_artefacts = row
        .get("eligible_current_artefacts")
        .and_then(value_as_positive_usize)
        .unwrap_or_default();
    let fully_indexed_current_artefacts = row
        .get("fully_indexed_current_artefacts")
        .and_then(value_as_positive_usize)
        .unwrap_or_default();
    Ok(eligible_current_artefacts == fully_indexed_current_artefacts)
}

async fn current_summary_embedding_rows_are_complete(
    relational: &RelationalStorage,
    repo_id: &str,
    setup: &embeddings::EmbeddingSetup,
) -> Result<bool> {
    let current_rows = query_current_rows(
        relational,
        &build_current_summary_embedding_coverage_rows_sql(repo_id, setup),
    )
    .await?;
    if current_rows.is_empty() {
        return Ok(true);
    }

    let coverage_rows = current_rows
        .into_iter()
        .filter_map(parse_current_summary_embedding_coverage_row)
        .collect::<Vec<_>>();
    if coverage_rows.is_empty() {
        return Ok(true);
    }

    let shared_lookup_pairs = coverage_rows
        .iter()
        .filter(|row| !row.has_current_summary)
        .map(|row| (row.artefact_id.clone(), row.content_id.clone()))
        .collect::<Vec<_>>();
    let shared_summary_rows =
        match build_shared_historical_summary_presence_sql(repo_id, &shared_lookup_pairs) {
            Some(sql) => query_shared_rows(relational, &sql).await?,
            None => Vec::new(),
        };
    let shared_summary_keys = shared_summary_rows
        .into_iter()
        .filter_map(|row| {
            Some((
                row.get("artefact_id")?.as_str()?.to_string(),
                row.get("content_id")?.as_str()?.to_string(),
            ))
        })
        .collect::<HashSet<_>>();

    let eligible_current_artefacts = coverage_rows
        .iter()
        .filter(|row| {
            row.has_current_summary
                || shared_summary_keys.contains(&(row.artefact_id.clone(), row.content_id.clone()))
        })
        .count();
    let fully_indexed_current_artefacts = coverage_rows
        .iter()
        .filter(|row| {
            (row.has_current_summary
                || shared_summary_keys.contains(&(row.artefact_id.clone(), row.content_id.clone())))
                && row.has_current_features
                && row.has_current_embedding
        })
        .count();
    Ok(eligible_current_artefacts == fully_indexed_current_artefacts)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CurrentSummaryEmbeddingCoverageRow {
    artefact_id: String,
    content_id: String,
    has_current_summary: bool,
    has_current_features: bool,
    has_current_embedding: bool,
}

fn parse_current_summary_embedding_coverage_row(
    row: Value,
) -> Option<CurrentSummaryEmbeddingCoverageRow> {
    Some(CurrentSummaryEmbeddingCoverageRow {
        artefact_id: row.get("artefact_id")?.as_str()?.to_string(),
        content_id: row.get("content_id")?.as_str()?.to_string(),
        has_current_summary: value_as_bool(&row["has_current_summary"]),
        has_current_features: value_as_bool(&row["has_current_features"]),
        has_current_embedding: value_as_bool(&row["has_current_embedding"]),
    })
}

async fn load_semantic_summary_map_from_table(
    relational: &RelationalStorage,
    artefact_ids: &[String],
    table: &str,
    _representation_kind: embeddings::EmbeddingRepresentationKind,
) -> Result<HashMap<String, String>> {
    if artefact_ids.is_empty() {
        return Ok(HashMap::new());
    }

    load_semantic_summary_map_from_sql(
        relational,
        artefact_ids,
        build_semantic_summary_lookup_sql(artefact_ids, table),
        _representation_kind,
    )
    .await
}

async fn load_semantic_summary_map_from_sql(
    relational: &RelationalStorage,
    artefact_ids: &[String],
    sql: String,
    _representation_kind: embeddings::EmbeddingRepresentationKind,
) -> Result<HashMap<String, String>> {
    if artefact_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = query_shared_rows(relational, &sql).await?;
    let mut out = HashMap::with_capacity(rows.len());
    for row in rows {
        let Some(artefact_id) = row.get("artefact_id").and_then(Value::as_str) else {
            continue;
        };
        if let Some(summary) = resolve_embedding_summary(&row) {
            out.insert(artefact_id.to_string(), summary);
        }
    }
    Ok(out)
}

async fn load_current_repo_artefact_snapshots(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<BTreeSet<(String, String)>> {
    let rows = query_current_rows(
        relational,
        &build_current_repo_artefact_snapshots_sql(repo_id),
    )
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some((
                row.get("artefact_id")?.as_str()?.to_string(),
                row.get("snapshot_id")?.as_str()?.to_string(),
            ))
        })
        .collect())
}

fn filter_rows_for_current_snapshots(
    rows: Vec<Value>,
    current_snapshots: &BTreeSet<(String, String)>,
) -> Vec<Value> {
    rows.into_iter()
        .filter(|row| {
            let Some(artefact_id) = row.get("artefact_id").and_then(Value::as_str) else {
                return false;
            };
            let Some(snapshot_id) = row.get("snapshot_id").and_then(Value::as_str) else {
                return false;
            };
            current_snapshots.contains(&(artefact_id.to_string(), snapshot_id.to_string()))
        })
        .collect()
}

fn parse_active_embedding_state_rows(
    rows: &[Value],
) -> Vec<embeddings::ActiveEmbeddingRepresentationState> {
    let mut states = BTreeSet::new();
    for row in rows {
        let Some(representation_kind) = row
            .get("representation_kind")
            .and_then(Value::as_str)
            .and_then(parse_representation_kind)
        else {
            continue;
        };
        let Some(provider) = row.get("provider").and_then(Value::as_str) else {
            continue;
        };
        let Some(model) = row.get("model").and_then(Value::as_str) else {
            continue;
        };
        let Some(dimension) = row
            .get("dimension")
            .and_then(value_as_positive_usize)
            .filter(|value| *value > 0)
        else {
            continue;
        };
        let setup_fingerprint = row
            .get("setup_fingerprint")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                embeddings::EmbeddingSetup::new(provider, model, dimension).setup_fingerprint
            });
        states.insert((
            representation_kind,
            provider.to_string(),
            model.to_string(),
            dimension,
            setup_fingerprint,
        ));
    }

    states
        .into_iter()
        .map(
            |(representation_kind, provider, model, dimension, setup_fingerprint)| {
                embeddings::ActiveEmbeddingRepresentationState::new(
                    representation_kind,
                    embeddings::EmbeddingSetup {
                        provider,
                        model,
                        dimension,
                        setup_fingerprint,
                    },
                )
            },
        )
        .collect()
}

fn missing_current_summary_projection_table(err: &anyhow::Error) -> bool {
    let message = format!("{err:#}");
    missing_relation_error(&message, "artefacts_current")
        || missing_relation_error(&message, "current_file_state")
        || (missing_relation_error(&message, "symbol_semantics")
            && !missing_relation_error(&message, "symbol_semantics_current"))
}

fn missing_current_embedding_projection_table(err: &anyhow::Error) -> bool {
    let message = format!("{err:#}");
    missing_relation_error(&message, "symbol_embeddings_current")
}

fn missing_shared_embedding_table(err: &anyhow::Error) -> bool {
    let message = format!("{err:#}");
    missing_relation_error(&message, "symbol_embeddings")
}

fn missing_relation_error(message: &str, relation: &str) -> bool {
    message.contains(&format!("no such table: {relation}"))
        || message.contains(&format!("relation \"{relation}\" does not exist"))
        || message.contains(&format!("relation '{relation}' does not exist"))
        || message.contains(&format!("relation {relation} does not exist"))
}

fn value_as_positive_usize(value: &Value) -> Option<usize> {
    if let Some(value) = value.as_u64() {
        return usize::try_from(value).ok();
    }
    if let Some(value) = value.as_i64() {
        return usize::try_from(value).ok();
    }
    value.as_str()?.trim().parse::<usize>().ok()
}

fn value_as_bool(value: &Value) -> bool {
    if let Some(value) = value.as_bool() {
        return value;
    }
    if let Some(value) = value.as_u64() {
        return value > 0;
    }
    if let Some(value) = value.as_i64() {
        return value > 0;
    }
    value
        .as_str()
        .map(str::trim)
        .map(|value| {
            value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("t") || value == "1"
        })
        .unwrap_or(false)
}

fn parse_representation_kind(raw: &str) -> Option<embeddings::EmbeddingRepresentationKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "code" | "baseline" | "enriched" => Some(embeddings::EmbeddingRepresentationKind::Code),
        "summary" => Some(embeddings::EmbeddingRepresentationKind::Summary),
        "identity" | "locator" => Some(embeddings::EmbeddingRepresentationKind::Identity),
        _ => None,
    }
}

fn resolve_embedding_summary(row: &Value) -> Option<String> {
    let template_summary = row
        .get("template_summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let docstring_summary = row
        .get("docstring_summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let canonical_summary = row
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let llm_summary = row
        .get("llm_summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let has_llm_enrichment = llm_summary.is_some()
        || row
            .get("source_model")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());

    if has_llm_enrichment {
        canonical_summary.map(str::to_string).or_else(|| {
            Some(semantic::synthesize_deterministic_summary(
                template_summary,
                docstring_summary,
            ))
        })
    } else {
        Some(semantic::synthesize_deterministic_summary(
            template_summary,
            docstring_summary,
        ))
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;
    use tempfile::tempdir;

    use super::{
        load_current_repo_embedding_states, load_current_semantic_summary_map,
        missing_current_summary_projection_table,
    };
    use crate::capability_packs::semantic_clones::embeddings;
    use crate::host::devql::{
        RelationalPrimaryBackend, RelationalStorage, sqlite_exec_path_allow_create,
    };

    #[test]
    fn summary_projection_missing_postgres_relation_falls_back() {
        let err =
            anyhow!("error returned from database: relation \"artefacts_current\" does not exist");

        assert!(missing_current_summary_projection_table(&err));
    }

    #[test]
    fn summary_projection_missing_current_table_does_not_fallback() {
        let err = anyhow!(
            "error returned from database: relation \"symbol_semantics_current\" does not exist"
        );

        assert!(!missing_current_summary_projection_table(&err));
    }

    #[tokio::test]
    async fn current_summary_map_reads_local_projection_even_when_primary_is_postgres() {
        let temp = tempdir().expect("temp dir");
        let sqlite_path = temp.path().join("semantic.sqlite");
        sqlite_exec_path_allow_create(
            &sqlite_path,
            "CREATE TABLE symbol_semantics_current (
                artefact_id TEXT PRIMARY KEY,
                repo_id TEXT NOT NULL,
                path TEXT NOT NULL,
                content_id TEXT NOT NULL,
                symbol_id TEXT,
                docstring_summary TEXT,
                llm_summary TEXT,
                template_summary TEXT NOT NULL,
                summary TEXT NOT NULL,
                source_model TEXT
            );
            INSERT INTO symbol_semantics_current (
                artefact_id, repo_id, path, content_id, symbol_id,
                docstring_summary, llm_summary, template_summary, summary, source_model
            ) VALUES (
                'artefact-1', 'repo-1', 'src/lib.rs', 'blob-1', 'sym-1',
                NULL, 'Current summary.', 'Current template.', 'Current summary.', 'test:model'
            );",
        )
        .await
        .expect("seed current semantic summary rows");
        let relational = RelationalStorage::primary_backend_for_tests(
            sqlite_path,
            RelationalPrimaryBackend::Postgres,
        );

        let summaries = load_current_semantic_summary_map(
            &relational,
            &["artefact-1".to_string()],
            embeddings::EmbeddingRepresentationKind::Summary,
        )
        .await
        .expect("load current semantic summary map");

        assert_eq!(
            summaries.get("artefact-1").map(String::as_str),
            Some("Current summary.")
        );
    }

    #[tokio::test]
    async fn current_embedding_states_fall_back_to_historical_rows_for_current_snapshots() {
        let temp = tempdir().expect("temp dir");
        let sqlite_path = temp.path().join("semantic.sqlite");
        let active_setup = embeddings::EmbeddingSetup::new("test-provider", "test-model", 3);
        let stale_setup = embeddings::EmbeddingSetup::new("stale-provider", "stale-model", 5);
        sqlite_exec_path_allow_create(
            &sqlite_path,
            &format!(
                "CREATE TABLE artefacts_current (
                    repo_id TEXT NOT NULL,
                    artefact_id TEXT NOT NULL,
                    content_id TEXT NOT NULL
                );
                CREATE TABLE symbol_embeddings (
                    artefact_id TEXT NOT NULL,
                    repo_id TEXT NOT NULL,
                    blob_sha TEXT NOT NULL,
                    representation_kind TEXT NOT NULL,
                    setup_fingerprint TEXT NOT NULL,
                    provider TEXT NOT NULL,
                    model TEXT NOT NULL,
                    dimension INTEGER NOT NULL,
                    embedding_input_hash TEXT NOT NULL,
                    embedding TEXT
                );
                INSERT INTO artefacts_current (repo_id, artefact_id, content_id) VALUES
                    ('repo-1', 'artefact-1', 'blob-1'),
                    ('repo-1', 'artefact-2', 'blob-2');
                INSERT INTO symbol_embeddings (
                    artefact_id, repo_id, blob_sha, representation_kind, setup_fingerprint,
                    provider, model, dimension, embedding_input_hash, embedding
                ) VALUES
                    ('artefact-1', 'repo-1', 'blob-1', 'code', '{active_fp}', 'test-provider', 'test-model', 3, 'hash-1', '[0.1,0.2,0.3]'),
                    ('artefact-2', 'repo-1', 'blob-2', 'code', '{active_fp}', 'test-provider', 'test-model', 3, 'hash-2', '[0.3,0.2,0.1]'),
                    ('artefact-1', 'repo-1', 'blob-stale', 'code', '{stale_fp}', 'stale-provider', 'stale-model', 5, 'hash-stale', '[0.9,0.8,0.7]');",
                active_fp = active_setup.setup_fingerprint,
                stale_fp = stale_setup.setup_fingerprint,
            ),
        )
        .await
        .expect("seed historical embedding rows");
        let relational = RelationalStorage::local_only(sqlite_path);

        let states = load_current_repo_embedding_states(
            &relational,
            "repo-1",
            Some(embeddings::EmbeddingRepresentationKind::Code),
        )
        .await
        .expect("load current repo embedding states");

        assert_eq!(states.len(), 1);
        assert_eq!(
            states[0].representation_kind,
            embeddings::EmbeddingRepresentationKind::Code
        );
        assert_eq!(states[0].setup.provider, "test-provider");
        assert_eq!(states[0].setup.model, "test-model");
        assert_eq!(states[0].setup.dimension, 3);
        assert_eq!(
            states[0].setup.setup_fingerprint,
            active_setup.setup_fingerprint
        );
    }
}
