use anyhow::{Context, Result};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID;
use crate::capability_packs::semantic_clones::embeddings::{
    ActiveEmbeddingRepresentationState, build_symbol_embedding_input_hash,
    build_symbol_embedding_inputs, build_symbol_embedding_row, resolve_embedding_setup,
    symbol_embeddings_require_reindex,
};
use crate::capability_packs::semantic_clones::features::{
    build_semantic_feature_input_hash, semantic_features_require_reindex,
};
use crate::capability_packs::semantic_clones::ingesters::{
    EmbeddingRefreshMode, SemanticFeaturesRefreshPayload, SemanticFeaturesRefreshScope,
    SemanticSummaryRefreshMode, SymbolEmbeddingsRefreshPayload, SymbolEmbeddingsRefreshScope,
};
use crate::capability_packs::semantic_clones::runtime_config::{
    EmbeddingProviderMode, SummaryProviderMode, embeddings_enabled, resolve_embedding_provider,
    resolve_semantic_clones_config, resolve_summary_provider,
};
use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::capability_packs::semantic_clones::workplane::{
    load_effective_mailbox_intent_for_repo, payload_artefact_id, payload_is_repo_backfill,
    payload_repo_backfill_artefact_ids, payload_representation_kind,
};
use crate::capability_packs::semantic_clones::{
    SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
    SEMANTIC_CLONES_SEMANTIC_FEATURES_REFRESH_INGESTER_ID,
    SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
    build_active_embedding_setup_persist_sql, build_conditional_current_semantic_persist_rows_sql,
    build_current_symbol_embedding_persist_sql, build_embedding_setup_persist_sql,
    build_semantic_get_index_state_sql, build_semantic_persist_rows_sql,
    build_sqlite_symbol_embedding_persist_sql, clear_repo_active_embedding_setup,
    clear_repo_symbol_embedding_rows, determine_repo_embedding_sync_action,
    ensure_required_llm_summary_output, load_current_semantic_summary_map,
    load_semantic_feature_inputs_for_artefacts, load_semantic_feature_inputs_for_current_repo,
    load_symbol_embedding_index_state, parse_semantic_index_state_rows,
};
use crate::config::resolve_store_backend_config_for_repo;
use crate::host::devql::{
    DevqlConfig, RelationalStorage, build_capability_host, resolve_repo_identity,
};
use crate::host::runtime_store::{
    CapabilityWorkplaneJobInsert, SemanticEmbeddingMailboxItemInsert, SemanticMailboxItemKind,
    SemanticSummaryMailboxItemInsert, WorkplaneJobRecord,
};

use super::semantic_writer::{
    CommitEmbeddingBatchRequest, CommitSummaryBatchRequest, SemanticBatchRepoContext,
};
use super::workplane::{
    ClaimedEmbeddingMailboxBatch, ClaimedSummaryMailboxBatch, SEMANTIC_MAILBOX_BATCH_SIZE,
    fallback_repo_identity,
};
use super::{EnrichmentJobTarget, FollowUpJob, JobExecutionOutcome};

const WORKPLANE_SUMMARY_REPO_BACKFILL_BATCH_SIZE: usize = 16;
const WORKPLANE_EMBEDDING_REPO_BACKFILL_BATCH_SIZE: usize = 8;

#[cfg(test)]
use crate::capability_packs::semantic_clones::features as semantic_features;

#[cfg(test)]
use super::{EnrichmentJob, EnrichmentJobKind};

type SemanticFeatureInput =
    crate::capability_packs::semantic_clones::features::SemanticFeatureInput;

struct SummaryRefreshWorkplanePlan {
    inputs: Vec<SemanticFeatureInput>,
    follow_ups: Vec<FollowUpJob>,
}

struct EmbeddingRefreshWorkplanePlan {
    scope: SymbolEmbeddingsRefreshScope,
    path: Option<String>,
    content_id: Option<String>,
    inputs: Vec<SemanticFeatureInput>,
    manage_active_state: bool,
    follow_ups: Vec<FollowUpJob>,
}

pub(super) struct PreparedSummaryMailboxBatch {
    pub commit: CommitSummaryBatchRequest,
    pub expanded_count: usize,
    pub attempts: u32,
}

pub(super) struct PreparedEmbeddingMailboxBatch {
    pub commit: CommitEmbeddingBatchRequest,
    pub expanded_count: usize,
    pub attempts: u32,
}

