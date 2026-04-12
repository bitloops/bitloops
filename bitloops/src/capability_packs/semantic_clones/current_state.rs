use crate::host::capability_host::{
    CapabilityConfigView, CurrentStateConsumer, CurrentStateConsumerContext,
    CurrentStateConsumerFuture, CurrentStateConsumerRequest, CurrentStateConsumerResult,
    ReconcileMode,
};

use super::runtime_config::resolve_semantic_clones_config;
use super::types::{SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_CURRENT_STATE_CONSUMER_ID};
use super::workplane::{
    enqueue_embedding_jobs, enqueue_summary_refresh_jobs, resolve_effective_mailbox_intent,
};
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
            let intent = resolve_effective_mailbox_intent(context.workplane.as_ref(), &config)?;
            if !intent.has_any_pipeline_intent() {
                super::pipeline::delete_repo_current_symbol_clone_edges(
                    context.storage.as_ref(),
                    &request.repo_id,
                )
                .await?;
                return Ok(CurrentStateConsumerResult::applied(
                    request.to_generation_seq_inclusive,
                ));
            }
            if !intent.has_any_embedding_intent() {
                super::pipeline::delete_repo_current_symbol_clone_edges(
                    context.storage.as_ref(),
                    &request.repo_id,
                )
                .await?;
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

            enqueue_summary_refresh_jobs(context.workplane.as_ref(), &inputs, &intent)?;
            enqueue_embedding_jobs(context.workplane.as_ref(), &inputs, &intent)?;
            Ok(CurrentStateConsumerResult::applied(
                request.to_generation_seq_inclusive,
            ))
        })
    }
}
