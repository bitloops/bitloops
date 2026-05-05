use super::*;
use crate::capability_packs::architecture_graph::roles::{
    default_queue_store, enqueue_adjudication_jobs_for_delta,
};

pub struct ArchitectureGraphCurrentStateConsumer;

impl CurrentStateConsumer for ArchitectureGraphCurrentStateConsumer {
    fn capability_id(&self) -> &str {
        ARCHITECTURE_GRAPH_CAPABILITY_ID
    }

    fn consumer_id(&self) -> &str {
        ARCHITECTURE_GRAPH_CONSUMER_ID
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
                .context("loading current files for architecture graph")?;
            let artefacts = context
                .relational
                .load_current_canonical_artefacts(&request.repo_id)
                .context("loading current artefacts for architecture graph")?;
            let dependency_edges = context
                .relational
                .load_current_canonical_edges(&request.repo_id)
                .context("loading current dependency edges for architecture graph")?;

            let mut warnings = Vec::new();
            let mut builder = GraphBuilder::new(
                &request.repo_id,
                request.to_generation_seq_inclusive,
                request
                    .run_id
                    .as_deref()
                    .unwrap_or(ARCHITECTURE_GRAPH_CONSUMER_ID),
            );
            builder.seed_repo_structure();
            builder.add_code_nodes(&artefacts);
            builder.add_dependency_edges(&dependency_edges);
            builder.add_entry_points_and_flows(
                context,
                &request.repo_root,
                &files,
                &artefacts,
                &dependency_edges,
            );
            builder.add_components_for_containers(&artefacts);
            add_test_harness_facts(context, &mut builder, &mut warnings).await;
            builder.add_change_unit(request);
            let mut synthesised_nodes = 0usize;
            let mut synthesised_edges = 0usize;
            match add_agent_synthesised_facts(context, request, &mut builder).await {
                Ok(Some(counts)) => {
                    synthesised_nodes = counts.0;
                    synthesised_edges = counts.1;
                }
                Ok(None) => {}
                Err(err) => warnings.push(format!(
                    "Architecture fact synthesis output rejected: {err:#}"
                )),
            }

            let facts = builder.finish();
            let metrics = json!({
                "nodes": facts.nodes.len(),
                "edges": facts.edges.len(),
                "synthesised_nodes": synthesised_nodes,
                "synthesised_edges": synthesised_edges,
                "files": files.len(),
                "artefacts": artefacts.len(),
                "dependency_edges": dependency_edges.len(),
                "reconcile_mode": format!("{:?}", request.reconcile_mode),
            });
            replace_computed_graph(
                &context.storage,
                &request.repo_id,
                facts,
                request.to_generation_seq_inclusive,
                &warnings,
                metrics.clone(),
            )
            .await?;

            let adjudication_metrics = enqueue_adjudication_jobs_for_delta(
                &request.repo_id,
                request.to_generation_seq_inclusive,
                &request.artefact_upserts,
                context.workplane.as_ref(),
                default_queue_store().as_ref(),
            )?;
            let mut metrics_with_adjudication = metrics;
            if let Some(metrics_obj) = metrics_with_adjudication.as_object_mut() {
                metrics_obj.insert(
                    "role_adjudication_selected".to_string(),
                    serde_json::Value::from(adjudication_metrics.selected as u64),
                );
                metrics_obj.insert(
                    "role_adjudication_enqueued".to_string(),
                    serde_json::Value::from(adjudication_metrics.enqueued as u64),
                );
                metrics_obj.insert(
                    "role_adjudication_deduped".to_string(),
                    serde_json::Value::from(adjudication_metrics.deduped as u64),
                );
            }

            Ok(CurrentStateConsumerResult {
                applied_to_generation_seq: request.to_generation_seq_inclusive,
                warnings,
                metrics: Some(metrics_with_adjudication),
            })
        })
    }
}
