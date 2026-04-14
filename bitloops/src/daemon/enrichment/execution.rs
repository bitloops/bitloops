use anyhow::Result;
use serde_json::json;
use std::collections::BTreeMap;

use crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID;
use crate::capability_packs::semantic_clones::ingesters::{
    EmbeddingRefreshMode, SemanticFeaturesRefreshPayload, SemanticFeaturesRefreshScope,
    SemanticSummaryRefreshMode, SymbolEmbeddingsRefreshPayload, SymbolEmbeddingsRefreshScope,
};
use crate::capability_packs::semantic_clones::runtime_config::{
    embeddings_enabled, resolve_semantic_clones_config,
};
use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::capability_packs::semantic_clones::workplane::{
    load_effective_mailbox_intent_for_repo, payload_artefact_id, payload_is_repo_backfill,
    payload_representation_kind,
};
use crate::capability_packs::semantic_clones::{
    SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
    SEMANTIC_CLONES_SEMANTIC_FEATURES_REFRESH_INGESTER_ID,
    SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID, clear_repo_active_embedding_setup,
    clear_repo_symbol_embedding_rows, load_semantic_feature_inputs_for_artefacts,
    load_semantic_feature_inputs_for_current_repo,
    load_semantic_feature_inputs_for_historical_repo,
};
use crate::config::resolve_store_backend_config_for_repo;
use crate::host::devql::{
    DevqlConfig, RelationalStorage, build_capability_host, resolve_repo_identity,
};
use crate::host::runtime_store::WorkplaneJobRecord;

use super::workplane::fallback_repo_identity;
use super::{EnrichmentJobTarget, FollowUpJob, JobExecutionOutcome};

#[cfg(test)]
use anyhow::Context;

#[cfg(test)]
use crate::capability_packs::semantic_clones::features as semantic_features;

#[cfg(test)]
use super::{EnrichmentJob, EnrichmentJobKind};

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
            let payload = SemanticFeaturesRefreshPayload {
                scope: SemanticFeaturesRefreshScope::Historical,
                path: None,
                content_id: None,
                inputs,
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
                Ok(_) => {
                    let mut outcome = JobExecutionOutcome::ok();
                    if mailbox_intent.summary_embeddings_active {
                        if payload_is_repo_backfill(&job.payload) {
                            outcome.follow_ups.push(symbol_embeddings_repo_backfill_follow_up(
                                job,
                                crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary,
                            ));
                        } else if let Some(artefact_id) = payload_artefact_id(&job.payload) {
                            outcome.follow_ups.push(symbol_embeddings_follow_up_from_artefact_ids(
                                job,
                                &[artefact_id],
                                crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary,
                            ));
                        }
                    }
                    outcome
                }
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
            let inputs = match load_workplane_job_inputs(&relational, job).await {
                Ok(inputs) => inputs,
                Err(err) => return JobExecutionOutcome::failed(err),
            };
            if inputs.is_empty() {
                return JobExecutionOutcome::ok();
            }
            let payload = SymbolEmbeddingsRefreshPayload {
                scope: SymbolEmbeddingsRefreshScope::Historical,
                path: None,
                content_id: None,
                inputs,
                expected_input_hashes: BTreeMap::new(),
                representation_kind,
                mode: EmbeddingRefreshMode::ConfiguredStrict,
                manage_active_state: false,
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
                    let mut outcome = JobExecutionOutcome::ok();
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
        target: EnrichmentJobTarget::new(job.config_root.clone(), job.repo_root.clone()),
        artefact_ids: artefact_ids.to_vec(),
        input_hashes: input_hashes.clone(),
        representation_kind,
    }
}

#[cfg(test)]
fn clone_edges_rebuild_follow_up(job: &EnrichmentJob) -> FollowUpJob {
    FollowUpJob::CloneEdgesRebuild {
        target: EnrichmentJobTarget::new(job.config_root.clone(), job.repo_root.clone()),
    }
}

fn clone_edges_rebuild_follow_up_from_workplane(job: &WorkplaneJobRecord) -> FollowUpJob {
    FollowUpJob::CloneEdgesRebuild {
        target: EnrichmentJobTarget::new(job.config_root.clone(), job.repo_root.clone()),
    }
}

fn symbol_embeddings_follow_up_from_artefact_ids(
    job: &WorkplaneJobRecord,
    artefact_ids: &[String],
    representation_kind: crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind,
) -> FollowUpJob {
    FollowUpJob::SymbolEmbeddings {
        target: EnrichmentJobTarget::new(job.config_root.clone(), job.repo_root.clone()),
        artefact_ids: artefact_ids.to_vec(),
        input_hashes: BTreeMap::new(),
        representation_kind,
    }
}

fn symbol_embeddings_repo_backfill_follow_up(
    job: &WorkplaneJobRecord,
    representation_kind: crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind,
) -> FollowUpJob {
    FollowUpJob::SymbolEmbeddings {
        target: EnrichmentJobTarget::new(job.config_root.clone(), job.repo_root.clone()),
        artefact_ids: Vec::new(),
        input_hashes: BTreeMap::new(),
        representation_kind,
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

async fn load_repo_backfill_inputs(
    relational: &RelationalStorage,
    job: &WorkplaneJobRecord,
) -> Result<Vec<crate::capability_packs::semantic_clones::features::SemanticFeatureInput>> {
    let mut inputs =
        load_semantic_feature_inputs_for_current_repo(relational, &job.repo_root, &job.repo_id)
            .await?;
    inputs.extend(
        load_semantic_feature_inputs_for_historical_repo(relational, &job.repo_root, &job.repo_id)
            .await?,
    );
    inputs.sort_by(|left, right| left.artefact_id.cmp(&right.artefact_id));
    inputs.dedup_by(|left, right| left.artefact_id == right.artefact_id);
    Ok(inputs)
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
