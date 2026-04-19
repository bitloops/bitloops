use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID;
use crate::capability_packs::semantic_clones::embeddings::{
    ActiveEmbeddingRepresentationState, build_symbol_embedding_input_hash,
    build_symbol_embedding_inputs, build_symbol_embedding_row, resolve_embedding_setup,
    symbol_embeddings_require_reindex,
};
use crate::capability_packs::semantic_clones::runtime_config::{
    EmbeddingProviderMode, resolve_embedding_provider, resolve_semantic_clones_config,
};
use crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX;
use crate::capability_packs::semantic_clones::{
    build_active_embedding_setup_persist_sql, build_current_symbol_embedding_persist_sql,
    build_embedding_setup_persist_sql, build_sqlite_symbol_embedding_persist_sql,
    determine_repo_embedding_sync_action, load_current_semantic_summary_map,
    load_symbol_embedding_index_state,
};
use crate::config::resolve_store_backend_config_for_repo;
use crate::host::devql::{
    DevqlConfig, RelationalStorage, build_capability_host, resolve_repo_identity,
};
use crate::host::runtime_store::{
    CapabilityWorkplaneJobInsert, SemanticEmbeddingMailboxItemInsert, SemanticMailboxItemKind,
};

use super::super::semantic_writer::{CommitEmbeddingBatchRequest, SemanticBatchRepoContext};
use super::super::workplane::{
    ClaimedEmbeddingMailboxBatch, SEMANTIC_EMBEDDING_MAILBOX_BATCH_SIZE, fallback_repo_identity,
};
use super::helpers::{
    dedupe_inputs_by_artefact_id, load_current_semantic_inputs, payload_artefact_ids_from_value,
    select_current_semantic_input_scope,
};

pub(crate) struct PreparedEmbeddingMailboxBatch {
    pub commit: CommitEmbeddingBatchRequest,
    pub expanded_count: usize,
    pub attempts: u32,
}

pub(crate) async fn prepare_embedding_mailbox_batch(
    batch: &ClaimedEmbeddingMailboxBatch,
) -> Result<PreparedEmbeddingMailboxBatch> {
    let repo = resolve_repo_identity(&batch.repo_root)
        .unwrap_or_else(|_| fallback_repo_identity(&batch.repo_root, &batch.repo_id));
    let cfg = DevqlConfig::from_roots(batch.config_root.clone(), batch.repo_root.clone(), repo)?;
    let backends = resolve_store_backend_config_for_repo(&batch.config_root)?;
    let relational =
        RelationalStorage::connect(&cfg, &backends.relational, "semantic embedding batch").await?;
    let capability_host = build_capability_host(&batch.repo_root, cfg.repo.clone())?;
    let config =
        resolve_semantic_clones_config(&capability_host.config_view(SEMANTIC_CLONES_CAPABILITY_ID));
    let selection = resolve_embedding_provider(
        &config,
        &capability_host.inference_for_capability(SEMANTIC_CLONES_CAPABILITY_ID),
        batch.representation_kind,
        EmbeddingProviderMode::ConfiguredStrict,
    )?;
    let Some(provider) = selection.provider else {
        anyhow::bail!(
            "embedding provider is unavailable for representation `{}`",
            batch.representation_kind
        );
    };
    let setup = resolve_embedding_setup(provider.as_ref())?;

    let current_input_selection = select_current_semantic_input_scope(&batch.items);
    let current_inputs = load_current_semantic_inputs(
        &relational,
        &batch.repo_root,
        &batch.repo_id,
        current_input_selection.requested_artefact_ids(),
    )
    .await?;
    let current_by_artefact = current_inputs
        .iter()
        .cloned()
        .map(|input| (input.artefact_id.clone(), input))
        .collect::<HashMap<_, _>>();

    let mut expanded_inputs = Vec::new();
    let mut replacement_backfill_item = None;
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
                    .map(payload_artefact_ids_from_value)
                    .unwrap_or_default();
                let mut selected = if requested_ids.is_empty() {
                    current_inputs.clone()
                } else {
                    requested_ids
                        .iter()
                        .filter_map(|artefact_id| current_by_artefact.get(artefact_id).cloned())
                        .collect::<Vec<_>>()
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

    let mut embedding_statements = Vec::new();
    let mut upserted_any = false;
    if !embedding_inputs.is_empty() {
        embedding_statements.push(build_embedding_setup_persist_sql(&setup));
    }
    for embedding_input in embedding_inputs {
        let state = load_symbol_embedding_index_state(
            &relational,
            &embedding_input.artefact_id,
            embedding_input.representation_kind,
            &setup.setup_fingerprint,
        )
        .await?;
        let next_input_hash =
            build_symbol_embedding_input_hash(&embedding_input, provider.as_ref());
        if !symbol_embeddings_require_reindex(&state, &next_input_hash) {
            continue;
        }
        let embedding_input_for_row = embedding_input.clone();
        let provider_for_row = Arc::clone(&provider);
        let row = tokio::task::spawn_blocking(move || {
            build_symbol_embedding_row(&embedding_input_for_row, provider_for_row.as_ref())
        })
        .await
        .context("building embedding row on blocking worker")??;
        embedding_statements.push(build_sqlite_symbol_embedding_persist_sql(&row)?);
        if let Some(current_input) = current_by_artefact.get(&row.artefact_id) {
            embedding_statements.push(build_current_symbol_embedding_persist_sql(
                current_input,
                &current_input.path,
                &current_input.blob_sha,
                &row,
            )?);
        }
        upserted_any = true;
    }

    let mut setup_statements = Vec::new();
    if upserted_any {
        let sync_action = determine_repo_embedding_sync_action(
            &relational,
            &batch.repo_id,
            batch.representation_kind,
            &setup,
        )
        .await?;
        let _ = sync_action;
        setup_statements.push(build_active_embedding_setup_persist_sql(
            &batch.repo_id,
            &ActiveEmbeddingRepresentationState::new(batch.representation_kind, setup.clone()),
        ));
    }

    let clone_rebuild_signal = if upserted_any {
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
    })
}
