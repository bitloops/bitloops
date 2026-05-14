use super::*;

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
            let (facts, warnings, metrics) = {
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

                let mut artefacts_by_path =
                    BTreeMap::<String, Vec<LanguageEntryPointArtefact>>::new();
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
                });
                (facts, warnings, metrics)
            };
            context.ensure_parent_process_alive("persisting architecture graph")?;
            replace_computed_graph(
                &context.storage,
                &request.repo_id,
                facts,
                request.to_generation_seq_inclusive,
                &warnings,
                metrics.clone(),
            )
            .await?;

            Ok(CurrentStateConsumerResult {
                applied_to_generation_seq: request.to_generation_seq_inclusive,
                warnings,
                metrics: Some(metrics),
            })
        })
    }
}
