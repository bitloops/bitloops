use async_graphql::{Context, Result};
use serde_json::{Map, Value, json};

use crate::capability_packs::architecture_graph::ingesters::build_assertion_from_payload;
use crate::capability_packs::architecture_graph::storage::{
    ArchitectureGraphAssertion, assertion_id, container_node_id, edge_id_for_kind,
    insert_assertion, revoke_assertion, system_node_id,
};
use crate::graphql::DevqlGraphqlContext;
use crate::graphql::types::{
    ArchitectureGraphAssertionResult, ArchitectureGraphEdgeKind, ArchitectureGraphNodeKind,
    ArchitectureGraphTargetKind, ArchitectureSystemMembershipAssertionResult,
    AssertArchitectureGraphFactInput, AssertArchitectureSystemMembershipInput,
    RevokeArchitectureGraphAssertionResult,
};

use super::errors::operation_error;

pub(super) async fn assert_architecture_graph_fact(
    ctx: &Context<'_>,
    input: AssertArchitectureGraphFactInput,
) -> Result<ArchitectureGraphAssertionResult> {
    let context = ctx.data_unchecked::<DevqlGraphqlContext>();
    context.require_repo_write_scope().map_err(|err| {
        operation_error(
            "BAD_USER_INPUT",
            "validation",
            "assertArchitectureGraphFact",
            err,
        )
    })?;
    let payload = assertion_payload(input)?;
    let assertion = build_assertion_from_payload(context.repo_id(), payload).map_err(|err| {
        map_architecture_graph_operation_error("assertArchitectureGraphFact", err)
    })?;
    let relational = context
        .open_relational_storage("assertArchitectureGraphFact")
        .await
        .map_err(|err| {
            operation_error(
                "BACKEND_ERROR",
                "configuration",
                "assertArchitectureGraphFact",
                err,
            )
        })?;
    insert_assertion(&relational, &assertion)
        .await
        .map_err(|err| {
            map_architecture_graph_operation_error("assertArchitectureGraphFact", err)
        })?;

    Ok(ArchitectureGraphAssertionResult {
        success: true,
        assertion_id: assertion.assertion_id,
    })
}

pub(super) async fn revoke_architecture_graph_assertion(
    ctx: &Context<'_>,
    id: String,
) -> Result<RevokeArchitectureGraphAssertionResult> {
    let context = ctx.data_unchecked::<DevqlGraphqlContext>();
    context.require_repo_write_scope().map_err(|err| {
        operation_error(
            "BAD_USER_INPUT",
            "validation",
            "revokeArchitectureGraphAssertion",
            err,
        )
    })?;
    let id = require_non_empty(id, "id", "revokeArchitectureGraphAssertion")?;
    let relational = context
        .open_relational_storage("revokeArchitectureGraphAssertion")
        .await
        .map_err(|err| {
            operation_error(
                "BACKEND_ERROR",
                "configuration",
                "revokeArchitectureGraphAssertion",
                err,
            )
        })?;
    let revoked = revoke_assertion(&relational, context.repo_id(), &id)
        .await
        .map_err(|err| {
            map_architecture_graph_operation_error("revokeArchitectureGraphAssertion", err)
        })?;

    Ok(RevokeArchitectureGraphAssertionResult {
        success: true,
        id,
        revoked,
    })
}