#[cfg(test)]
pub(super) async fn execute_job(job: &EnrichmentJob) -> JobExecutionOutcome {
    let repo = resolve_repo_identity(&job.repo_root)
        .unwrap_or_else(|_| fallback_repo_identity(&job.repo_root, &job.repo_id));
    let cfg = match DevqlConfig::from_roots(job.config_root.clone(), job.repo_root.clone(), repo) {
        Ok(cfg) => cfg,
        Err(err) => return JobExecutionOutcome::failed(err),
    };
    let backends = match resolve_store_backend_config_for_repo(&job.config_root) {
        Ok(backends) => backends,
        Err(err) => return JobExecutionOutcome::failed(err),
    };
    let relational =
        match RelationalStorage::connect(&cfg, &backends.relational, "daemon enrichment worker")
            .await
        {
            Ok(relational) => relational,
            Err(err) => return JobExecutionOutcome::failed(err),
        };
    let capability_host = match build_capability_host(&job.repo_root, cfg.repo.clone()) {
        Ok(host) => host,
        Err(err) => return JobExecutionOutcome::failed(err),
    };

    match &job.job {
        EnrichmentJobKind::SemanticSummaries {
            artefact_ids,
            input_hashes,
            ..
        } => {
            let inputs = match load_enrichment_job_inputs(&relational, job, artefact_ids).await {
                Ok(inputs) => inputs,
                Err(err) => return JobExecutionOutcome::failed(err),
            };
            execute_semantic_job(&capability_host, &relational, job, &inputs, input_hashes).await
        }
        EnrichmentJobKind::SymbolEmbeddings {
            artefact_ids,
            input_hashes,
            representation_kind,
            ..
        } => {
            let inputs = match load_enrichment_job_inputs(&relational, job, artefact_ids).await {
                Ok(inputs) => inputs,
                Err(err) => return JobExecutionOutcome::failed(err),
            };
            execute_embedding_job(
                &capability_host,
                &relational,
                job,
                &inputs,
                input_hashes,
                *representation_kind,
            )
            .await
        }
        EnrichmentJobKind::CloneEdgesRebuild {} => {
            execute_clone_edges_rebuild_job(&capability_host, &relational, job).await
        }
    }
}

