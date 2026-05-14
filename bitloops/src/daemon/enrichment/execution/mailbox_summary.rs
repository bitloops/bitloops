use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID;
use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::capability_packs::semantic_clones::features::{
    SemanticFeatureInput, build_semantic_feature_input_hash, semantic_features_require_reindex,
};
use crate::capability_packs::semantic_clones::runtime_config::{
    SummaryProviderMode, embedding_slot_for_representation, resolve_semantic_clones_config,
    resolve_summary_provider,
};
use crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX;
use crate::capability_packs::semantic_clones::{
    build_conditional_current_semantic_persist_rows_sql,
    build_conditional_current_symbol_feature_persist_rows_sql,
    build_delete_current_symbol_semantics_for_artefact_sql,
    build_repair_current_semantic_projection_from_historical_sql,
    build_semantic_get_index_state_sql, build_semantic_persist_rows_sql,
    build_symbol_feature_persist_rows_sql, ensure_required_llm_summary_output,
    parse_semantic_index_state_rows,
};
use crate::config::resolve_store_backend_config_for_repo;
use crate::host::devql::{
    DevqlConfig, RelationalStorage, RelationalStorageRole, build_capability_host, esc_pg,
};
use crate::host::runtime_store::{
    SemanticEmbeddingMailboxItemInsert, SemanticMailboxItemKind, SemanticSummaryMailboxItemInsert,
};

use super::super::semantic_writer::{CommitSummaryBatchRequest, SemanticBatchRepoContext};
use super::super::workplane::{
    ClaimedSummaryMailboxBatch, SEMANTIC_SUMMARY_MAILBOX_BATCH_SIZE,
    repo_identity_from_runtime_metadata,
};
use super::helpers::{
    dedupe_inputs_by_artefact_id, load_current_semantic_inputs, payload_artefact_ids_from_value,
    select_current_semantic_input_scope,
};

pub(crate) struct PreparedSummaryMailboxBatch {
    pub commit: CommitSummaryBatchRequest,
    pub expanded_count: usize,
    pub attempts: u32,
}