pub(super) async fn assert_architecture_system_membership(
    ctx: &Context<'_>,
    input: AssertArchitectureSystemMembershipInput,
) -> Result<ArchitectureSystemMembershipAssertionResult> {
    let context = ctx.data_unchecked::<DevqlGraphqlContext>();
    let operation = "assertArchitectureSystemMembership";
    let system_key = require_non_empty(input.system_key, "systemKey", operation)?;
    let reason = require_non_empty(input.reason, "reason", operation)?;
    let confidence = input.confidence.unwrap_or(0.90);
    if !(0.0..=1.0).contains(&confidence) {
        return Err(operation_error(
            "BAD_USER_INPUT",
            "validation",
            operation,
            "`confidence` must be between 0 and 1",
        ));
    }
    let repo_id = resolve_membership_repo_id(
        context,
        input.repository.as_deref(),
        input.container_id.as_deref(),
        input.deployment_unit_id.as_deref(),
        operation,
    )
    .await?;
    let container_key = input
        .container_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let container_id = input
        .container_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            container_key
                .as_deref()
                .map(|container_key| container_node_id(&repo_id, container_key))
        })
        .ok_or_else(|| {
            operation_error(
                "BAD_USER_INPUT",
                "validation",
                operation,
                "`containerId` or `containerKey` is required",
            )
        })?;
    let system_id = system_node_id(&system_key);
    let source = input
        .source
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("devql_mutation")
        .to_string();
    let base_properties = input
        .properties
        .map(|properties| properties.0)
        .unwrap_or_else(|| json!({}));
    let system_properties = merge_properties(
        &base_properties,
        json!({
            "system_key": system_key,
        }),
    );
    let container_properties = merge_properties(
        &base_properties,
        json!({
            "system_key": system_key,
            "container_key": container_key,
            "container_kind": input.container_kind,
        }),
    );
    let system_assertion = node_assertion(
        &repo_id,
        &system_id,
        ArchitectureGraphNodeKind::System,
        input
            .system_label
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&system_key)
            .to_string(),
        reason.clone(),
        source.clone(),
        confidence,
        system_properties,
    );
    let container_assertion = node_assertion(
        &repo_id,
        &container_id,
        ArchitectureGraphNodeKind::Container,
        input
            .container_label
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or(container_key.as_deref())
            .unwrap_or(&container_id)
            .to_string(),
        reason.clone(),
        source.clone(),
        confidence,
        container_properties,
    );
    let contains_assertion = edge_assertion(
        &repo_id,
        ArchitectureGraphEdgeKind::Contains,
        &system_id,
        &container_id,
        reason.clone(),
        source.clone(),
        confidence,
        json!({ "system_key": system_key }),
    );
    let mut assertions = vec![system_assertion, container_assertion, contains_assertion];
    if let Some(deployment_unit_id) = input
        .deployment_unit_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        assertions.push(edge_assertion(
            &repo_id,
            ArchitectureGraphEdgeKind::Realises,
            deployment_unit_id,
            &container_id,
            reason,
            source,
            confidence,
            json!({ "system_key": system_key }),
        ));
    }

    let relational = context
        .open_relational_storage(operation)
        .await
        .map_err(|err| operation_error("BACKEND_ERROR", "configuration", operation, err))?;
    let mut assertion_ids = Vec::new();
    for assertion in &assertions {
        insert_assertion(&relational, assertion)
            .await
            .map_err(|err| map_architecture_graph_operation_error(operation, err))?;
        assertion_ids.push(assertion.assertion_id.clone());
    }

    Ok(ArchitectureSystemMembershipAssertionResult {
        success: true,
        system_id,
        container_id,
        assertion_ids,
    })
}

