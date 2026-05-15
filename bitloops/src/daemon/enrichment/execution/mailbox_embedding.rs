use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID;
use crate::capability_packs::semantic_clones::embeddings::{
    ActiveEmbeddingRepresentationState, EmbeddingRepresentationKind,
    build_symbol_embedding_input_hash, build_symbol_embedding_inputs, build_symbol_embedding_rows,
    resolve_embedding_setup, symbol_embeddings_require_reindex,
};
use crate::capability_packs::semantic_clones::features::{
    SemanticFeatureHashKey, build_symbol_feature_rows,
};
use crate::capability_packs::semantic_clones::runtime_config::{
    EmbeddingProviderMode, SummaryProviderMode, resolve_embedding_provider,
    resolve_semantic_clones_config, resolve_summary_provider,
};
use crate::capability_packs::semantic_clones::stage_embeddings::{
    load_current_symbol_embedding_index_states, load_symbol_embedding_index_states,
};
use crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX;
use crate::capability_packs::semantic_clones::vector_backend::{
    build_postgres_pgvector_partial_index_sql, build_sqlite_current_vec_table_init_statements,
    build_sqlite_current_vec_upsert_statements,
    build_sqlite_stale_current_rows_for_path_delete_statements,
};
use crate::capability_packs::semantic_clones::{
    build_active_embedding_setup_persist_sql,
    build_conditional_current_symbol_feature_persist_rows_sql,
    build_current_symbol_embedding_persist_sql,
    build_delete_stale_current_symbol_embedding_rows_for_path_sql,
    build_embedding_setup_persist_sql, build_postgres_current_symbol_embedding_persist_sql,
    build_postgres_symbol_embedding_persist_sql, build_sqlite_symbol_embedding_persist_sql,
    build_symbol_feature_persist_rows_sql, load_current_semantic_summary_map,
};
use crate::config::resolve_store_backend_config_for_repo;
use crate::host::devql::{
    DevqlConfig, RelationalPrimaryBackend, RelationalStorage, build_capability_host, esc_pg,
};
use crate::host::runtime_store::{
    CapabilityWorkplaneJobInsert, SemanticEmbeddingMailboxItemInsert, SemanticMailboxItemKind,
};

use super::super::semantic_writer::{CommitEmbeddingBatchRequest, SemanticBatchRepoContext};
use super::super::workplane::{
    ClaimedEmbeddingMailboxBatch, SEMANTIC_EMBEDDING_MAILBOX_BATCH_SIZE,
    repo_identity_from_runtime_metadata,
};
use super::helpers::{
    dedupe_inputs_by_artefact_id, load_current_semantic_inputs, payload_artefact_ids_from_value,
    select_current_semantic_input_scope,
};