pub(super) async fn execute_workplane_job(job: &WorkplaneJobRecord) -> JobExecutionOutcome {
    let repo = resolve_repo_identity(&job.repo_root)
        .unwrap_or_else(|_| fallback_repo_identity(&job.repo_root, &job.repo_id));
    let cfg = match DevqlConfig::from_roots(job.config_root.clone(), job.repo_root.clone(), repo) {
        Ok(cfg) => cfg,
        Err(err) => return JobExecutionOutcome::failed(err),
    };
    let backends = match resolve_store_backend_config_for_repo(&job.config_root) {
        Ok(backends) => backends,
        Err(err) => return JobExecutionOutcome::failed(err),
    };
    let relational =
        match RelationalStorage::connect(&cfg, &backends.relational, "daemon enrichment worker")
            .await
        {
            Ok(relational) => relational,
            Err(err) => return JobExecutionOutcome::failed(err),
        };
    let capability_host = match build_capability_host(&job.repo_root, cfg.repo.clone()) {
        Ok(host) => host,
        Err(err) => return JobExecutionOutcome::failed(err),
    };
    let semantic_clones =
        resolve_semantic_clones_config(&capability_host.config_view(SEMANTIC_CLONES_CAPABILITY_ID));
    let mailbox_intent =
        match load_effective_mailbox_intent_for_repo(&job.repo_root, &semantic_clones) {
            Ok(intent) => intent,
            Err(err) => return JobExecutionOutcome::failed(err),
        };

    match job.mailbox_name.as_str() {
        SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX => {
            if !mailbox_intent.summary_refresh_active {
                return JobExecutionOutcome::ok();
            }
            let inputs = match load_workplane_job_inputs(&relational, job).await {
                Ok(inputs) => inputs,
                Err(err) => return JobExecutionOutcome::failed(err),
            };
            if inputs.is_empty() {
                return JobExecutionOutcome::ok();
            }
            let plan = build_summary_refresh_workplane_plan(
                job,
                inputs,
                mailbox_intent.summary_embeddings_active,
            );
            let payload = SemanticFeaturesRefreshPayload {
                scope: SemanticFeaturesRefreshScope::Historical,
                path: None,
                content_id: None,
                inputs: plan.inputs,
                mode: SemanticSummaryRefreshMode::ConfiguredStrict,
            };
            match capability_host
                .invoke_ingester_with_relational(
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    SEMANTIC_CLONES_SEMANTIC_FEATURES_REFRESH_INGESTER_ID,
                    match serde_json::to_value(&payload) {
                        Ok(value) => value,
                        Err(err) => return JobExecutionOutcome::failed(err.into()),
                    },
                    Some(&relational),
                )
                .await
            {
                Ok(_) => JobExecutionOutcome {
                    error: None,
                    follow_ups: plan.follow_ups,
                },
                Err(err) => JobExecutionOutcome::failed(err),
            }
        }
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX | SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX => {
            let Some(representation_kind) = payload_representation_kind(&job.mailbox_name) else {
                return JobExecutionOutcome::failed(anyhow::anyhow!(
                    "embedding mailbox `{}` missing representation mapping",
                    job.mailbox_name
                ));
            };
            let (scope, path, content_id, inputs) =
                match load_workplane_embedding_refresh_inputs(&relational, job).await {
                    Ok(inputs) => inputs,
                    Err(err) => return JobExecutionOutcome::failed(err),
                };
            if inputs.is_empty() {
                return JobExecutionOutcome::ok();
            }
            let plan = build_embedding_refresh_workplane_plan(
                job,
                scope,
                path,
                content_id,
                inputs,
                representation_kind,
            );
            let payload = SymbolEmbeddingsRefreshPayload {
                scope: plan.scope,
                path: plan.path,
                content_id: plan.content_id,
                inputs: plan.inputs,
                expected_input_hashes: BTreeMap::new(),
                representation_kind,
                mode: EmbeddingRefreshMode::ConfiguredStrict,
                manage_active_state: plan.manage_active_state,
                perform_clone_rebuild_inline: false,
            };
            match capability_host
                .invoke_ingester_with_relational(
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
                    match serde_json::to_value(&payload) {
                        Ok(value) => value,
                        Err(err) => return JobExecutionOutcome::failed(err.into()),
                    },
                    Some(&relational),
                )
                .await
            {
                Ok(_) => {
                    let mut outcome = JobExecutionOutcome {
                        error: None,
                        follow_ups: plan.follow_ups,
                    };
                    outcome
                        .follow_ups
                        .push(clone_edges_rebuild_follow_up_from_workplane(job));
                    outcome
                }
                Err(err) => JobExecutionOutcome::failed(err),
            }
        }
        SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX => {
            execute_clone_edges_rebuild_workplane_job(&capability_host, &relational, job).await
        }
        mailbox_name => JobExecutionOutcome::failed(anyhow::anyhow!(
            "unsupported workplane mailbox `{mailbox_name}` for capability `{}`",
            job.capability_id
        )),
    }
}

#[cfg(test)]
async fn execute_semantic_job(
    capability_host: &crate::host::capability_host::DevqlCapabilityHost,
    relational: &RelationalStorage,
    job: &EnrichmentJob,
    inputs: &[semantic_features::SemanticFeatureInput],
    input_hashes: &BTreeMap<String, String>,
) -> JobExecutionOutcome {
    let semantic_clones =
        resolve_semantic_clones_config(&capability_host.config_view(SEMANTIC_CLONES_CAPABILITY_ID));
    let artefact_ids = inputs
        .iter()
        .map(|input| input.artefact_id.clone())
        .collect::<Vec<_>>();
    let payload = SemanticFeaturesRefreshPayload {
        scope: SemanticFeaturesRefreshScope::Historical,
        path: None,
        content_id: None,
        inputs: inputs.to_vec(),
        mode: SemanticSummaryRefreshMode::ConfiguredStrict,
    };
    let result = match capability_host
        .invoke_ingester_with_relational(
            SEMANTIC_CLONES_CAPABILITY_ID,
            SEMANTIC_CLONES_SEMANTIC_FEATURES_REFRESH_INGESTER_ID,
            match serde_json::to_value(&payload) {
                Ok(value) => value,
                Err(err) => return JobExecutionOutcome::failed(err.into()),
            },
            Some(relational),
        )
        .await
    {
        Ok(result) => result,
        Err(err) => return JobExecutionOutcome::failed(err),
    };

    let produced_enriched_semantics = result.payload["produced_enriched_semantics"]
        .as_bool()
        .unwrap_or(false);

    let mut outcome = JobExecutionOutcome::ok();
    if produced_enriched_semantics && embeddings_enabled(&semantic_clones) {
        outcome.follow_ups.push(symbol_embeddings_follow_up(
            job,
            &artefact_ids,
            input_hashes,
            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
        ));
        outcome.follow_ups.push(symbol_embeddings_follow_up(
            job,
            &artefact_ids,
            input_hashes,
            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary,
        ));
    }
    outcome
}