fn assertion_payload(input: AssertArchitectureGraphFactInput) -> Result<Value> {
    let reason = require_non_empty(input.reason, "reason", "assertArchitectureGraphFact")?;
    let mut payload = Map::new();
    payload.insert(
        "action".to_string(),
        Value::String(input.action.as_db().to_string()),
    );
    payload.insert(
        "target_kind".to_string(),
        Value::String(input.target_kind.as_db().to_string()),
    );
    payload.insert("reason".to_string(), Value::String(reason));
    insert_optional_string(&mut payload, "source", input.source);
    if let Some(confidence) = input.confidence {
        if !(0.0..=1.0).contains(&confidence) {
            return Err(operation_error(
                "BAD_USER_INPUT",
                "validation",
                "assertArchitectureGraphFact",
                "`confidence` must be between 0 and 1",
            ));
        }
        payload.insert("confidence".to_string(), json!(confidence));
    }
    insert_optional_json(&mut payload, "provenance", input.provenance);
    insert_optional_json(&mut payload, "evidence", input.evidence);
    insert_optional_json(&mut payload, "properties", input.properties);

    match input.target_kind {
        ArchitectureGraphTargetKind::Node => {
            let Some(node) = input.node else {
                return Err(operation_error(
                    "BAD_USER_INPUT",
                    "validation",
                    "assertArchitectureGraphFact",
                    "`node` is required when `targetKind` is NODE",
                ));
            };
            if input.edge.is_some() {
                return Err(operation_error(
                    "BAD_USER_INPUT",
                    "validation",
                    "assertArchitectureGraphFact",
                    "`edge` must be omitted when `targetKind` is NODE",
                ));
            }
            insert_optional_string(&mut payload, "node_id", node.id);
            payload.insert(
                "node_kind".to_string(),
                Value::String(node.kind.as_db().to_string()),
            );
            insert_optional_string(&mut payload, "label", node.label);
            insert_optional_string(&mut payload, "artefact_id", node.artefact_id);
            insert_optional_string(&mut payload, "symbol_id", node.symbol_id);
            insert_optional_string(&mut payload, "path", node.path);
            insert_optional_string(&mut payload, "entry_kind", node.entry_kind);
        }
        ArchitectureGraphTargetKind::Edge => {
            let Some(edge) = input.edge else {
                return Err(operation_error(
                    "BAD_USER_INPUT",
                    "validation",
                    "assertArchitectureGraphFact",
                    "`edge` is required when `targetKind` is EDGE",
                ));
            };
            if input.node.is_some() {
                return Err(operation_error(
                    "BAD_USER_INPUT",
                    "validation",
                    "assertArchitectureGraphFact",
                    "`node` must be omitted when `targetKind` is EDGE",
                ));
            }
            insert_optional_string(&mut payload, "edge_id", edge.id);
            payload.insert(
                "edge_kind".to_string(),
                Value::String(edge.kind.as_db().to_string()),
            );
            payload.insert("from_node_id".to_string(), Value::String(edge.from_node_id));
            payload.insert("to_node_id".to_string(), Value::String(edge.to_node_id));
        }
    }
    Ok(Value::Object(payload))
}

async fn resolve_membership_repo_id(
    context: &DevqlGraphqlContext,
    repository: Option<&str>,
    container_id: Option<&str>,
    deployment_unit_id: Option<&str>,
    operation: &'static str,
) -> Result<String> {
    if let Some(repository) = repository.map(str::trim).filter(|value| !value.is_empty()) {
        return context
            .resolve_repository_selection(repository)
            .await
            .map(|repository| repository.repo_id().to_string())
            .map_err(|err| operation_error("BAD_USER_INPUT", "validation", operation, err));
    }
    if let Some(repo_id) =
        unique_repo_for_architecture_targets(context, container_id, deployment_unit_id).await?
    {
        return Ok(repo_id);
    }
    context
        .require_repo_write_scope()
        .map(|()| context.repo_id().to_string())
        .map_err(|err| operation_error("BAD_USER_INPUT", "validation", operation, err))
}

async fn unique_repo_for_architecture_targets(
    context: &DevqlGraphqlContext,
    container_id: Option<&str>,
    deployment_unit_id: Option<&str>,
) -> Result<Option<String>> {
    let ids = [container_id, deployment_unit_id]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Ok(None);
    }
    let id_list = ids
        .iter()
        .map(|id| format!("'{}'", crate::host::devql::esc_pg(id)))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT DISTINCT repo_id FROM architecture_graph_nodes_current WHERE node_id IN ({id_list}) \
         UNION SELECT DISTINCT repo_id FROM architecture_graph_assertions WHERE node_id IN ({id_list})"
    );
    let rows = context.query_devql_sqlite_rows(&sql).await?;
    let mut repo_ids = rows
        .iter()
        .filter_map(|row| row.get("repo_id").and_then(Value::as_str))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    repo_ids.sort();
    repo_ids.dedup();
    Ok(match repo_ids.as_slice() {
        [repo_id] => Some(repo_id.clone()),
        _ => None,
    })
}

