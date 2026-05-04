#![cfg(test)]

use std::collections::BTreeMap;

use serde_json::json;

use crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID;
use crate::capability_packs::semantic_clones::features as semantic_features;
use crate::capability_packs::semantic_clones::ingesters::{
    EmbeddingRefreshMode, SemanticFeaturesRefreshPayload, SemanticFeaturesRefreshScope,
    SemanticSummaryRefreshMode, SymbolEmbeddingsRefreshPayload, SymbolEmbeddingsRefreshScope,
};
use crate::capability_packs::semantic_clones::runtime_config::{
    embeddings_enabled, resolve_semantic_clones_config,
};
use crate::capability_packs::semantic_clones::{
    SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
    SEMANTIC_CLONES_SEMANTIC_FEATURES_REFRESH_INGESTER_ID,
    SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
};
use crate::config::resolve_store_backend_config_for_repo;
use crate::host::devql::{DevqlConfig, RelationalStorage, build_capability_host};

use super::super::workplane::repo_identity_from_runtime_metadata;
use super::super::{EnrichmentJob, EnrichmentJobKind, JobExecutionOutcome};
use super::follow_ups::{clone_edges_rebuild_follow_up, symbol_embeddings_follow_up};
use super::helpers::clear_embedding_outputs;
use super::loaders::load_enrichment_job_inputs;

pub(crate) async fn execute_job(job: &EnrichmentJob) -> JobExecutionOutcome {
    let repo = repo_identity_from_runtime_metadata(&job.repo_root, &job.repo_id);
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