#[cfg(test)]
async fn execute_embedding_job(
    capability_host: &crate::host::capability_host::DevqlCapabilityHost,
    relational: &RelationalStorage,
    job: &EnrichmentJob,
    inputs: &[semantic_features::SemanticFeatureInput],
    input_hashes: &BTreeMap<String, String>,
    representation_kind: crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind,
) -> JobExecutionOutcome {
    let payload = SymbolEmbeddingsRefreshPayload {
        scope: SymbolEmbeddingsRefreshScope::Historical,
        path: None,
        content_id: None,
        inputs: inputs.to_vec(),
        expected_input_hashes: input_hashes.clone(),
        representation_kind,
        mode: EmbeddingRefreshMode::ConfiguredStrict,
        manage_active_state: true,
        perform_clone_rebuild_inline: true,
    };
    let result = match capability_host
        .invoke_ingester_with_relational(
            SEMANTIC_CLONES_CAPABILITY_ID,
            SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
            match serde_json::to_value(&payload) {
                Ok(value) => value,
                Err(err) => return JobExecutionOutcome::failed(err.into()),
            },
            Some(relational),
        )
        .await
    {
        Ok(result) => result,
        Err(err) => return JobExecutionOutcome::failed(err),
    };

    let mut outcome = JobExecutionOutcome::ok();
    if result.payload["clone_rebuild_recommended"]
        .as_bool()
        .unwrap_or(false)
    {
        outcome.follow_ups.push(clone_edges_rebuild_follow_up(job));
    }
    outcome
}

#[cfg(test)]
async fn execute_clone_edges_rebuild_job(
    capability_host: &crate::host::capability_host::DevqlCapabilityHost,
    relational: &RelationalStorage,
    job: &EnrichmentJob,
) -> JobExecutionOutcome {
    let semantic_clones =
        resolve_semantic_clones_config(&capability_host.config_view(SEMANTIC_CLONES_CAPABILITY_ID));
    if !embeddings_enabled(&semantic_clones) {
        return match clear_embedding_outputs(relational, &job.repo_id).await {
            Ok(()) => JobExecutionOutcome::ok(),
            Err(err) => JobExecutionOutcome::failed(err),
        };
    }

    match capability_host
        .invoke_ingester_with_relational(
            SEMANTIC_CLONES_CAPABILITY_ID,
            SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
            json!({}),
            Some(relational),
        )
        .await
    {
        Ok(_) => JobExecutionOutcome::ok(),
        Err(err) => JobExecutionOutcome::failed(err),
    }
}

#[cfg(test)]
fn symbol_embeddings_follow_up(
    job: &EnrichmentJob,
    artefact_ids: &[String],
    input_hashes: &BTreeMap<String, String>,
    representation_kind: crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind,
) -> FollowUpJob {
    FollowUpJob::SymbolEmbeddings {
        target: EnrichmentJobTarget::new(job.config_root.clone(), job.repo_root.clone())
            .with_init_session_id(None),
        artefact_ids: artefact_ids.to_vec(),
        input_hashes: input_hashes.clone(),
        representation_kind,
    }
}

#[cfg(test)]
fn clone_edges_rebuild_follow_up(job: &EnrichmentJob) -> FollowUpJob {
    FollowUpJob::CloneEdgesRebuild {
        target: EnrichmentJobTarget::new(job.config_root.clone(), job.repo_root.clone())
            .with_init_session_id(None),
    }
}

fn clone_edges_rebuild_follow_up_from_workplane(job: &WorkplaneJobRecord) -> FollowUpJob {
    FollowUpJob::CloneEdgesRebuild {
        target: EnrichmentJobTarget::new(job.config_root.clone(), job.repo_root.clone())
            .with_init_session_id(job.init_session_id.clone()),
    }
}

