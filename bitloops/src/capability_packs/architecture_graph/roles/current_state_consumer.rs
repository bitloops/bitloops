use std::path::Path;

use anyhow::Context;
use serde_json::json;

use crate::capability_packs::architecture_graph::roles::{
    RoleAdjudicationEnqueueMetrics, default_queue_store, enqueue_adjudication_requests,
};
use crate::capability_packs::architecture_graph::types::{
    ARCHITECTURE_GRAPH_CAPABILITY_ID, ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT,
    ARCHITECTURE_GRAPH_ROLE_CURRENT_STATE_CONSUMER_ID,
};
use crate::capability_packs::semantic_clones::{
    runtime_config::resolve_semantic_clones_config,
    types::SEMANTIC_CLONES_CAPABILITY_ID,
    workplane::{
        architecture_embedding_jobs_for_artefacts, architecture_embedding_path_cleanup_jobs,
        load_effective_mailbox_intent_for_repo,
    },
};
use crate::host::capability_host::{
    CapabilityConfigView, CurrentStateConsumer, CurrentStateConsumerContext,
    CurrentStateConsumerFuture, CurrentStateConsumerRequest, CurrentStateConsumerResult,
};
use crate::host::devql::{RelationalStorage, esc_pg, sql_string_list_pg};

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
            let mut architecture_embedding_enqueue_failed = false;
            let architecture_embedding_metrics = match enqueue_architecture_embedding_refresh(
                &request.repo_id,
                &request.repo_root,
                outcome.metrics.full_reconcile,
                &outcome.architecture_embedding_refresh_paths,
                &outcome.architecture_embedding_cleanup_paths,
                context,
            )
            .await
            {
                Ok(metrics) => metrics,
                Err(err) => {
                    warnings.push(format!(
                        "Architecture embedding refresh enqueue failed: {err:#}"
                    ));
                    architecture_embedding_enqueue_failed = true;
                    ArchitectureEmbeddingEnqueueMetrics::default()
                }
            };
            let role_adjudication_configured = context
                .inference
                .has_slot(ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT);
            let mut role_adjudication_enqueue_failed = false;
            let mut role_adjudication_skipped_unconfigured = 0usize;
            let adjudication_metrics = if role_adjudication_configured {
                match enqueue_adjudication_requests(
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
                }
            } else {
                role_adjudication_skipped_unconfigured = adjudication_request_count;
                if adjudication_request_count > 0 {
                    warnings.push(format!(
                        "Architecture role adjudication skipped because inference slot `{ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT}` is not configured"
                    ));
                }
                RoleAdjudicationEnqueueMetrics {
                    selected: adjudication_request_count,
                    enqueued: 0,
                    deduped: 0,
                }
            };

            Ok(CurrentStateConsumerResult {
                applied_to_generation_seq: request.to_generation_seq_inclusive,
                warnings,
                metrics: Some(role_current_state_metrics(
                    role_metrics,
                    &adjudication_metrics,
                    role_adjudication_enqueue_failed,
                    role_adjudication_skipped_unconfigured,
                    &architecture_embedding_metrics,
                    architecture_embedding_enqueue_failed,
                )),
            })
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ArchitectureEmbeddingEnqueueMetrics {
    selected: u64,
    enqueued: u64,
    deduped: u64,
}

async fn enqueue_architecture_embedding_refresh(
    repo_id: &str,
    repo_root: &Path,
    full_reconcile: bool,
    refresh_paths: &[String],
    cleanup_paths: &[String],
    context: &CurrentStateConsumerContext,
) -> anyhow::Result<ArchitectureEmbeddingEnqueueMetrics> {
    let semantic_clones_config = resolve_semantic_clones_config(&CapabilityConfigView::new(
        SEMANTIC_CLONES_CAPABILITY_ID,
        context.config_root.clone(),
    ));
    let intent = load_effective_mailbox_intent_for_repo(repo_root, &semantic_clones_config)
        .context("loading semantic-clones mailbox intent for architecture embedding refresh")?;
    if !intent.architecture_embeddings_active {
        return Ok(ArchitectureEmbeddingEnqueueMetrics::default());
    }

    let artefact_ids = if full_reconcile {
        load_current_embedding_artefact_ids(context.storage.as_ref(), repo_id, None).await?
    } else {
        load_current_embedding_artefact_ids(context.storage.as_ref(), repo_id, Some(refresh_paths))
            .await?
    };
    let mut jobs = architecture_embedding_jobs_for_artefacts(&artefact_ids)?;
    jobs.extend(architecture_embedding_path_cleanup_jobs(cleanup_paths)?);
    let selected = jobs.len() as u64;
    if jobs.is_empty() {
        return Ok(ArchitectureEmbeddingEnqueueMetrics::default());
    }
    let result = context.workplane.enqueue_jobs(jobs)?;
    Ok(ArchitectureEmbeddingEnqueueMetrics {
        selected,
        enqueued: result.inserted_jobs,
        deduped: result.updated_jobs,
    })
}

async fn load_current_embedding_artefact_ids(
    relational: &RelationalStorage,
    repo_id: &str,
    paths: Option<&[String]>,
) -> anyhow::Result<Vec<String>> {
    let path_filter = paths
        .filter(|paths| !paths.is_empty())
        .map(|paths| format!("AND current.path IN ({})", sql_string_list_pg(paths)))
        .unwrap_or_default();
    let rows = relational
        .query_rows(&format!(
            "SELECT current.artefact_id \
             FROM artefacts_current current \
             JOIN current_file_state state ON state.repo_id = current.repo_id AND state.path = current.path \
             WHERE current.repo_id = '{}' \
               {path_filter} \
               AND state.analysis_mode = 'code' \
               AND LOWER(COALESCE(current.canonical_kind, COALESCE(current.language_kind, 'symbol'))) <> 'import' \
             ORDER BY current.path, current.start_line, current.symbol_id, COALESCE(current.start_byte, 0), current.artefact_id",
            esc_pg(repo_id),
        ))
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            row.get("artefact_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .collect())
}

fn role_current_state_metrics(
    role_metrics: serde_json::Value,
    adjudication_metrics: &RoleAdjudicationEnqueueMetrics,
    role_adjudication_enqueue_failed: bool,
    role_adjudication_skipped_unconfigured: usize,
    architecture_embedding_metrics: &ArchitectureEmbeddingEnqueueMetrics,
    architecture_embedding_enqueue_failed: bool,
) -> serde_json::Value {
    let mut metrics = json!({
        "roles": role_metrics,
        "architecture_embedding_selected": architecture_embedding_metrics.selected,
        "architecture_embedding_enqueued": architecture_embedding_metrics.enqueued,
        "architecture_embedding_deduped": architecture_embedding_metrics.deduped,
        "role_adjudication_selected": adjudication_metrics.selected,
        "role_adjudication_enqueued": adjudication_metrics.enqueued,
        "role_adjudication_deduped": adjudication_metrics.deduped,
    });
    if role_adjudication_skipped_unconfigured > 0 {
        metrics["role_adjudication_skipped_unconfigured"] =
            serde_json::Value::Number(role_adjudication_skipped_unconfigured.into());
    }
    if architecture_embedding_enqueue_failed {
        metrics["architecture_embedding_enqueue_failed"] = serde_json::Value::Bool(true);
    }
    if role_adjudication_enqueue_failed {
        metrics["role_adjudication_enqueue_failed"] = serde_json::Value::Bool(true);
    }
    metrics
}
