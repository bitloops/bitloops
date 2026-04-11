use crate::host::capability_host::{
    CapabilityConfigView, CurrentStateConsumer, CurrentStateConsumerContext,
    CurrentStateConsumerFuture, CurrentStateConsumerRequest, CurrentStateConsumerResult,
    ReconcileMode,
};

use super::runtime_config::{embeddings_enabled, resolve_semantic_clones_config};
use super::types::{SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_CURRENT_STATE_CONSUMER_ID};
use crate::capability_packs::semantic_clones::features as semantic_features;
use crate::config::resolve_bound_daemon_config_root_for_repo;
use std::collections::BTreeMap;

pub struct SemanticClonesCurrentStateConsumer;

impl CurrentStateConsumer for SemanticClonesCurrentStateConsumer {
    fn capability_id(&self) -> &str {
        SEMANTIC_CLONES_CAPABILITY_ID
    }

    fn consumer_id(&self) -> &str {
        SEMANTIC_CLONES_CURRENT_STATE_CONSUMER_ID
    }

    fn reconcile<'a>(
        &'a self,
        request: &'a CurrentStateConsumerRequest,
        context: &'a CurrentStateConsumerContext,
    ) -> CurrentStateConsumerFuture<'a> {
        Box::pin(async move {
            let config = resolve_semantic_clones_config(&CapabilityConfigView::new(
                SEMANTIC_CLONES_CAPABILITY_ID,
                context.config_root.clone(),
            ));
            if !embeddings_enabled(&config) {
                super::pipeline::delete_repo_current_symbol_clone_edges(
                    context.storage.as_ref(),
                    &request.repo_id,
                )
                .await?;
                return Ok(CurrentStateConsumerResult::applied(
                    request.to_generation_seq_inclusive,
                ));
            }

            let inputs = match request.reconcile_mode {
                ReconcileMode::MergedDelta => {
                    let artefact_ids = request
                        .artefact_upserts
                        .iter()
                        .map(|artefact| artefact.artefact_id.clone())
                        .collect::<Vec<_>>();
                    crate::capability_packs::semantic_clones::load_semantic_feature_inputs_for_artefacts(
                        context.storage.as_ref(),
                        request.repo_root.as_path(),
                        &artefact_ids,
                    )
                    .await?
                }
                ReconcileMode::FullReconcile => {
                    crate::capability_packs::semantic_clones::load_semantic_feature_inputs_for_current_repo(
                        context.storage.as_ref(),
                        request.repo_root.as_path(),
                        &request.repo_id,
                    )
                    .await?
                }
            };

            if inputs.is_empty() {
                return Ok(CurrentStateConsumerResult::applied(
                    request.to_generation_seq_inclusive,
                ));
            }

            let input_hashes = build_input_hashes(&inputs);
            let config_root =
                resolve_bound_daemon_config_root_for_repo(request.repo_root.as_path())?;
            let target = crate::daemon::EnrichmentJobTarget::new(
                config_root,
                request.repo_root.clone(),
                request.repo_id.clone(),
                request
                    .active_branch
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
            );
            let enrichment = crate::daemon::shared_enrichment_coordinator();
            enrichment
                .enqueue_symbol_embeddings(
                    target.clone(),
                    inputs.clone(),
                    input_hashes.clone(),
                    crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
                )
                .await?;
            enrichment
                .enqueue_symbol_embeddings(
                    target,
                    inputs,
                    input_hashes,
                    crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary,
                )
                .await?;
            Ok(CurrentStateConsumerResult::applied(
                request.to_generation_seq_inclusive,
            ))
        })
    }
}

fn build_input_hashes(
    inputs: &[semantic_features::SemanticFeatureInput],
) -> BTreeMap<String, String> {
    inputs
        .iter()
        .map(|input| {
            (
                input.artefact_id.clone(),
                semantic_features::build_semantic_feature_input_hash(
                    input,
                    &semantic_features::NoopSemanticSummaryProvider,
                ),
            )
        })
        .collect()
}
