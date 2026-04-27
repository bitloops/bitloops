use std::collections::{BTreeSet, HashMap};

use anyhow::Result;
use serde_json::Value;

use crate::capability_packs::semantic_clones::RepoEmbeddingSyncAction;
use crate::capability_packs::semantic_clones::embeddings;
use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::devql::RelationalStorage;

use super::ensure_semantic_embeddings_schema;
use super::sql::{
    build_active_embedding_setup_lookup_sql, build_current_repo_embedding_states_sql,
    build_current_repo_semantic_clone_coverage_sql, build_current_semantic_summary_lookup_sql,
    build_semantic_summary_lookup_sql, build_symbol_embedding_index_state_sql,
};

pub(crate) async fn load_active_embedding_setup(
    relational: &RelationalStorage,
    repo_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
) -> Result<Option<embeddings::ActiveEmbeddingRepresentationState>> {
    ensure_semantic_embeddings_schema(relational).await?;
    let rows = relational
        .query_rows(&build_active_embedding_setup_lookup_sql(
            repo_id,
            representation_kind,
        ))
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
    let rows = relational
        .query_rows(&build_current_repo_embedding_states_sql(
            repo_id,
            representation_kind,
        ))
        .await?;
    Ok(parse_active_embedding_state_rows(&rows))
}

pub(crate) async fn load_symbol_embedding_index_state(
    relational: &RelationalStorage,
    artefact_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    setup_fingerprint: &str,
) -> Result<embeddings::SymbolEmbeddingIndexState> {
    let rows = relational
        .query_rows(&build_symbol_embedding_index_state_sql(
            artefact_id,
            "symbol_embeddings",
            representation_kind,
            setup_fingerprint,
        ))
        .await?;
    Ok(parse_symbol_embedding_index_state_rows(&rows))
}

pub(crate) async fn load_current_symbol_embedding_index_state(
    relational: &RelationalStorage,
    artefact_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    setup_fingerprint: &str,
) -> Result<embeddings::SymbolEmbeddingIndexState> {
    let rows = relational
        .query_rows(&build_symbol_embedding_index_state_sql(
            artefact_id,
            "symbol_embeddings_current",
            representation_kind,
            setup_fingerprint,
        ))
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

    let current = load_semantic_summary_map_from_sql(
        relational,
        artefact_ids,
        build_current_semantic_summary_lookup_sql(artefact_ids),
        representation_kind,
    )
    .await;
    match current {
        Ok(summary_map) => Ok(summary_map),
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

async fn current_repo_semantic_clone_rows_are_complete(
    relational: &RelationalStorage,
    repo_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    setup: &embeddings::EmbeddingSetup,
) -> Result<bool> {
    let rows = relational
        .query_rows(&build_current_repo_semantic_clone_coverage_sql(
            repo_id,
            representation_kind,
            setup,
        ))
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

    let rows = relational.query_rows(&sql).await?;
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
    message.contains("no such table: artefacts_current")
        || message.contains("no such table: current_file_state")
        || (message.contains("no such table: symbol_semantics")
            && !message.contains("no such table: symbol_semantics_current"))
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