fn node_assertion(
    repo_id: &str,
    node_id: &str,
    node_kind: ArchitectureGraphNodeKind,
    label: String,
    reason: String,
    source: String,
    confidence: f64,
    properties: Value,
) -> ArchitectureGraphAssertion {
    ArchitectureGraphAssertion {
        assertion_id: assertion_id(
            repo_id,
            "ASSERT",
            ArchitectureGraphTargetKind::Node.as_db(),
            &format!("node:{node_id}"),
        ),
        repo_id: repo_id.to_string(),
        action: "ASSERT".to_string(),
        target_kind: ArchitectureGraphTargetKind::Node.as_db().to_string(),
        node_id: Some(node_id.to_string()),
        node_kind: Some(node_kind.as_db().to_string()),
        edge_id: None,
        edge_kind: None,
        from_node_id: None,
        to_node_id: None,
        label: Some(label),
        artefact_id: None,
        symbol_id: None,
        path: None,
        entry_kind: None,
        reason,
        source,
        confidence: Some(confidence),
        provenance: json!({ "source": "assertArchitectureSystemMembership" }),
        evidence: json!([]),
        properties,
    }
}

fn edge_assertion(
    repo_id: &str,
    edge_kind: ArchitectureGraphEdgeKind,
    from_node_id: &str,
    to_node_id: &str,
    reason: String,
    source: String,
    confidence: f64,
    properties: Value,
) -> ArchitectureGraphAssertion {
    let edge_id = edge_id_for_kind(repo_id, edge_kind.as_db(), from_node_id, to_node_id);
    ArchitectureGraphAssertion {
        assertion_id: assertion_id(
            repo_id,
            "ASSERT",
            ArchitectureGraphTargetKind::Edge.as_db(),
            &format!("edge:{edge_id}"),
        ),
        repo_id: repo_id.to_string(),
        action: "ASSERT".to_string(),
        target_kind: ArchitectureGraphTargetKind::Edge.as_db().to_string(),
        node_id: None,
        node_kind: None,
        edge_id: Some(edge_id),
        edge_kind: Some(edge_kind.as_db().to_string()),
        from_node_id: Some(from_node_id.to_string()),
        to_node_id: Some(to_node_id.to_string()),
        label: None,
        artefact_id: None,
        symbol_id: None,
        path: None,
        entry_kind: None,
        reason,
        source,
        confidence: Some(confidence),
        provenance: json!({ "source": "assertArchitectureSystemMembership" }),
        evidence: json!([]),
        properties,
    }
}

fn merge_properties(base: &Value, extra: Value) -> Value {
    let mut merged = match base {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    if let Value::Object(extra) = extra {
        for (key, value) in extra {
            if !value.is_null() {
                merged.insert(key, value);
            }
        }
    }
    Value::Object(merged)
}

fn insert_optional_json(
    payload: &mut Map<String, Value>,
    key: &str,
    value: Option<crate::graphql::types::JsonScalar>,
) {
    if let Some(value) = value {
        payload.insert(key.to_string(), value.0);
    }
}

fn insert_optional_string(payload: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        payload.insert(key.to_string(), Value::String(value));
    }
}

fn require_non_empty(value: String, field: &str, operation: &'static str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(operation_error(
            "BAD_USER_INPUT",
            "validation",
            operation,
            format!("`{field}` must not be empty"),
        ));
    }
    Ok(trimmed.to_string())
}

fn map_architecture_graph_operation_error(
    operation: &'static str,
    error: impl std::fmt::Display,
) -> async_graphql::Error {
    let message = error.to_string();
    let lower = message.to_ascii_lowercase();
    if lower.contains("must not be empty")
        || lower.contains("requires")
        || lower.contains("unsupported architecture graph")
        || lower.contains("confidence")
    {
        return operation_error("BAD_USER_INPUT", "validation", operation, message);
    }
    operation_error("BACKEND_ERROR", "architecture_graph", operation, message)
}