pub(crate) struct PreparedEmbeddingMailboxBatch {
    pub commit: CommitEmbeddingBatchRequest,
    pub expanded_count: usize,
    pub attempts: u32,
    pub timings: EmbeddingBatchPrepareTimings,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct EmbeddingBatchPrepareTimings {
    pub config_ms: u64,
    pub input_ms: u64,
    pub summary_ms: u64,
    pub freshness_ms: u64,
    pub embedding_ms: u64,
    pub sql_ms: u64,
    pub setup_ms: u64,
    pub total_ms: u64,
}

fn acked_embedding_mailbox_batch(
    batch: &ClaimedEmbeddingMailboxBatch,
    config_ms: u64,
    total_ms: u64,
) -> PreparedEmbeddingMailboxBatch {
    PreparedEmbeddingMailboxBatch {
        commit: CommitEmbeddingBatchRequest {
            repo: SemanticBatchRepoContext {
                repo_id: batch.repo_id.clone(),
                repo_root: batch.repo_root.clone(),
                config_root: batch.config_root.clone(),
            },
            lease_token: batch.lease_token.clone(),
            embedding_statements: Vec::new(),
            setup_statements: Vec::new(),
            remote_embedding_statements: Vec::new(),
            remote_setup_statements: Vec::new(),
            clone_rebuild_signal: None,
            replacement_backfill_item: None,
            acked_item_ids: batch
                .items
                .iter()
                .map(|item| item.item_id.clone())
                .collect(),
        },
        expanded_count: 0,
        attempts: batch
            .items
            .iter()
            .map(|item| item.attempts)
            .max()
            .unwrap_or(0),
        timings: EmbeddingBatchPrepareTimings {
            config_ms,
            total_ms,
            ..EmbeddingBatchPrepareTimings::default()
        },
    }
}

pub(crate) async fn prepare_embedding_mailbox_batch(
    batch: &ClaimedEmbeddingMailboxBatch,
) -> Result<PreparedEmbeddingMailboxBatch> {
    let total_started = Instant::now();
    let config_started = Instant::now();
    let repo = repo_identity_from_runtime_metadata(&batch.repo_root, &batch.repo_id);
    let cfg = DevqlConfig::from_roots(batch.config_root.clone(), batch.repo_root.clone(), repo)?;
    let backends = resolve_store_backend_config_for_repo(&batch.config_root)?;
    let relational =
        RelationalStorage::connect(&cfg, &backends.relational, "semantic embedding batch").await?;
    let capability_host = build_capability_host(&batch.repo_root, cfg.repo.clone())?;
    let inference = capability_host.inference_for_capability(SEMANTIC_CLONES_CAPABILITY_ID);
    let config =
        resolve_semantic_clones_config(&capability_host.config_view(SEMANTIC_CLONES_CAPABILITY_ID));
    let mailbox_intent =
        crate::capability_packs::semantic_clones::workplane::load_effective_mailbox_intent_for_repo(
            &batch.repo_root,
            &config,
        )?;
    let representation_active = match batch.representation_kind {
        EmbeddingRepresentationKind::Code | EmbeddingRepresentationKind::Identity => {
            mailbox_intent.code_embeddings_active
        }
        EmbeddingRepresentationKind::Summary => mailbox_intent.summary_embeddings_active,
    };
    if !representation_active {
        return Ok(acked_embedding_mailbox_batch(
            batch,
            elapsed_ms(config_started),
            elapsed_ms(total_started),
        ));
    }
    let selection = resolve_embedding_provider(
        &config,
        &inference,
        batch.representation_kind,
        EmbeddingProviderMode::ConfiguredStrict,
    )?;
    let Some(provider) = selection.provider else {
        return Ok(acked_embedding_mailbox_batch(
            batch,
            elapsed_ms(config_started),
            elapsed_ms(total_started),
        ));
    };
    let setup = resolve_embedding_setup(provider.as_ref())?;
    let code_feature_hash_provider = if batch.representation_kind
        == EmbeddingRepresentationKind::Code
    {
        Some(SemanticFeatureHashKey::from_summary_provider_cache_key(
            resolve_summary_provider(&config, &inference, SummaryProviderMode::ConfiguredStrict)?
                .provider
                .cache_key(),
        ))
    } else {
        None
    };
    let config_ms = elapsed_ms(config_started);

    let input_started = Instant::now();
    let contains_repo_wide_backfill = batch.items.iter().any(|item| {
        item.item_kind == SemanticMailboxItemKind::RepoBackfill && item.payload_json.is_none()
    });
    let explicit_artefact_ids =
        contains_repo_wide_backfill.then(|| explicit_artefact_ids_from_batch(&batch.items));
    let current_input_selection =
        (!contains_repo_wide_backfill).then(|| select_current_semantic_input_scope(&batch.items));
    let requested_artefact_ids = if contains_repo_wide_backfill {
        explicit_artefact_ids.as_deref()
    } else {
        current_input_selection
            .as_ref()
            .and_then(|selection| selection.requested_artefact_ids())
    };
    let current_inputs = load_current_semantic_inputs(
        &relational,
        &batch.repo_root,
        &batch.repo_id,
        requested_artefact_ids,
    )
    .await?;
    let mut current_by_artefact = current_inputs
        .iter()
        .cloned()
        .map(|input| (input.artefact_id.clone(), input))
        .collect::<HashMap<_, _>>();

    let mut expanded_inputs = Vec::new();
    let mut replacement_backfill_item = None;
    let mut repo_wide_artefact_ids = None;
    for item in &batch.items {
        match item.item_kind {
            SemanticMailboxItemKind::Artefact => {
                if let Some(artefact_id) = item.artefact_id.as_ref()
                    && let Some(input) = current_by_artefact.get(artefact_id)
                {
                    expanded_inputs.push(input.clone());
                }
            }
            SemanticMailboxItemKind::RepoBackfill => {
                let requested_ids = item
                    .payload_json
                    .as_ref()
                    .map(payload_artefact_ids_from_value);
                let mut selected = match requested_ids {
                    Some(requested_ids) => requested_ids
                        .iter()
                        .filter_map(|artefact_id| current_by_artefact.get(artefact_id).cloned())
                        .collect::<Vec<_>>(),
                    None => {
                        let artefact_ids = match repo_wide_artefact_ids.as_ref() {
                            Some(ids) => ids,
                            None => {
                                repo_wide_artefact_ids = Some(
                                    load_current_embedding_backfill_artefact_ids(
                                        &relational,
                                        &batch.repo_id,
                                    )
                                    .await?,
                                );
                                repo_wide_artefact_ids
                                    .as_ref()
                                    .expect("repo-wide artefact ids loaded above")
                            }
                        };
                        let selected_ids = artefact_ids
                            .iter()
                            .take(SEMANTIC_EMBEDDING_MAILBOX_BATCH_SIZE)
                            .cloned()
                            .collect::<Vec<_>>();
                        if artefact_ids.len() > SEMANTIC_EMBEDDING_MAILBOX_BATCH_SIZE {
                            let remaining_ids = artefact_ids
                                .iter()
                                .skip(SEMANTIC_EMBEDDING_MAILBOX_BATCH_SIZE)
                                .cloned()
                                .collect::<Vec<_>>();
                            replacement_backfill_item =
                                Some(SemanticEmbeddingMailboxItemInsert::new(
                                    item.init_session_id.clone(),
                                    batch.representation_kind.to_string(),
                                    SemanticMailboxItemKind::RepoBackfill,
                                    None,
                                    Some(serde_json::to_value(remaining_ids)?),
                                    item.dedupe_key.clone(),
                                ));
                        }
                        let selected_inputs = load_current_semantic_inputs(
                            &relational,
                            &batch.repo_root,
                            &batch.repo_id,
                            Some(&selected_ids),
                        )
                        .await?;
                        for input in &selected_inputs {
                            current_by_artefact.insert(input.artefact_id.clone(), input.clone());
                        }
                        selected_inputs
                    }
                };
                if selected.len() > SEMANTIC_EMBEDDING_MAILBOX_BATCH_SIZE {
                    let remaining_ids = selected
                        .split_off(SEMANTIC_EMBEDDING_MAILBOX_BATCH_SIZE)
                        .into_iter()
                        .map(|input| input.artefact_id)
                        .collect::<Vec<_>>();
                    replacement_backfill_item = Some(SemanticEmbeddingMailboxItemInsert::new(
                        item.init_session_id.clone(),
                        batch.representation_kind.to_string(),
                        SemanticMailboxItemKind::RepoBackfill,
                        None,
                        Some(serde_json::to_value(remaining_ids)?),
                        item.dedupe_key.clone(),
                    ));
                }
                expanded_inputs.extend(selected);
            }
        }
    }
    dedupe_inputs_by_artefact_id(&mut expanded_inputs);
    let input_ms = elapsed_ms(input_started);

    let summary_started = Instant::now();
    let summary_map = load_current_semantic_summary_map(
        &relational,
        &expanded_inputs
            .iter()
            .map(|input| input.artefact_id.clone())
            .collect::<Vec<_>>(),
        batch.representation_kind,
    )
    .await?;
    let embedding_inputs =
        build_symbol_embedding_inputs(&expanded_inputs, batch.representation_kind, &summary_map);
    let should_prune_stale_current_rows = batch
        .items
        .iter()
        .all(|item| item.item_kind == SemanticMailboxItemKind::Artefact);
    let mut current_paths_by_content = BTreeSet::<(String, String)>::new();
    let mut keep_current_artefact_ids_by_path_content =
        BTreeMap::<(String, String), Vec<String>>::new();
    if should_prune_stale_current_rows {
        for input in current_by_artefact.values() {
            current_paths_by_content.insert((input.path.clone(), input.blob_sha.clone()));
        }
        for (path, content_id) in &current_paths_by_content {
            keep_current_artefact_ids_by_path_content
                .entry((path.clone(), content_id.clone()))
                .or_default()
                .extend(
                    load_current_embedding_artefact_ids_for_path_content(
                        &relational,
                        &batch.repo_id,
                        path,
                        content_id,
                    )
                    .await?,
                );
        }
        for keep_artefact_ids in keep_current_artefact_ids_by_path_content.values_mut() {
            keep_artefact_ids.sort();
            keep_artefact_ids.dedup();
        }
    }
    let summary_ms = elapsed_ms(summary_started);

    let mut embedding_statements = Vec::new();
    let mut remote_embedding_statements = Vec::new();
    let mut repaired_feature_projection = false;
    if batch.representation_kind == EmbeddingRepresentationKind::Code {
        let feature_hash_provider = code_feature_hash_provider
            .as_ref()
            .expect("code embedding batches resolve a feature hash key")
            .clone();
        for input in &expanded_inputs {
            let input_for_rows = input.clone();
            let hash_key_for_rows = feature_hash_provider.clone();
            let rows = tokio::task::spawn_blocking(move || {
                build_symbol_feature_rows(&input_for_rows, &hash_key_for_rows)
            })
            .await
            .context("building code embedding feature rows on blocking worker")?;
            embedding_statements.push(build_symbol_feature_persist_rows_sql(
                &rows,
                relational.dialect(),
            )?);
            embedding_statements.push(build_conditional_current_symbol_feature_persist_rows_sql(
                &rows,
                input,
                relational.dialect(),
            )?);
            repaired_feature_projection = true;
        }
    }
    let mut upserted_any = false;
    if !embedding_inputs.is_empty() {
        embedding_statements.push(build_embedding_setup_persist_sql(&setup));
        if relational.primary_backend() == RelationalPrimaryBackend::Postgres {
            remote_embedding_statements.push(build_embedding_setup_persist_sql(&setup));
        }
    }
    if should_prune_stale_current_rows {
        for (path, content_id) in current_paths_by_content {
            let keep_artefact_ids = keep_current_artefact_ids_by_path_content
                .remove(&(path.clone(), content_id.clone()))
                .unwrap_or_default();
            let delete_sql = build_delete_stale_current_symbol_embedding_rows_for_path_sql(
                &batch.repo_id,
                &path,
                &content_id,
                batch.representation_kind,
                &keep_artefact_ids,
            );
            embedding_statements.push(delete_sql.clone());
            embedding_statements.extend(
                build_sqlite_stale_current_rows_for_path_delete_statements(
                    &relational,
                    &batch.repo_id,
                    &path,
                    batch.representation_kind,
                    &keep_artefact_ids,
                )
                .await?,
            );
            if relational.primary_backend() == RelationalPrimaryBackend::Postgres {
                remote_embedding_statements.push(delete_sql);
            }
        }
    }
    let freshness_started = Instant::now();
    let embedding_artefact_ids = embedding_inputs
        .iter()
        .map(|input| input.artefact_id.clone())
        .collect::<Vec<_>>();
    let historical_index_states = load_symbol_embedding_index_states(
        &relational,
        &embedding_artefact_ids,
        batch.representation_kind,
        &setup.setup_fingerprint,
    )
    .await?;
    let current_index_states = load_current_symbol_embedding_index_states(
        &relational,
        &embedding_artefact_ids,
        batch.representation_kind,
        &setup.setup_fingerprint,
    )
    .await?;
    let mut reindex_inputs = Vec::new();
    for embedding_input in embedding_inputs {
        let historical_state = historical_index_states
            .get(&embedding_input.artefact_id)
            .cloned()
            .unwrap_or_default();
        let current_state = current_index_states
            .get(&embedding_input.artefact_id)
            .cloned()
            .unwrap_or_default();
        let next_input_hash =
            build_symbol_embedding_input_hash(&embedding_input, provider.as_ref());
        let historical_fresh =
            !symbol_embeddings_require_reindex(&historical_state, &next_input_hash);
        let current_fresh = !symbol_embeddings_require_reindex(&current_state, &next_input_hash);
        if historical_fresh && current_fresh {
            continue;
        }
        reindex_inputs.push(embedding_input);
    }
    let freshness_ms = elapsed_ms(freshness_started);
    let mut embedding_ms = 0;
    let mut sql_ms = 0;
    if !reindex_inputs.is_empty() {
        let provider_for_rows = Arc::clone(&provider);
        let embedding_started = Instant::now();
        let rows = tokio::task::spawn_blocking(move || {
            build_symbol_embedding_rows(&reindex_inputs, provider_for_rows.as_ref())
        })
        .await
        .context("building embedding rows on blocking worker")??;
        embedding_ms = elapsed_ms(embedding_started);
        let sql_started = Instant::now();
        let mut sqlite_vec_dimensions = BTreeSet::new();
        let mut sqlite_vec_upsert_statements = Vec::new();
        let mut postgres_index_dimensions = BTreeSet::new();
        for row in rows {
            embedding_statements.push(build_sqlite_symbol_embedding_persist_sql(&row)?);
            if let Some(current_input) = current_by_artefact.get(&row.artefact_id) {
                embedding_statements.push(build_current_symbol_embedding_persist_sql(
                    current_input,
                    &current_input.path,
                    &current_input.blob_sha,
                    &row,
                )?);
                sqlite_vec_dimensions.insert(row.dimension);
                sqlite_vec_upsert_statements.extend(build_sqlite_current_vec_upsert_statements(
                    &current_input.path,
                    &row,
                )?);
            }
            if relational.primary_backend() == RelationalPrimaryBackend::Postgres {
                remote_embedding_statements
                    .push(build_postgres_symbol_embedding_persist_sql(&row)?);
                postgres_index_dimensions.insert(row.dimension);
                if let Some(current_input) = current_by_artefact.get(&row.artefact_id) {
                    remote_embedding_statements.push(
                        build_postgres_current_symbol_embedding_persist_sql(
                            current_input,
                            &current_input.path,
                            &current_input.blob_sha,
                            &row,
                        )?,
                    );
                }
            }
        }
        for dimension in sqlite_vec_dimensions {
            embedding_statements.extend(
                build_sqlite_current_vec_table_init_statements(&relational, dimension).await?,
            );
        }
        embedding_statements.extend(sqlite_vec_upsert_statements);
        if relational.primary_backend() == RelationalPrimaryBackend::Postgres {
            for dimension in postgres_index_dimensions {
                remote_embedding_statements.push(build_postgres_pgvector_partial_index_sql(
                    "symbol_embeddings",
                    dimension,
                ));
                remote_embedding_statements.push(build_postgres_pgvector_partial_index_sql(
                    "symbol_embeddings_current",
                    dimension,
                ));
            }
        }
        sql_ms = elapsed_ms(sql_started);
        upserted_any = true;
    }

    let mut setup_statements = Vec::new();
    let mut remote_setup_statements = Vec::new();
    let setup_started = Instant::now();
    if upserted_any {
        setup_statements.push(build_active_embedding_setup_persist_sql(
            &batch.repo_id,
            &ActiveEmbeddingRepresentationState::new(batch.representation_kind, setup.clone()),
        ));
        if relational.primary_backend() == RelationalPrimaryBackend::Postgres {
            remote_setup_statements.push(build_active_embedding_setup_persist_sql(
                &batch.repo_id,
                &ActiveEmbeddingRepresentationState::new(batch.representation_kind, setup.clone()),
            ));
        }
    }
    let setup_ms = elapsed_ms(setup_started);

    let clone_rebuild_signal = if (upserted_any || repaired_feature_projection)
        && matches!(
            batch.representation_kind,
            EmbeddingRepresentationKind::Code | EmbeddingRepresentationKind::Summary
        ) {
        Some(CapabilityWorkplaneJobInsert::new(
            SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
            None,
            Some(SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX.to_string()),
            serde_json::to_value(
                crate::capability_packs::semantic_clones::workplane::SemanticClonesMailboxPayload::RepoBackfill {
                    work_item_count: Some(1),
                    artefact_ids: None,
                },
            )?,
        ))
    } else {
        None
    };

    Ok(PreparedEmbeddingMailboxBatch {
        commit: CommitEmbeddingBatchRequest {
            repo: SemanticBatchRepoContext {
                repo_id: batch.repo_id.clone(),
                repo_root: batch.repo_root.clone(),
                config_root: batch.config_root.clone(),
            },
            lease_token: batch.lease_token.clone(),
            embedding_statements,
            setup_statements,
            remote_embedding_statements,
            remote_setup_statements,
            clone_rebuild_signal,
            replacement_backfill_item,
            acked_item_ids: batch
                .items
                .iter()
                .map(|item| item.item_id.clone())
                .collect(),
        },
        expanded_count: expanded_inputs.len(),
        attempts: batch
            .items
            .iter()
            .map(|item| item.attempts)
            .max()
            .unwrap_or(0),
        timings: EmbeddingBatchPrepareTimings {
            config_ms,
            input_ms,
            summary_ms,
            freshness_ms,
            embedding_ms,
            sql_ms,
            setup_ms,
            total_ms: elapsed_ms(total_started),
        },
    })
}

fn explicit_artefact_ids_from_batch(
    items: &[crate::host::runtime_store::SemanticEmbeddingMailboxItemRecord],
) -> Vec<String> {
    let mut ids = Vec::new();
    for item in items {
        match item.item_kind {
            SemanticMailboxItemKind::Artefact => {
                if let Some(artefact_id) = item.artefact_id.as_ref() {
                    ids.push(artefact_id.clone());
                }
            }
            SemanticMailboxItemKind::RepoBackfill => {
                if let Some(payload) = item.payload_json.as_ref() {
                    ids.extend(payload_artefact_ids_from_value(payload));
                }
            }
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

async fn load_current_embedding_backfill_artefact_ids(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<Vec<String>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT current.artefact_id \
FROM artefacts_current current \
JOIN current_file_state state ON state.repo_id = current.repo_id AND state.path = current.path \
WHERE current.repo_id = '{}' \
  AND state.analysis_mode = 'code' \
  AND LOWER(COALESCE(current.canonical_kind, COALESCE(current.language_kind, 'symbol'))) <> 'import' \
ORDER BY current.path, current.start_line, current.symbol_id, COALESCE(current.start_byte, 0), current.artefact_id",
            esc_pg(repo_id),
        ))
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row: Value| {
            row.get("artefact_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect())
}

async fn load_current_embedding_artefact_ids_for_path_content(
    relational: &RelationalStorage,
    repo_id: &str,
    path: &str,
    content_id: &str,
) -> Result<Vec<String>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT DISTINCT current.artefact_id AS artefact_id \
FROM artefacts_current current \
JOIN current_file_state state ON state.repo_id = current.repo_id AND state.path = current.path \
WHERE current.repo_id = '{repo_id}' \
  AND current.path = '{path}' \
  AND current.content_id = '{content_id}' \
  AND state.analysis_mode = 'code' \
  AND LOWER(COALESCE(current.canonical_kind, COALESCE(current.language_kind, 'symbol'))) <> 'import' \
ORDER BY current.artefact_id",
            repo_id = esc_pg(repo_id),
            path = esc_pg(path),
            content_id = esc_pg(content_id),
        ))
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row: Value| {
            row.get("artefact_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect())
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}