pub(crate) async fn prepare_summary_mailbox_batch<F>(
    batch: &ClaimedSummaryMailboxBatch,
    mut on_item_prepared: F,
) -> Result<PreparedSummaryMailboxBatch>
where
    F: FnMut(&str, &BTreeSet<String>),
{
    let repo = repo_identity_from_runtime_metadata(&batch.repo_root, &batch.repo_id);
    let cfg = DevqlConfig::from_roots(batch.config_root.clone(), batch.repo_root.clone(), repo)?;
    let backends = resolve_store_backend_config_for_repo(&batch.config_root)?;
    let relational =
        RelationalStorage::connect(&cfg, &backends.relational, "semantic summary batch").await?;
    let capability_host = build_capability_host(&batch.repo_root, cfg.repo.clone())?;
    let config =
        resolve_semantic_clones_config(&capability_host.config_view(SEMANTIC_CLONES_CAPABILITY_ID));
    let summary_provider = resolve_summary_provider(
        &config,
        &capability_host.inference_for_capability(SEMANTIC_CLONES_CAPABILITY_ID),
        SummaryProviderMode::ConfiguredStrict,
    )?;
    let summary_embeddings_enabled =
        embedding_slot_for_representation(&config, EmbeddingRepresentationKind::Summary).is_some();

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
    let current_by_artefact = current_inputs
        .iter()
        .cloned()
        .map(|input| (input.artefact_id.clone(), input))
        .collect::<HashMap<_, _>>();

    let mut expanded_inputs = Vec::new();
    let mut artefact_session_ids = HashMap::<String, BTreeSet<String>>::new();
    let mut replacement_backfill_item = None;
    let mut current_by_artefact = current_by_artefact;
    let mut repo_wide_artefact_ids = None;
    for item in &batch.items {
        match item.item_kind {
            SemanticMailboxItemKind::Artefact => {
                if let Some(artefact_id) = item.artefact_id.as_ref()
                    && let Some(input) = current_by_artefact.get(artefact_id)
                {
                    expanded_inputs.push(input.clone());
                    if let Some(init_session_id) = item.init_session_id.as_ref() {
                        artefact_session_ids
                            .entry(input.artefact_id.clone())
                            .or_default()
                            .insert(init_session_id.clone());
                    }
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
                                    load_current_summary_backfill_artefact_ids(
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
                            .take(SEMANTIC_SUMMARY_MAILBOX_BATCH_SIZE)
                            .cloned()
                            .collect::<Vec<_>>();
                        if artefact_ids.len() > SEMANTIC_SUMMARY_MAILBOX_BATCH_SIZE {
                            let remaining_ids = artefact_ids
                                .iter()
                                .skip(SEMANTIC_SUMMARY_MAILBOX_BATCH_SIZE)
                                .cloned()
                                .collect::<Vec<_>>();
                            replacement_backfill_item =
                                Some(SemanticSummaryMailboxItemInsert::new(
                                    item.init_session_id.clone(),
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
                if selected.len() > SEMANTIC_SUMMARY_MAILBOX_BATCH_SIZE {
                    let remaining_ids = selected
                        .split_off(SEMANTIC_SUMMARY_MAILBOX_BATCH_SIZE)
                        .into_iter()
                        .map(|input| input.artefact_id)
                        .collect::<Vec<_>>();
                    replacement_backfill_item = Some(SemanticSummaryMailboxItemInsert::new(
                        item.init_session_id.clone(),
                        SemanticMailboxItemKind::RepoBackfill,
                        None,
                        Some(serde_json::to_value(remaining_ids)?),
                        item.dedupe_key.clone(),
                    ));
                }
                if let Some(init_session_id) = item.init_session_id.as_ref() {
                    for input in &selected {
                        artefact_session_ids
                            .entry(input.artefact_id.clone())
                            .or_default()
                            .insert(init_session_id.clone());
                    }
                }
                expanded_inputs.extend(selected);
            }
        }
    }
    dedupe_inputs_by_artefact_id(&mut expanded_inputs);

    let mut semantic_statements = Vec::new();
    let mut remote_semantic_statements = Vec::new();
    let shared_dialect = relational.dialect_for_role(RelationalStorageRole::SharedRelational);
    let shared_writes_remote = relational.has_remote_shared_relational_authority();
    let push_shared_statement =
        |local: &mut Vec<String>, remote: &mut Vec<String>, statement: String| {
            if shared_writes_remote {
                remote.push(statement);
            } else {
                local.push(statement);
            }
        };
    let mut embedding_follow_ups = Vec::new();
    for input in &expanded_inputs {
        let persist_summaries = summary_provider.provider.persists_summaries_for(input);
        let next_input_hash =
            build_semantic_feature_input_hash(input, summary_provider.provider.as_ref());
        let state = parse_semantic_index_state_rows(
            &relational
                .query_rows_for_role(
                    RelationalStorageRole::SharedRelational,
                    &build_semantic_get_index_state_sql(&input.artefact_id),
                )
                .await?,
        );
        if !semantic_features_require_reindex(
            &state,
            &next_input_hash,
            summary_provider.provider.requires_model_output(),
            persist_summaries,
        ) {
            if !shared_writes_remote {
                semantic_statements.push(
                    build_repair_current_semantic_projection_from_historical_sql(
                        &input.repo_id,
                        std::slice::from_ref(&input.artefact_id),
                        relational.dialect(),
                    ),
                );
            }
            if !persist_summaries {
                semantic_statements.push(build_delete_current_symbol_semantics_for_artefact_sql(
                    &input.repo_id,
                    &input.artefact_id,
                ));
            }
            if summary_embeddings_enabled && persist_summaries {
                embedding_follow_ups.push(summary_embedding_follow_up_for(input));
            }
            continue;
        }

        let input_for_rows = input.clone();
        let provider = Arc::clone(&summary_provider.provider);
        let rows = tokio::task::spawn_blocking(move || {
            crate::capability_packs::semantic_clones::features::build_semantic_feature_rows(
                &input_for_rows,
                provider.as_ref(),
            )
        })
        .await
        .context("building semantic summary rows on blocking worker")?;
        ensure_required_llm_summary_output(&rows, summary_provider.provider.as_ref())?;
        if persist_summaries {
            push_shared_statement(
                &mut semantic_statements,
                &mut remote_semantic_statements,
                build_semantic_persist_rows_sql(&rows, shared_dialect)?,
            );
            semantic_statements.push(build_conditional_current_semantic_persist_rows_sql(
                &rows,
                input,
                relational.dialect(),
            )?);
            if summary_embeddings_enabled {
                embedding_follow_ups.push(summary_embedding_follow_up_for(input));
            }
        } else {
            push_shared_statement(
                &mut semantic_statements,
                &mut remote_semantic_statements,
                build_symbol_feature_persist_rows_sql(&rows, shared_dialect)?,
            );
            semantic_statements.push(build_conditional_current_symbol_feature_persist_rows_sql(
                &rows,
                input,
                relational.dialect(),
            )?);
            semantic_statements.push(build_delete_current_symbol_semantics_for_artefact_sql(
                &input.repo_id,
                &input.artefact_id,
            ));
        }
        if let Some(init_session_ids) = artefact_session_ids.get(&input.artefact_id) {
            on_item_prepared(&input.artefact_id, init_session_ids);
        }
    }

    Ok(PreparedSummaryMailboxBatch {
        commit: CommitSummaryBatchRequest {
            repo: SemanticBatchRepoContext {
                repo_id: batch.repo_id.clone(),
                repo_root: batch.repo_root.clone(),
                config_root: batch.config_root.clone(),
            },
            lease_token: batch.lease_token.clone(),
            semantic_statements,
            remote_semantic_statements,
            embedding_follow_ups,
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
    })
}

fn summary_embedding_follow_up_for(
    input: &SemanticFeatureInput,
) -> SemanticEmbeddingMailboxItemInsert {
    SemanticEmbeddingMailboxItemInsert::new(
        None,
        EmbeddingRepresentationKind::Summary.to_string(),
        SemanticMailboxItemKind::Artefact,
        Some(input.artefact_id.clone()),
        None,
        Some(format!(
            "{}:{}",
            SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX, input.artefact_id
        )),
    )
}

fn explicit_artefact_ids_from_batch(
    items: &[crate::host::runtime_store::SemanticSummaryMailboxItemRecord],
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

async fn load_current_summary_backfill_artefact_ids(
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
