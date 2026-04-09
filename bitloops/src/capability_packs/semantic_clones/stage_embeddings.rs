//! Stage 2: symbol embedding rows (`symbol_embeddings`) for the semantic_clones pipeline.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::adapters::model_providers::embeddings::EmbeddingProvider;
use crate::capability_packs::semantic_clones::embeddings;
use crate::capability_packs::semantic_clones::features as semantic;
use crate::host::devql::{RelationalStorage, esc_pg, sql_string_list_pg};

#[path = "stage_embeddings/schema.rs"]
mod schema;

pub(crate) use schema::{
    init_postgres_semantic_embeddings_schema, init_sqlite_semantic_embeddings_schema,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepoEmbeddingSyncAction {
    Incremental,
    AdoptExisting,
    RefreshCurrentRepo,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CurrentRepoEmbeddingRefreshResult {
    pub semantic_feature_stats: semantic::SemanticFeatureIngestionStats,
    pub embedding_stats: embeddings::SymbolEmbeddingIngestionStats,
    pub clone_build: crate::capability_packs::semantic_clones::scoring::SymbolCloneBuildResult,
}

pub(crate) async fn upsert_symbol_embedding_rows(
    relational: &RelationalStorage,
    inputs: &[semantic::SemanticFeatureInput],
    representation_kind: embeddings::EmbeddingRepresentationKind,
    embedding_provider: Arc<dyn EmbeddingProvider>,
) -> Result<embeddings::SymbolEmbeddingIngestionStats> {
    let mut stats = embeddings::SymbolEmbeddingIngestionStats::default();
    if inputs.is_empty() {
        return Ok(stats);
    }

    ensure_semantic_embeddings_schema(relational).await?;
    let setup = embeddings::resolve_embedding_setup(embedding_provider.as_ref())?;

    let artefact_ids = inputs
        .iter()
        .map(|input| input.artefact_id.clone())
        .collect::<Vec<_>>();
    let summary_by_artefact_id =
        load_semantic_summary_map(relational, &artefact_ids, representation_kind).await?;
    let embedding_inputs = embeddings::build_symbol_embedding_inputs(
        inputs,
        representation_kind,
        &summary_by_artefact_id,
    );
    stats.eligible = embedding_inputs.len();

    for input in embedding_inputs {
        let next_input_hash =
            embeddings::build_symbol_embedding_input_hash(&input, embedding_provider.as_ref());
        let state = load_symbol_embedding_index_state(
            relational,
            &input.artefact_id,
            input.representation_kind,
            &setup.setup_fingerprint,
        )
        .await?;
        if !embeddings::symbol_embeddings_require_reindex(&state, &next_input_hash) {
            stats.skipped += 1;
            continue;
        }

        let input = input.clone();
        let embedding_provider = Arc::clone(&embedding_provider);
        let row = tokio::task::spawn_blocking(move || {
            embeddings::build_symbol_embedding_row(&input, embedding_provider.as_ref())
        })
        .await
        .context("building semantic embedding row on blocking worker")??;
        persist_symbol_embedding_row(relational, &row).await?;
        stats.upserted += 1;
    }

    Ok(stats)
}

#[allow(dead_code)]
pub(crate) async fn upsert_current_symbol_embedding_rows(
    relational: &RelationalStorage,
    path: &str,
    content_id: &str,
    inputs: &[semantic::SemanticFeatureInput],
    representation_kind: embeddings::EmbeddingRepresentationKind,
    embedding_provider: Arc<dyn EmbeddingProvider>,
) -> Result<embeddings::SymbolEmbeddingIngestionStats> {
    let mut stats = embeddings::SymbolEmbeddingIngestionStats::default();
    let Some(first) = inputs.first() else {
        return Ok(stats);
    };

    ensure_semantic_embeddings_schema(relational).await?;
    let setup = embeddings::resolve_embedding_setup(embedding_provider.as_ref())?;

    let artefact_ids = inputs
        .iter()
        .map(|input| input.artefact_id.clone())
        .collect::<Vec<_>>();
    let summary_by_artefact_id =
        load_current_semantic_summary_map(relational, &artefact_ids, representation_kind).await?;
    let input_by_artefact_id = inputs
        .iter()
        .map(|input| (input.artefact_id.clone(), input))
        .collect::<HashMap<_, _>>();
    let embedding_inputs = embeddings::build_symbol_embedding_inputs(
        inputs,
        representation_kind,
        &summary_by_artefact_id,
    );
    stats.eligible = embedding_inputs.len();
    delete_stale_current_symbol_embedding_rows_for_path(
        relational,
        &first.repo_id,
        path,
        content_id,
        representation_kind,
        &embedding_inputs
            .iter()
            .map(|input| input.artefact_id.clone())
            .collect::<Vec<_>>(),
    )
    .await?;

    for input in embedding_inputs {
        let input_metadata = input_by_artefact_id
            .get(&input.artefact_id)
            .copied()
            .ok_or_else(|| {
                anyhow::anyhow!("missing current semantic input for `{}`", input.artefact_id)
            })?;
        let next_input_hash =
            embeddings::build_symbol_embedding_input_hash(&input, embedding_provider.as_ref());
        let state = load_current_symbol_embedding_index_state(
            relational,
            &input.artefact_id,
            input.representation_kind,
            &setup.setup_fingerprint,
        )
        .await?;
        if !embeddings::symbol_embeddings_require_reindex(&state, &next_input_hash) {
            stats.skipped += 1;
            continue;
        }
        let input = input.clone();
        let embedding_provider = Arc::clone(&embedding_provider);
        let row = tokio::task::spawn_blocking(move || {
            embeddings::build_symbol_embedding_row(&input, embedding_provider.as_ref())
        })
        .await
        .context("building current semantic embedding row on blocking worker")??;
        persist_current_symbol_embedding_row(relational, input_metadata, path, content_id, &row)
            .await?;
        stats.upserted += 1;
    }

    Ok(stats)
}

pub(crate) async fn ensure_semantic_embeddings_schema(
    relational: &RelationalStorage,
) -> Result<()> {
    init_sqlite_semantic_embeddings_schema(relational.sqlite_path()).await?;
    if let Some(remote_client) = relational.remote_client() {
        init_postgres_semantic_embeddings_schema(remote_client).await?;
    }
    Ok(())
}

#[allow(dead_code)]
pub(crate) async fn clear_repo_symbol_embedding_rows(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<()> {
    ensure_semantic_embeddings_schema(relational).await?;
    relational
        .exec_batch_transactional(&[
            format!(
                "DELETE FROM symbol_embeddings WHERE repo_id = '{}'",
                esc_pg(repo_id),
            ),
            format!(
                "DELETE FROM symbol_embeddings_current WHERE repo_id = '{}'",
                esc_pg(repo_id),
            ),
        ])
        .await
}

#[allow(dead_code)]
pub(crate) async fn clear_current_symbol_embedding_rows_for_path(
    relational: &RelationalStorage,
    repo_id: &str,
    path: &str,
) -> Result<()> {
    ensure_semantic_embeddings_schema(relational).await?;
    let sql = format!(
        "DELETE FROM symbol_embeddings_current WHERE repo_id = '{}' AND path = '{}'",
        esc_pg(repo_id),
        esc_pg(path),
    );
    relational.exec(&sql).await
}

pub(crate) async fn clear_repo_active_embedding_setup(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<()> {
    ensure_semantic_embeddings_schema(relational).await?;
    let sql = format!(
        "DELETE FROM semantic_clone_embedding_setup_state WHERE repo_id = '{}'",
        esc_pg(repo_id),
    );
    relational.exec(&sql).await
}

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

pub(crate) async fn persist_active_embedding_setup(
    relational: &RelationalStorage,
    repo_id: &str,
    active_state: &embeddings::ActiveEmbeddingRepresentationState,
) -> Result<()> {
    ensure_semantic_embeddings_schema(relational).await?;
    persist_embedding_setup(relational, &active_state.setup).await?;
    let sql = build_active_embedding_setup_persist_sql(repo_id, active_state);
    relational.exec(&sql).await
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

pub(crate) async fn refresh_current_repo_symbol_embeddings_and_clone_edges(
    relational: &RelationalStorage,
    repo_root: &Path,
    repo_id: &str,
    summary_provider: Arc<dyn semantic::SemanticSummaryProvider>,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    embedding_provider: Arc<dyn EmbeddingProvider>,
) -> Result<CurrentRepoEmbeddingRefreshResult> {
    ensure_semantic_embeddings_schema(relational).await?;
    let setup = embeddings::resolve_embedding_setup(embedding_provider.as_ref())?;
    let current_inputs =
        super::load_semantic_feature_inputs_for_current_repo(relational, repo_root, repo_id)
            .await?;
    let semantic_feature_stats =
        super::upsert_semantic_feature_rows(relational, &current_inputs, summary_provider).await?;
    let embedding_stats = upsert_symbol_embedding_rows(
        relational,
        &current_inputs,
        representation_kind,
        embedding_provider,
    )
    .await?;
    if embedding_stats.eligible == 0 {
        return Ok(CurrentRepoEmbeddingRefreshResult {
            semantic_feature_stats,
            embedding_stats,
            clone_build: Default::default(),
        });
    }
    persist_active_embedding_setup(
        relational,
        repo_id,
        &embeddings::ActiveEmbeddingRepresentationState::new(representation_kind, setup),
    )
    .await?;
    let clone_build = if representation_kind == embeddings::EmbeddingRepresentationKind::Code {
        crate::capability_packs::semantic_clones::pipeline::rebuild_symbol_clone_edges(
            relational, repo_id,
        )
        .await?
    } else {
        Default::default()
    };

    Ok(CurrentRepoEmbeddingRefreshResult {
        semantic_feature_stats,
        embedding_stats,
        clone_build,
    })
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

async fn load_symbol_embedding_index_state(
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

async fn load_current_symbol_embedding_index_state(
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

async fn load_semantic_summary_map(
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
async fn load_current_semantic_summary_map(
    relational: &RelationalStorage,
    artefact_ids: &[String],
    representation_kind: embeddings::EmbeddingRepresentationKind,
) -> Result<HashMap<String, String>> {
    load_semantic_summary_map_from_table(
        relational,
        artefact_ids,
        "symbol_semantics_current",
        representation_kind,
    )
    .await
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

    let rows = relational
        .query_rows(&build_semantic_summary_lookup_sql(artefact_ids, table))
        .await?;
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

async fn persist_symbol_embedding_row(
    relational: &RelationalStorage,
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<()> {
    persist_embedding_setup(
        relational,
        &embeddings::EmbeddingSetup {
            provider: row.provider.clone(),
            model: row.model.clone(),
            dimension: row.dimension,
            setup_fingerprint: row.setup_fingerprint.clone(),
        },
    )
    .await?;
    let sql = build_sqlite_symbol_embedding_persist_sql(row)?;
    relational.exec(&sql).await
}

#[allow(dead_code)]
async fn persist_current_symbol_embedding_row(
    relational: &RelationalStorage,
    input: &semantic::SemanticFeatureInput,
    path: &str,
    content_id: &str,
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<()> {
    persist_embedding_setup(
        relational,
        &embeddings::EmbeddingSetup {
            provider: row.provider.clone(),
            model: row.model.clone(),
            dimension: row.dimension,
            setup_fingerprint: row.setup_fingerprint.clone(),
        },
    )
    .await?;
    let sql = build_current_symbol_embedding_persist_sql(input, path, content_id, row)?;
    relational.exec(&sql).await
}

fn build_symbol_embedding_index_state_sql(
    artefact_id: &str,
    table: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    setup_fingerprint: &str,
) -> String {
    format!(
        "SELECT embedding_input_hash AS embedding_hash \
FROM {table} \
WHERE artefact_id = '{artefact_id}' AND representation_kind = '{representation_kind}' \
  AND setup_fingerprint = '{setup_fingerprint}'",
        table = table,
        artefact_id = esc_pg(artefact_id),
        representation_kind = esc_pg(&representation_kind.to_string()),
        setup_fingerprint = esc_pg(setup_fingerprint),
    )
}

async fn delete_stale_current_symbol_embedding_rows_for_path(
    relational: &RelationalStorage,
    repo_id: &str,
    path: &str,
    content_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    keep_artefact_ids: &[String],
) -> Result<()> {
    let extra_delete_clause = if keep_artefact_ids.is_empty() {
        " OR 1 = 1".to_string()
    } else {
        format!(
            " OR artefact_id NOT IN ({})",
            sql_string_list_pg(keep_artefact_ids)
        )
    };
    let sql = format!(
        "DELETE FROM symbol_embeddings_current \
WHERE repo_id = '{repo_id}' AND path = '{path}' AND {representation_predicate} \
  AND (content_id <> '{content_id}'{extra_delete_clause})",
        repo_id = esc_pg(repo_id),
        path = esc_pg(path),
        content_id = esc_pg(content_id),
        representation_predicate =
            representation_kind_sql_predicate("representation_kind", representation_kind),
        extra_delete_clause = extra_delete_clause,
    );
    relational.exec(&sql).await
}

fn build_active_embedding_setup_lookup_sql(
    repo_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
) -> String {
    format!(
        "SELECT representation_kind, provider, model, dimension, setup_fingerprint \
FROM semantic_clone_embedding_setup_state \
WHERE repo_id = '{repo_id}' AND {representation_predicate}",
        repo_id = esc_pg(repo_id),
        representation_predicate =
            representation_kind_sql_predicate("representation_kind", representation_kind),
    )
}

fn build_current_repo_embedding_states_sql(
    repo_id: &str,
    representation_kind: Option<embeddings::EmbeddingRepresentationKind>,
) -> String {
    let representation_filter = representation_kind
        .map(|kind| {
            format!(
                "AND {}",
                representation_kind_sql_predicate("e.representation_kind", kind)
            )
        })
        .unwrap_or_default();
    format!(
        "SELECT representation_kind, provider, model, dimension, setup_fingerprint \
FROM ( \
    SELECT e.representation_kind AS representation_kind, e.provider AS provider, e.model AS model, e.dimension AS dimension, e.setup_fingerprint AS setup_fingerprint \
    FROM artefacts_current a \
    JOIN symbol_embeddings_current e ON e.repo_id = a.repo_id AND e.artefact_id = a.artefact_id \
    WHERE a.repo_id = '{repo_id}' {representation_filter} \
    UNION \
    SELECT e.representation_kind AS representation_kind, e.provider AS provider, e.model AS model, e.dimension AS dimension, e.setup_fingerprint AS setup_fingerprint \
    FROM artefacts_current a \
    JOIN symbol_embeddings e ON e.repo_id = a.repo_id AND e.artefact_id = a.artefact_id \
    WHERE a.repo_id = '{repo_id}' {representation_filter} \
) setups \
ORDER BY representation_kind, provider, model, dimension, setup_fingerprint",
        repo_id = esc_pg(repo_id),
        representation_filter = representation_filter,
    )
}

fn build_current_repo_semantic_clone_coverage_sql(
    repo_id: &str,
    representation_kind: embeddings::EmbeddingRepresentationKind,
    setup: &embeddings::EmbeddingSetup,
) -> String {
    format!(
        "SELECT \
            (SELECT COUNT(*) FROM artefacts_current a \
             WHERE a.repo_id = '{repo_id}' \
               AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import') AS eligible_current_artefacts, \
            (SELECT COUNT(DISTINCT a.artefact_id) FROM artefacts_current a \
             JOIN symbol_semantics ss ON ss.artefact_id = a.artefact_id \
             JOIN symbol_features sf ON sf.artefact_id = a.artefact_id \
             JOIN symbol_embeddings e ON e.artefact_id = a.artefact_id \
             WHERE a.repo_id = '{repo_id}' \
               AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import' \
               AND {representation_predicate} \
               AND e.provider = '{provider}' \
               AND e.model = '{model}' \
               AND e.dimension = {dimension}) AS fully_indexed_current_artefacts",
        repo_id = esc_pg(repo_id),
        representation_predicate =
            representation_kind_sql_predicate("e.representation_kind", representation_kind),
        provider = esc_pg(&setup.provider),
        model = esc_pg(&setup.model),
        dimension = setup.dimension,
    )
}

fn build_active_embedding_setup_persist_sql(
    repo_id: &str,
    active_state: &embeddings::ActiveEmbeddingRepresentationState,
) -> String {
    let setup = &active_state.setup;
    format!(
        "INSERT INTO semantic_clone_embedding_setup_state (repo_id, representation_kind, provider, model, dimension, setup_fingerprint) \
VALUES ('{repo_id}', '{representation_kind}', '{provider}', '{model}', {dimension}, '{setup_fingerprint}') \
ON CONFLICT (repo_id, representation_kind) DO UPDATE SET provider = excluded.provider, model = excluded.model, dimension = excluded.dimension, setup_fingerprint = excluded.setup_fingerprint, updated_at = CURRENT_TIMESTAMP",
        repo_id = esc_pg(repo_id),
        representation_kind = esc_pg(&active_state.representation_kind.to_string()),
        provider = esc_pg(&setup.provider),
        model = esc_pg(&setup.model),
        dimension = setup.dimension,
        setup_fingerprint = esc_pg(&setup.setup_fingerprint),
    )
}

fn parse_symbol_embedding_index_state_rows(
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
        _ => None,
    }
}

fn build_semantic_summary_lookup_sql(artefact_ids: &[String], table: &str) -> String {
    format!(
        "SELECT artefact_id, docstring_summary, llm_summary, template_summary, summary, source_model \
FROM {table} \
WHERE artefact_id IN ({})",
        sql_string_list_pg(artefact_ids),
        table = table,
    )
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

#[cfg(test)]
fn build_postgres_symbol_embedding_persist_sql(
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<String> {
    let embedding_expr = sql_vector_string(&row.embedding)?;
    Ok(format!(
        "INSERT INTO symbol_embeddings (artefact_id, repo_id, blob_sha, representation_kind, setup_fingerprint, provider, model, dimension, embedding_input_hash, embedding) \
VALUES ('{artefact_id}', '{repo_id}', '{blob_sha}', '{representation_kind}', '{setup_fingerprint}', '{provider}', '{model}', {dimension}, '{embedding_input_hash}', {embedding}) \
ON CONFLICT (artefact_id, representation_kind, setup_fingerprint) DO UPDATE SET repo_id = EXCLUDED.repo_id, blob_sha = EXCLUDED.blob_sha, provider = EXCLUDED.provider, model = EXCLUDED.model, dimension = EXCLUDED.dimension, embedding_input_hash = EXCLUDED.embedding_input_hash, embedding = EXCLUDED.embedding, generated_at = now()",
        artefact_id = esc_pg(&row.artefact_id),
        repo_id = esc_pg(&row.repo_id),
        blob_sha = esc_pg(&row.blob_sha),
        representation_kind = esc_pg(&row.representation_kind.to_string()),
        setup_fingerprint = esc_pg(&row.setup_fingerprint),
        provider = esc_pg(&row.provider),
        model = esc_pg(&row.model),
        dimension = row.dimension,
        embedding_input_hash = esc_pg(&row.embedding_input_hash),
        embedding = embedding_expr,
    ))
}

fn build_sqlite_symbol_embedding_persist_sql(
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<String> {
    let embedding_json = sql_json_string(&row.embedding)?;
    Ok(format!(
        "INSERT INTO symbol_embeddings (artefact_id, repo_id, blob_sha, representation_kind, setup_fingerprint, provider, model, dimension, embedding_input_hash, embedding) \
VALUES ('{artefact_id}', '{repo_id}', '{blob_sha}', '{representation_kind}', '{setup_fingerprint}', '{provider}', '{model}', {dimension}, '{embedding_input_hash}', '{embedding}') \
ON CONFLICT (artefact_id, representation_kind, setup_fingerprint) DO UPDATE SET repo_id = excluded.repo_id, blob_sha = excluded.blob_sha, provider = excluded.provider, model = excluded.model, dimension = excluded.dimension, embedding_input_hash = excluded.embedding_input_hash, embedding = excluded.embedding, generated_at = CURRENT_TIMESTAMP",
        artefact_id = esc_pg(&row.artefact_id),
        repo_id = esc_pg(&row.repo_id),
        blob_sha = esc_pg(&row.blob_sha),
        representation_kind = esc_pg(&row.representation_kind.to_string()),
        setup_fingerprint = esc_pg(&row.setup_fingerprint),
        provider = esc_pg(&row.provider),
        model = esc_pg(&row.model),
        dimension = row.dimension,
        embedding_input_hash = esc_pg(&row.embedding_input_hash),
        embedding = embedding_json,
    ))
}

#[allow(dead_code)]
fn build_current_symbol_embedding_persist_sql(
    input: &semantic::SemanticFeatureInput,
    path: &str,
    content_id: &str,
    row: &embeddings::SymbolEmbeddingRow,
) -> Result<String> {
    let embedding_json = sql_json_string(&row.embedding)?;
    let symbol_id_sql = input
        .symbol_id
        .as_deref()
        .map(|value| format!("'{}'", esc_pg(value)))
        .unwrap_or_else(|| "NULL".to_string());
    Ok(format!(
        "INSERT INTO symbol_embeddings_current (artefact_id, repo_id, path, content_id, symbol_id, representation_kind, setup_fingerprint, provider, model, dimension, embedding_input_hash, embedding) \
VALUES ('{artefact_id}', '{repo_id}', '{path}', '{content_id}', {symbol_id}, '{representation_kind}', '{setup_fingerprint}', '{provider}', '{model}', {dimension}, '{embedding_input_hash}', '{embedding}') \
ON CONFLICT (artefact_id, representation_kind, setup_fingerprint) DO UPDATE SET repo_id = excluded.repo_id, path = excluded.path, content_id = excluded.content_id, symbol_id = excluded.symbol_id, provider = excluded.provider, model = excluded.model, dimension = excluded.dimension, embedding_input_hash = excluded.embedding_input_hash, embedding = excluded.embedding, generated_at = CURRENT_TIMESTAMP",
        artefact_id = esc_pg(&row.artefact_id),
        repo_id = esc_pg(&row.repo_id),
        path = esc_pg(path),
        content_id = esc_pg(content_id),
        symbol_id = symbol_id_sql,
        representation_kind = esc_pg(&row.representation_kind.to_string()),
        setup_fingerprint = esc_pg(&row.setup_fingerprint),
        provider = esc_pg(&row.provider),
        model = esc_pg(&row.model),
        dimension = row.dimension,
        embedding_input_hash = esc_pg(&row.embedding_input_hash),
        embedding = embedding_json,
    ))
}

async fn persist_embedding_setup(
    relational: &RelationalStorage,
    setup: &embeddings::EmbeddingSetup,
) -> Result<()> {
    relational
        .exec(&build_embedding_setup_persist_sql(setup))
        .await
}

fn build_embedding_setup_persist_sql(setup: &embeddings::EmbeddingSetup) -> String {
    format!(
        "INSERT INTO semantic_embedding_setups (setup_fingerprint, provider, model, dimension) \
VALUES ('{setup_fingerprint}', '{provider}', '{model}', {dimension}) \
ON CONFLICT (setup_fingerprint) DO UPDATE SET provider = excluded.provider, model = excluded.model, dimension = excluded.dimension",
        setup_fingerprint = esc_pg(&setup.setup_fingerprint),
        provider = esc_pg(&setup.provider),
        model = esc_pg(&setup.model),
        dimension = setup.dimension,
    )
}

fn representation_kind_sql_predicate(
    column: &str,
    kind: embeddings::EmbeddingRepresentationKind,
) -> String {
    let values = kind
        .storage_values()
        .iter()
        .map(|value| format!("'{}'", esc_pg(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{column} IN ({values})")
}

#[cfg(test)]
fn sql_vector_string(values: &[f32]) -> Result<String> {
    let json = sql_json_string(values)?;
    Ok(format!("'{json}'::vector"))
}

fn sql_json_string(values: &[f32]) -> Result<String> {
    if values.is_empty() {
        bail!("cannot persist empty embedding vector");
    }

    for value in values {
        if !value.is_finite() {
            bail!("cannot persist embedding vector containing non-finite values");
        }
    }

    Ok(esc_pg(&serde_json::to_string(values)?))
}

#[cfg(test)]
#[path = "stage_embeddings_tests.rs"]
mod semantic_embedding_persistence_tests;
