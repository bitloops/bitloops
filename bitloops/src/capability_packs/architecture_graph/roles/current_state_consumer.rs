use anyhow::Context;
use serde_json::json;

use crate::capability_packs::architecture_graph::roles::{
    RoleAdjudicationEnqueueMetrics, default_queue_store, enqueue_adjudication_requests,
};
use crate::capability_packs::architecture_graph::types::{
    ARCHITECTURE_GRAPH_CAPABILITY_ID, ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID,
};
use crate::host::capability_host::{
    CurrentStateConsumer, CurrentStateConsumerContext, CurrentStateConsumerFuture,
    CurrentStateConsumerRequest, CurrentStateConsumerResult,
};

use super::classifier::{
    ArchitectureRoleClassificationInput, classify_architecture_roles_for_current_state,
    role_classification_scope_from_request,
};
use super::fact_extraction::RelationalArchitectureRoleCurrentStateSource;

pub struct ArchitectureGraphRoleCurrentStateConsumer;

impl CurrentStateConsumer for ArchitectureGraphRoleCurrentStateConsumer {
    fn capability_id(&self) -> &str {
        ARCHITECTURE_GRAPH_CAPABILITY_ID
    }

    fn consumer_id(&self) -> &str {
        ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID
    }

    fn reconcile<'a>(
        &'a self,
        request: &'a CurrentStateConsumerRequest,
        context: &'a CurrentStateConsumerContext,
    ) -> CurrentStateConsumerFuture<'a> {
        Box::pin(async move {
            let files = context
                .relational
                .load_current_canonical_files(&request.repo_id)
                .context("loading current files for architecture role classification")?;
            let role_current_state = RelationalArchitectureRoleCurrentStateSource::new(
                &request.repo_id,
                context.relational.as_ref(),
            );
            let outcome = classify_architecture_roles_for_current_state(
                context.storage.as_ref(),
                &role_current_state,
                ArchitectureRoleClassificationInput {
                    repo_id: &request.repo_id,
                    generation_seq: request.to_generation_seq_inclusive,
                    scope: role_classification_scope_from_request(request),
                    files: &files,
                },
            )
            .await
            .context("classifying architecture roles for current state")?;

            let mut warnings = outcome.warnings;
            let adjudication_request_count = outcome.adjudication_requests.len();
            let role_metrics = serde_json::to_value(&outcome.metrics)
                .unwrap_or_else(|_| json!({ "serialization_error": true }));
            let mut role_adjudication_enqueue_failed = false;
            let adjudication_metrics = match enqueue_adjudication_requests(
                &outcome.adjudication_requests,
                context.workplane.as_ref(),
                default_queue_store().as_ref(),
            ) {
                Ok(metrics) => metrics,
                Err(err) => {
                    warnings.push(format!(
                        "Architecture role adjudication enqueue failed: {err:#}"
                    ));
                    role_adjudication_enqueue_failed = true;
                    RoleAdjudicationEnqueueMetrics {
                        selected: adjudication_request_count,
                        enqueued: 0,
                        deduped: 0,
                    }
                }
            };

            Ok(CurrentStateConsumerResult {
                applied_to_generation_seq: request.to_generation_seq_inclusive,
                warnings,
                metrics: Some(role_current_state_metrics(
                    role_metrics,
                    &adjudication_metrics,
                    role_adjudication_enqueue_failed,
                )),
            })
        })
    }
}

fn role_current_state_metrics(
    role_metrics: serde_json::Value,
    adjudication_metrics: &RoleAdjudicationEnqueueMetrics,
    role_adjudication_enqueue_failed: bool,
) -> serde_json::Value {
    let mut metrics = json!({
        "roles": role_metrics,
        "role_adjudication_selected": adjudication_metrics.selected,
        "role_adjudication_enqueued": adjudication_metrics.enqueued,
        "role_adjudication_deduped": adjudication_metrics.deduped,
    });
    if role_adjudication_enqueue_failed {
        metrics["role_adjudication_enqueue_failed"] = serde_json::Value::Bool(true);
    }
    metrics
}
