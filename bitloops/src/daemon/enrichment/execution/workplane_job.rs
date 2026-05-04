use std::collections::BTreeMap;

use serde_json::json;

use crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID;
use crate::capability_packs::semantic_clones::ingesters::{
    EmbeddingRefreshMode, SemanticFeaturesRefreshPayload, SemanticFeaturesRefreshScope,
    SemanticSummaryRefreshMode, SymbolEmbeddingsRefreshPayload,
};
use crate::capability_packs::semantic_clones::runtime_config::{
    embeddings_enabled, resolve_selected_summary_slot, resolve_semantic_clones_config,
};
use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::capability_packs::semantic_clones::workplane::{
    load_effective_mailbox_intent_for_repo, payload_representation_kind,
};
use crate::capability_packs::semantic_clones::{
    SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
    SEMANTIC_CLONES_SEMANTIC_FEATURES_REFRESH_INGESTER_ID,
    SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
};
use crate::config::resolve_store_backend_config_for_repo;
use crate::host::devql::{DevqlConfig, RelationalStorage, build_capability_host};
use crate::host::capability_host::CapabilityMailboxHandler;
use crate::host::devql::{
    DevqlConfig, RelationalStorage, build_capability_host, resolve_repo_identity,
};
use crate::host::runtime_store::WorkplaneJobRecord;

use super::super::JobExecutionOutcome;
use super::super::workplane::repo_identity_from_runtime_metadata;
use super::follow_ups::clone_edges_rebuild_follow_up_from_workplane;
use super::helpers::clear_embedding_outputs;
use super::loaders::{load_workplane_embedding_refresh_inputs, load_workplane_job_inputs};
use super::workplane_plan::{
    build_embedding_refresh_workplane_plan, build_summary_refresh_workplane_plan,
};

fn summary_refresh_mode(
    semantic_clones: &crate::config::SemanticClonesConfig,
) -> SemanticSummaryRefreshMode {
    if resolve_selected_summary_slot(semantic_clones).is_some() {
        SemanticSummaryRefreshMode::ConfiguredStrict
    } else {
        SemanticSummaryRefreshMode::DeterministicOnly
    }
}

pub(crate) async fn execute_workplane_job(job: &WorkplaneJobRecord) -> JobExecutionOutcome {
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
                mode: summary_refresh_mode(&semantic_clones),
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
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
        | SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX
        | SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX => {
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
                    if matches!(
                        representation_kind,
                        crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code
                            | crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary
                    ) {
                        outcome
                            .follow_ups
                            .push(clone_edges_rebuild_follow_up_from_workplane(job));
                    }
                    outcome
                }
                Err(err) => JobExecutionOutcome::failed(err),
            }
        }
        SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX => {
            execute_clone_edges_rebuild_workplane_job(&capability_host, &relational, job).await
        }
        mailbox_name => {
            let Some(registration) =
                capability_host.mailbox_registration(&job.capability_id, mailbox_name)
            else {
                return JobExecutionOutcome::failed(anyhow::anyhow!(
                    "unsupported workplane mailbox `{mailbox_name}` for capability `{}`",
                    job.capability_id
                ));
            };
            let CapabilityMailboxHandler::Ingester(ingester_id) = registration.handler else {
                return JobExecutionOutcome::failed(anyhow::anyhow!(
                    "unsupported workplane mailbox `{mailbox_name}` for capability `{}`",
                    job.capability_id
                ));
            };
            match capability_host
                .invoke_ingester_with_relational(
                    job.capability_id.as_str(),
                    ingester_id,
                    job.payload.clone(),
                    Some(&relational),
                )
                .await
            {
                Ok(_) => JobExecutionOutcome::ok(),
                Err(err) => JobExecutionOutcome::failed(err),
            }
        }
    }
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
mod tests {
    use super::*;

    #[test]
    fn summary_refresh_mode_uses_deterministic_fallback_without_summary_slot() {
        assert_eq!(
            summary_refresh_mode(&crate::config::SemanticClonesConfig::default()),
            SemanticSummaryRefreshMode::DeterministicOnly
        );
    }

    #[test]
    fn summary_refresh_mode_is_strict_with_configured_summary_slot() {
        let config = crate::config::SemanticClonesConfig {
            inference: crate::config::SemanticClonesInferenceBindings {
                summary_generation: Some("summary_local".to_string()),
                ..crate::config::SemanticClonesInferenceBindings::default()
            },
            ..crate::config::SemanticClonesConfig::default()
        };

        assert_eq!(
            summary_refresh_mode(&config),
            SemanticSummaryRefreshMode::ConfiguredStrict
        );
    }
}