fn symbol_embeddings_follow_up_from_artefact_ids(
    job: &WorkplaneJobRecord,
    artefact_ids: &[String],
    representation_kind: crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind,
) -> FollowUpJob {
    FollowUpJob::SymbolEmbeddings {
        target: EnrichmentJobTarget::new(job.config_root.clone(), job.repo_root.clone())
            .with_init_session_id(job.init_session_id.clone()),
        artefact_ids: artefact_ids.to_vec(),
        input_hashes: BTreeMap::new(),
        representation_kind,
    }
}

fn build_summary_refresh_workplane_plan(
    job: &WorkplaneJobRecord,
    inputs: Vec<SemanticFeatureInput>,
    summary_embeddings_active: bool,
) -> SummaryRefreshWorkplanePlan {
    if payload_is_repo_backfill(&job.payload) {
        return build_repo_backfill_summary_refresh_workplane_plan(
            job,
            inputs,
            summary_embeddings_active,
        );
    }

    let mut follow_ups = Vec::new();
    if summary_embeddings_active && let Some(artefact_id) = payload_artefact_id(&job.payload) {
        follow_ups.push(symbol_embeddings_follow_up_from_artefact_ids(
            job,
            &[artefact_id],
            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary,
        ));
    }
    SummaryRefreshWorkplanePlan { inputs, follow_ups }
}

fn build_repo_backfill_summary_refresh_workplane_plan(
    job: &WorkplaneJobRecord,
    inputs: Vec<SemanticFeatureInput>,
    summary_embeddings_active: bool,
) -> SummaryRefreshWorkplanePlan {
    let batch_size = inputs.len().min(WORKPLANE_SUMMARY_REPO_BACKFILL_BATCH_SIZE);
    let (batch_inputs, remaining_inputs): (Vec<_>, Vec<_>) = inputs
        .into_iter()
        .enumerate()
        .partition(|(index, _)| *index < batch_size);
    let inputs = batch_inputs
        .into_iter()
        .map(|(_, input)| input)
        .collect::<Vec<_>>();
    let remaining_artefact_ids = remaining_inputs
        .into_iter()
        .map(|(_, input)| input.artefact_id)
        .collect::<Vec<_>>();
    let mut follow_ups = Vec::new();
    if summary_embeddings_active {
        let processed_artefact_ids = inputs
            .iter()
            .map(|input| input.artefact_id.clone())
            .collect::<Vec<_>>();
        if !processed_artefact_ids.is_empty() {
            follow_ups.push(symbol_embeddings_follow_up_from_artefact_ids(
                job,
                &processed_artefact_ids,
                crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary,
            ));
        }
    }
    if !remaining_artefact_ids.is_empty() {
        follow_ups.push(FollowUpJob::SemanticSummaries {
            target: EnrichmentJobTarget::new(job.config_root.clone(), job.repo_root.clone())
                .with_init_session_id(job.init_session_id.clone()),
            artefact_ids: remaining_artefact_ids,
        });
    }
    SummaryRefreshWorkplanePlan { inputs, follow_ups }
}

fn build_embedding_refresh_workplane_plan(
    job: &WorkplaneJobRecord,
    scope: SymbolEmbeddingsRefreshScope,
    path: Option<String>,
    content_id: Option<String>,
    inputs: Vec<SemanticFeatureInput>,
    representation_kind: crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind,
) -> EmbeddingRefreshWorkplanePlan {
    if !payload_is_repo_backfill(&job.payload) {
        return EmbeddingRefreshWorkplanePlan {
            scope,
            path,
            content_id,
            inputs,
            manage_active_state: false,
            follow_ups: Vec::new(),
        };
    }

    let batch_size = inputs
        .len()
        .min(WORKPLANE_EMBEDDING_REPO_BACKFILL_BATCH_SIZE);
    let (batch_inputs, remaining_inputs): (Vec<_>, Vec<_>) = inputs
        .into_iter()
        .enumerate()
        .partition(|(index, _)| *index < batch_size);
    let inputs = batch_inputs
        .into_iter()
        .map(|(_, input)| input)
        .collect::<Vec<_>>();
    let remaining_artefact_ids = remaining_inputs
        .into_iter()
        .map(|(_, input)| input.artefact_id)
        .collect::<Vec<_>>();
    let mut follow_ups = Vec::new();
    if !remaining_artefact_ids.is_empty() {
        follow_ups.push(FollowUpJob::RepoBackfillEmbeddings {
            target: EnrichmentJobTarget::new(job.config_root.clone(), job.repo_root.clone())
                .with_init_session_id(job.init_session_id.clone()),
            artefact_ids: remaining_artefact_ids,
            representation_kind,
        });
    }

    EmbeddingRefreshWorkplanePlan {
        scope,
        path,
        content_id,
        inputs,
        manage_active_state: follow_ups.is_empty(),
        follow_ups,
    }
}

