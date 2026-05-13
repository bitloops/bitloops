use super::*;
use crate::capability_packs::architecture_graph::roles::{
    RoleAdjudicationEnqueueMetrics, default_queue_store, enqueue_adjudication_requests,
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

            let mut artefacts_by_path = BTreeMap::<String, Vec<LanguageEntryPointArtefact>>::new();
            let mut component_inputs = Vec::<ComponentArtefactInput>::new();
            let mut artefact_count = 0usize;
            context
                .relational
                .visit_current_canonical_artefacts(&request.repo_id, &mut |artefact| {
                    builder.add_code_node(&artefact);
                    artefacts_by_path
                        .entry(artefact.path.clone())
                        .or_default()
                        .push(entry_point_artefact_from_current(&artefact));
                    component_inputs.push(ComponentArtefactInput {
                        artefact_id: artefact.artefact_id.clone(),
                        path: artefact.path.clone(),
                    });
                    artefact_count += 1;
                    Ok(())
                })
                .context("visiting current artefacts for architecture graph")?;

            let mut adjacency = BTreeMap::<String, BTreeSet<String>>::new();
            let mut dependency_edge_count = 0usize;
            context
                .relational
                .visit_current_canonical_edges(&request.repo_id, &mut |dependency| {
                    builder.add_dependency_edge(&dependency);
                    insert_dependency_adjacency(&mut adjacency, &dependency);
                    dependency_edge_count += 1;
                    Ok(())
                })
                .context("visiting current dependency edges for architecture graph")?;

            builder.add_entry_points_and_flows(
                context,
                &request.repo_root,
                &files,
                &artefacts_by_path,
                &adjacency,
            );
            builder.add_components_for_containers(&component_inputs);
            add_test_harness_facts(context, &mut builder, &mut warnings).await;
            let change_metrics = builder.add_change_unit(request);
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
            let mut role_metrics = serde_json::Value::Null;
            let mut adjudication_requests = Vec::new();
            let role_current_state =
                crate::capability_packs::architecture_graph::roles::fact_extraction::RelationalArchitectureRoleCurrentStateSource::new(
                    &request.repo_id,
                    context.relational.as_ref(),
                );
            match crate::capability_packs::architecture_graph::roles::classifier::classify_architecture_roles_for_current_state(
                context.storage.as_ref(),
                &role_current_state,
                crate::capability_packs::architecture_graph::roles::classifier::ArchitectureRoleClassificationInput {
                    repo_id: &request.repo_id,
                    generation_seq: request.to_generation_seq_inclusive,
                    scope: crate::capability_packs::architecture_graph::roles::classifier::role_classification_scope_from_request(request),
                    files: &files,
                },
            )
            .await
            {
                Ok(outcome) => {
                    warnings.extend(outcome.warnings);
                    adjudication_requests = outcome.adjudication_requests;
                    role_metrics = serde_json::to_value(outcome.metrics)
                        .unwrap_or_else(|_| json!({ "serialization_error": true }));
                }
                Err(err) => {
                    warnings.push(format!("Architecture role classification failed: {err:#}"));
                }
            }
            let metrics = json!({
                "nodes": facts.nodes.len(),
                "edges": facts.edges.len(),
                "synthesised_nodes": synthesised_nodes,
                "synthesised_edges": synthesised_edges,
                "files": files.len(),
                "artefacts": artefact_count,
                "dependency_edges": dependency_edge_count,
                "affected_paths": change_metrics.affected_paths,
                "impacted_nodes": change_metrics.impacted_nodes,
                "reconcile_mode": format!("{:?}", request.reconcile_mode),
                "roles": role_metrics,
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

            let mut role_adjudication_enqueue_failed = false;
            let adjudication_metrics = match enqueue_adjudication_requests(
                &adjudication_requests,
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
                        selected: adjudication_requests.len(),
                        enqueued: 0,
                        deduped: 0,
                    }
                }
            };
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
                if role_adjudication_enqueue_failed {
                    metrics_obj.insert(
                        "role_adjudication_enqueue_failed".to_string(),
                        serde_json::Value::Bool(true),
                    );
                }
            }

            Ok(CurrentStateConsumerResult {
                applied_to_generation_seq: request.to_generation_seq_inclusive,
                warnings,
                metrics: Some(metrics_with_adjudication),
            })
        })
    }
}
