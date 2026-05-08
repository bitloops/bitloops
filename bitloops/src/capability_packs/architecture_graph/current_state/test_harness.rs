use super::*;

pub(super) async fn add_test_harness_facts(
    context: &CurrentStateConsumerContext,
    builder: &mut GraphBuilder,
    warnings: &mut Vec<String>,
) {
    let test_rows = match optional_query(
        context,
        &format!(
            "SELECT artefact_id, symbol_id, path, name, canonical_kind, language_kind, symbol_fqn, start_line, end_line \
             FROM test_artefacts_current WHERE repo_id = '{}' ORDER BY path, start_line, symbol_id",
            crate::host::devql::esc_pg(&builder.repo_id)
        ),
        warnings,
    )
    .await
    {
        Some(rows) => rows,
        None => return,
    };

    let system_id = builder.fallback_system_id();
    let mut test_artefact_nodes = BTreeMap::new();
    let mut test_symbol_nodes = BTreeMap::new();
    for row in test_rows {
        let Some(artefact_id) = string_field(&row, "artefact_id") else {
            continue;
        };
        let Some(symbol_id) = string_field(&row, "symbol_id") else {
            continue;
        };
        let path = string_field(&row, "path");
        let test_node_id = node_id(
            &builder.repo_id,
            ArchitectureGraphNodeKind::Test,
            &artefact_id,
        );
        test_artefact_nodes.insert(artefact_id.clone(), test_node_id.clone());
        test_symbol_nodes.insert(symbol_id.clone(), test_node_id.clone());
        builder.upsert_node(ArchitectureGraphNodeFact {
            repo_id: builder.repo_id.clone(),
            node_id: test_node_id.clone(),
            node_kind: ArchitectureGraphNodeKind::Test.as_str().to_string(),
            label: string_field(&row, "name").unwrap_or_else(|| artefact_id.clone()),
            artefact_id: Some(artefact_id),
            symbol_id: Some(symbol_id),
            path,
            entry_kind: None,
            source_kind: "TEST_HARNESS".to_string(),
            confidence: 0.90,
            provenance: builder.provenance("test_harness_current_state"),
            evidence: json!([row]),
            properties: json!({}),
            last_observed_generation: Some(builder.generation),
        });
        builder.upsert_edge_by_kind(
            ArchitectureGraphEdgeKind::Contains,
            system_id.clone(),
            test_node_id,
            "TEST_HARNESS",
            0.80,
            builder.provenance("test_harness_current_state"),
            json!([]),
            json!({}),
        );
    }

    let edge_rows = match optional_query(
        context,
        &format!(
            "SELECT edge_id, from_artefact_id, from_symbol_id, to_artefact_id, to_symbol_id, edge_kind, path, start_line, end_line \
             FROM test_artefact_edges_current WHERE repo_id = '{}' ORDER BY edge_id",
            crate::host::devql::esc_pg(&builder.repo_id)
        ),
        warnings,
    )
    .await
    {
        Some(rows) => rows,
        None => return,
    };

    for row in edge_rows {
        let test_node = string_field(&row, "from_artefact_id")
            .and_then(|id| test_artefact_nodes.get(&id).cloned())
            .or_else(|| {
                string_field(&row, "from_symbol_id")
                    .and_then(|id| test_symbol_nodes.get(&id).cloned())
            });
        let production_node = string_field(&row, "to_artefact_id")
            .and_then(|id| builder.artefact_nodes.get(&id).cloned())
            .or_else(|| {
                string_field(&row, "to_symbol_id")
                    .and_then(|id| builder.symbol_nodes.get(&id).cloned())
            });
        let (Some(production_node), Some(test_node)) = (production_node, test_node) else {
            continue;
        };
        builder.upsert_edge_by_kind(
            ArchitectureGraphEdgeKind::VerifiedBy,
            production_node,
            test_node,
            "TEST_HARNESS",
            0.75,
            builder.provenance("test_harness_current_state"),
            json!([row]),
            json!({}),
        );
    }
}

async fn optional_query(
    context: &CurrentStateConsumerContext,
    sql: &str,
    warnings: &mut Vec<String>,
) -> Option<Vec<Value>> {
    match context.storage.query_rows(sql).await {
        Ok(rows) => Some(rows),
        Err(err) if err.to_string().contains("no such table") => {
            warnings.push(format!(
                "Optional architecture graph source unavailable: {err}"
            ));
            None
        }
        Err(err) => {
            warnings.push(format!(
                "Optional architecture graph source query failed: {err:#}"
            ));
            None
        }
    }
}