async fn load_workplane_job_inputs(
    relational: &RelationalStorage,
    job: &WorkplaneJobRecord,
) -> Result<Vec<crate::capability_packs::semantic_clones::features::SemanticFeatureInput>> {
    if payload_is_repo_backfill(&job.payload) {
        return load_repo_backfill_inputs(relational, job).await;
    }

    let Some(artefact_id) = payload_artefact_id(&job.payload) else {
        anyhow::bail!("workplane mailbox job missing artefact id");
    };
    load_semantic_feature_inputs_for_artefacts(
        relational,
        &job.repo_root,
        std::slice::from_ref(&artefact_id),
    )
    .await
}

async fn load_workplane_embedding_refresh_inputs(
    relational: &RelationalStorage,
    job: &WorkplaneJobRecord,
) -> Result<(
    SymbolEmbeddingsRefreshScope,
    Option<String>,
    Option<String>,
    Vec<crate::capability_packs::semantic_clones::features::SemanticFeatureInput>,
)> {
    if payload_is_repo_backfill(&job.payload) {
        return Ok((
            SymbolEmbeddingsRefreshScope::Historical,
            None,
            None,
            load_repo_backfill_inputs(relational, job).await?,
        ));
    }

    let Some(artefact_id) = payload_artefact_id(&job.payload) else {
        anyhow::bail!("workplane mailbox job missing artefact id");
    };
    let current_inputs =
        load_semantic_feature_inputs_for_current_repo(relational, &job.repo_root, &job.repo_id)
            .await?
            .into_iter()
            .filter(|input| input.artefact_id == artefact_id)
            .collect::<Vec<_>>();
    if let Some(first) = current_inputs.first() {
        let single_path = current_inputs
            .iter()
            .all(|input| input.path == first.path && input.blob_sha == first.blob_sha);
        if single_path {
            return Ok((
                SymbolEmbeddingsRefreshScope::CurrentPath,
                Some(first.path.clone()),
                Some(first.blob_sha.clone()),
                current_inputs,
            ));
        }
    }

    Ok((
        SymbolEmbeddingsRefreshScope::Historical,
        None,
        None,
        load_workplane_job_inputs(relational, job).await?,
    ))
}

async fn load_repo_backfill_inputs(
    relational: &RelationalStorage,
    job: &WorkplaneJobRecord,
) -> Result<Vec<crate::capability_packs::semantic_clones::features::SemanticFeatureInput>> {
    let current_inputs =
        load_semantic_feature_inputs_for_current_repo(relational, &job.repo_root, &job.repo_id)
            .await?;
    let Some(artefact_ids) = payload_repo_backfill_artefact_ids(&job.payload) else {
        return Ok(current_inputs);
    };
    let artefact_ids = artefact_ids
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    Ok(current_inputs
        .into_iter()
        .filter(|input| artefact_ids.contains(&input.artefact_id))
        .collect())
}

async fn clear_embedding_outputs(relational: &RelationalStorage, repo_id: &str) -> Result<()> {
    clear_repo_symbol_embedding_rows(relational, repo_id).await?;
    clear_repo_active_embedding_setup(relational, repo_id).await?;
    crate::capability_packs::semantic_clones::pipeline::delete_repo_symbol_clone_edges(
        relational, repo_id,
    )
    .await
}

async fn execute_clone_edges_rebuild_workplane_job(
    capability_host: &crate::host::capability_host::DevqlCapabilityHost,
    relational: &RelationalStorage,
    job: &WorkplaneJobRecord,
) -> JobExecutionOutcome {
    let semantic_clones =
        resolve_semantic_clones_config(&capability_host.config_view(SEMANTIC_CLONES_CAPABILITY_ID));
    if !embeddings_enabled(&semantic_clones) {
        return match clear_embedding_outputs(relational, &job.repo_id).await {
            Ok(()) => JobExecutionOutcome::ok(),
            Err(err) => JobExecutionOutcome::failed(err),
        };
    }

    match capability_host
        .invoke_ingester_with_relational(
            SEMANTIC_CLONES_CAPABILITY_ID,
            SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
            json!({}),
            Some(relational),
        )
        .await
    {
        Ok(_) => JobExecutionOutcome::ok(),
        Err(err) => JobExecutionOutcome::failed(err),
    }
}

