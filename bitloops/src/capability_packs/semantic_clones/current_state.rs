use crate::host::capability_host::{
    CapabilityConfigView, CurrentStateConsumer, CurrentStateConsumerContext,
    CurrentStateConsumerFuture, CurrentStateConsumerRequest, CurrentStateConsumerResult,
};

use super::runtime_config::resolve_semantic_clones_config;
use super::types::{SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_CURRENT_STATE_CONSUMER_ID};
use super::workplane::resolve_effective_mailbox_intent;
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

            Ok(CurrentStateConsumerResult::applied(
                request.to_generation_seq_inclusive,
            ))
        })
    }
}