pub(super) async fn prepare_summary_mailbox_batch<F>(
    batch: &ClaimedSummaryMailboxBatch,
    mut on_item_prepared: F,
) -> Result<PreparedSummaryMailboxBatch>
where
    F: FnMut(&str, &BTreeSet<String>),
{
    let repo = resolve_repo_identity(&batch.repo_root)
        .unwrap_or_else(|_| fallback_repo_identity(&batch.repo_root, &batch.repo_id));
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

    let current_inputs = load_semantic_feature_inputs_for_current_repo(
        &relational,
        &batch.repo_root,
        &batch.repo_id,
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
                    .map(payload_artefact_ids_from_value)
                    .unwrap_or_default();
                let mut selected = if requested_ids.is_empty() {
                    current_inputs.clone()
                } else {
                    let requested_ids = requested_ids.into_iter().collect::<BTreeSet<_>>();
                    current_inputs
                        .iter()
                        .filter(|input| requested_ids.contains(&input.artefact_id))
                        .cloned()
                        .collect::<Vec<_>>()
                };
                if selected.len() > SEMANTIC_MAILBOX_BATCH_SIZE {
                    let remaining_ids = selected
                        .split_off(SEMANTIC_MAILBOX_BATCH_SIZE)
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
    let mut embedding_follow_ups = Vec::new();
    for input in &expanded_inputs {
        let next_input_hash =
            build_semantic_feature_input_hash(input, summary_provider.provider.as_ref());
        let state = parse_semantic_index_state_rows(
            &relational
                .query_rows(&build_semantic_get_index_state_sql(&input.artefact_id))
                .await?,
        );
        if !semantic_features_require_reindex(&state, &next_input_hash) {
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
        semantic_statements.push(build_semantic_persist_rows_sql(
            &rows,
            relational.dialect(),
        )?);
        semantic_statements.push(build_conditional_current_semantic_persist_rows_sql(
            &rows,
            input,
            relational.dialect(),
        )?);
        if config.embedding_mode != crate::config::SemanticCloneEmbeddingMode::Off {
            embedding_follow_ups.push(SemanticEmbeddingMailboxItemInsert::new(
                None,
                crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary
                    .to_string(),
                SemanticMailboxItemKind::Artefact,
                Some(input.artefact_id.clone()),
                None,
                Some(format!(
                    "{}:{}",
                    SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX, input.artefact_id
                )),
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

pub(super) async fn prepare_embedding_mailbox_batch(
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

    let current_inputs = load_semantic_feature_inputs_for_current_repo(
        &relational,
        &batch.repo_root,
        &batch.repo_id,
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
                    let requested_ids = requested_ids.into_iter().collect::<BTreeSet<_>>();
                    current_inputs
                        .iter()
                        .filter(|input| requested_ids.contains(&input.artefact_id))
                        .cloned()
                        .collect::<Vec<_>>()
                };
                if selected.len() > SEMANTIC_MAILBOX_BATCH_SIZE {
                    let remaining_ids = selected
                        .split_off(SEMANTIC_MAILBOX_BATCH_SIZE)
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

fn dedupe_inputs_by_artefact_id(inputs: &mut Vec<SemanticFeatureInput>) {
    let mut seen = BTreeSet::new();
    inputs.retain(|input| seen.insert(input.artefact_id.clone()));
}

fn payload_artefact_ids_from_value(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::Array(values) => values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
async fn load_enrichment_job_inputs(
    relational: &RelationalStorage,
    job: &EnrichmentJob,
    artefact_ids: &[String],
) -> Result<Vec<semantic_features::SemanticFeatureInput>> {
    load_semantic_feature_inputs_for_artefacts(relational, &job.repo_root, artefact_ids)
        .await
        .with_context(|| {
            format!(
                "rehydrating enrichment inputs for job `{}` in repo `{}`",
                job.id, job.repo_id
            )
        })
}

#[cfg(test)]
mod execution_tests;
