use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::host::capability_host::{
    BoxFuture, CapabilityIngestContext, IngestRequest, IngestResult, IngesterHandler,
    IngesterRegistration,
};
use crate::host::devql::deterministic_uuid;

use super::storage::{
    ArchitectureGraphAssertion, assertion_id, insert_assertion, revoke_assertion,
};
use super::types::{
    ARCHITECTURE_GRAPH_ASSERT_INGESTER_ID, ARCHITECTURE_GRAPH_CAPABILITY_ID,
    ARCHITECTURE_GRAPH_REVOKE_INGESTER_ID, ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_INGESTER_ID,
    ArchitectureGraphAssertionAction, ArchitectureGraphTargetKind,
};
use super::{
    roles::{
        NoopRoleAssignmentWriter, NoopRoleFactsReader, NoopRoleTaxonomyReader,
        RoleAdjudicationMailboxPayload, RoleAdjudicationServices, default_queue_store,
        run_adjudication_request,
    },
    types::ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT,
};

#[derive(Debug, Clone, Deserialize)]
struct AssertArchitectureGraphFactPayload {
    #[serde(default)]
    assertion_id: Option<String>,
    action: String,
    target_kind: String,
    #[serde(default)]
    node_id: Option<String>,
    #[serde(default)]
    node_kind: Option<String>,
    #[serde(default)]
    edge_id: Option<String>,
    #[serde(default)]
    edge_kind: Option<String>,
    #[serde(default)]
    from_node_id: Option<String>,
    #[serde(default)]
    to_node_id: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    artefact_id: Option<String>,
    #[serde(default)]
    symbol_id: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    entry_kind: Option<String>,
    reason: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    confidence: Option<f64>,
    #[serde(default)]
    provenance: Option<Value>,
    #[serde(default)]
    evidence: Option<Value>,
    #[serde(default)]
    properties: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct RevokeArchitectureGraphAssertionPayload {
    id: String,
}

pub struct ArchitectureGraphAssertIngester;

impl IngesterHandler for ArchitectureGraphAssertIngester {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        ctx: &'a mut dyn CapabilityIngestContext,
    ) -> BoxFuture<'a, Result<IngestResult>> {
        Box::pin(async move {
            let payload: AssertArchitectureGraphFactPayload = request
                .parse_json()
                .context("parse architecture_graph.assert payload")?;
            let relational = ctx.devql_relational_scoped(ARCHITECTURE_GRAPH_CAPABILITY_ID)?;
            let assertion = build_assertion(ctx.repo().repo_id.as_str(), payload)?;
            insert_assertion(relational, &assertion).await?;

            Ok(IngestResult::new(
                json!({
                    "capability": ARCHITECTURE_GRAPH_CAPABILITY_ID,
                    "ingester": ARCHITECTURE_GRAPH_ASSERT_INGESTER_ID,
                    "status": "ok",
                    "assertion_id": assertion.assertion_id,
                }),
                format!(
                    "architecture graph assertion `{}` recorded",
                    assertion.assertion_id
                ),
            ))
        })
    }
}

pub struct ArchitectureGraphRevokeIngester;
pub struct ArchitectureRoleAdjudicationIngester;

impl IngesterHandler for ArchitectureGraphRevokeIngester {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        ctx: &'a mut dyn CapabilityIngestContext,
    ) -> BoxFuture<'a, Result<IngestResult>> {
        Box::pin(async move {
            let payload: RevokeArchitectureGraphAssertionPayload = request
                .parse_json()
                .context("parse architecture_graph.revoke payload")?;
            let relational = ctx.devql_relational_scoped(ARCHITECTURE_GRAPH_CAPABILITY_ID)?;
            let id = require_non_empty(payload.id, "id")?;
            let revoked = revoke_assertion(relational, ctx.repo().repo_id.as_str(), &id).await?;

            Ok(IngestResult::new(
                json!({
                    "capability": ARCHITECTURE_GRAPH_CAPABILITY_ID,
                    "ingester": ARCHITECTURE_GRAPH_REVOKE_INGESTER_ID,
                    "status": "ok",
                    "assertion_id": id,
                    "revoked": revoked,
                }),
                if revoked {
                    format!("architecture graph assertion `{id}` revoked")
                } else {
                    format!("architecture graph assertion `{id}` was not active")
                },
            ))
        })
    }
}

impl IngesterHandler for ArchitectureRoleAdjudicationIngester {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        ctx: &'a mut dyn CapabilityIngestContext,
    ) -> BoxFuture<'a, Result<IngestResult>> {
        Box::pin(async move {
            let payload: RoleAdjudicationMailboxPayload = request
                .parse_json()
                .context("parse architecture_graph.role_adjudication payload")?;
            let queue = default_queue_store();
            static TAXONOMY: NoopRoleTaxonomyReader = NoopRoleTaxonomyReader;
            static FACTS: NoopRoleFactsReader = NoopRoleFactsReader;
            static WRITER: NoopRoleAssignmentWriter = NoopRoleAssignmentWriter;

            let services = RoleAdjudicationServices {
                queue: queue.as_ref(),
                taxonomy: &TAXONOMY,
                facts: &FACTS,
                writer: &WRITER,
            };
            let write_outcome = run_adjudication_request(
                &payload.request,
                ctx.inference(),
                ctx.repo_root(),
                &services,
            )
            .with_context(|| {
                format!(
                    "run role adjudication for scope `{}` using slot `{}`",
                    payload.request.scope_key(),
                    ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_SLOT
                )
            })?;

            Ok(IngestResult::new(
                json!({
                    "capability": ARCHITECTURE_GRAPH_CAPABILITY_ID,
                    "ingester": ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_INGESTER_ID,
                    "status": "ok",
                    "scope_key": payload.request.scope_key(),
                    "persisted": write_outcome.persisted,
                    "writer_source": write_outcome.source,
                }),
                if write_outcome.persisted {
                    format!(
                        "architecture role adjudication persisted for `{}`",
                        payload.request.scope_key()
                    )
                } else {
                    format!(
                        "architecture role adjudication completed without persistence for `{}`",
                        payload.request.scope_key()
                    )
                },
            ))
        })
    }
}

pub fn build_assert_ingester() -> IngesterRegistration {
    IngesterRegistration::new(
        ARCHITECTURE_GRAPH_CAPABILITY_ID,
        ARCHITECTURE_GRAPH_ASSERT_INGESTER_ID,
        std::sync::Arc::new(ArchitectureGraphAssertIngester),
    )
}

pub fn build_revoke_ingester() -> IngesterRegistration {
    IngesterRegistration::new(
        ARCHITECTURE_GRAPH_CAPABILITY_ID,
        ARCHITECTURE_GRAPH_REVOKE_INGESTER_ID,
        std::sync::Arc::new(ArchitectureGraphRevokeIngester),
    )
}

pub fn build_role_adjudication_ingester() -> IngesterRegistration {
    IngesterRegistration::new(
        ARCHITECTURE_GRAPH_CAPABILITY_ID,
        ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_INGESTER_ID,
        std::sync::Arc::new(ArchitectureRoleAdjudicationIngester),
    )
}

fn build_assertion(
    repo_id: &str,
    payload: AssertArchitectureGraphFactPayload,
) -> Result<ArchitectureGraphAssertion> {
    let action = normalise_action(&payload.action)?;
    let target_kind = normalise_target_kind(&payload.target_kind)?;
    let reason = require_non_empty(payload.reason.clone(), "reason")?;
    let source = payload
        .source
        .clone()
        .and_then(non_empty_string)
        .unwrap_or_else(|| "devql_mutation".to_string());
    let confidence = payload.confidence;
    if let Some(confidence) = confidence
        && !(0.0..=1.0).contains(&confidence)
    {
        bail!("confidence must be between 0 and 1");
    }

    let identity = assertion_identity(&target_kind, &payload)?;
    let assertion_id = payload
        .assertion_id
        .and_then(non_empty_string)
        .unwrap_or_else(|| {
            assertion_id(
                repo_id,
                &action,
                &target_kind,
                &format!("{action}|{identity}"),
            )
        });

    let node_id = if target_kind == ArchitectureGraphTargetKind::Node.as_str() {
        non_empty_option(payload.node_id.clone()).or_else(|| {
            payload
                .node_kind
                .clone()
                .and_then(non_empty_string)
                .map(normalise_fact_kind)
                .map(|kind| {
                    deterministic_uuid(&format!(
                        "architecture_graph|node|{repo_id}|{kind}|{identity}"
                    ))
                })
        })
    } else {
        non_empty_option(payload.node_id.clone())
    };
    let edge_id = if target_kind == ArchitectureGraphTargetKind::Edge.as_str() {
        non_empty_option(payload.edge_id.clone()).or_else(|| {
            let edge_kind = payload
                .edge_kind
                .clone()
                .and_then(non_empty_string)
                .map(normalise_fact_kind)?;
            let from = non_empty_option(payload.from_node_id.clone())?;
            let to = non_empty_option(payload.to_node_id.clone())?;
            Some(deterministic_uuid(&format!(
                "architecture_graph|edge|{repo_id}|{edge_kind}|{from}|{to}"
            )))
        })
    } else {
        non_empty_option(payload.edge_id.clone())
    };

    Ok(ArchitectureGraphAssertion {
        assertion_id,
        repo_id: repo_id.to_string(),
        action,
        target_kind,
        node_id,
        node_kind: payload
            .node_kind
            .and_then(non_empty_string)
            .map(normalise_fact_kind),
        edge_id,
        edge_kind: payload
            .edge_kind
            .and_then(non_empty_string)
            .map(normalise_fact_kind),
        from_node_id: non_empty_option(payload.from_node_id),
        to_node_id: non_empty_option(payload.to_node_id),
        label: non_empty_option(payload.label),
        artefact_id: non_empty_option(payload.artefact_id),
        symbol_id: non_empty_option(payload.symbol_id),
        path: non_empty_option(payload.path),
        entry_kind: non_empty_option(payload.entry_kind),
        reason,
        source,
        confidence,
        provenance: payload
            .provenance
            .unwrap_or_else(|| json!({ "source": "devql_mutation" })),
        evidence: payload.evidence.unwrap_or_else(|| json!([])),
        properties: payload.properties.unwrap_or_else(|| json!({})),
    })
}

pub(crate) fn build_assertion_from_payload(
    repo_id: &str,
    payload: Value,
) -> Result<ArchitectureGraphAssertion> {
    let payload: AssertArchitectureGraphFactPayload =
        serde_json::from_value(payload).context("parse architecture_graph.assert payload")?;
    build_assertion(repo_id, payload)
}

fn assertion_identity(
    target_kind: &str,
    payload: &AssertArchitectureGraphFactPayload,
) -> Result<String> {
    match target_kind {
        kind if kind == ArchitectureGraphTargetKind::Node.as_str() => {
            if let Some(node_id) = non_empty_option(payload.node_id.clone()) {
                return Ok(format!("node:{node_id}"));
            }
            let node_kind = payload
                .node_kind
                .clone()
                .and_then(non_empty_string)
                .map(normalise_fact_kind)
                .ok_or_else(|| {
                    anyhow!("node assertions require `nodeKind` when `nodeId` is absent")
                })?;
            let label = non_empty_option(payload.label.clone())
                .or_else(|| non_empty_option(payload.path.clone()))
                .or_else(|| non_empty_option(payload.artefact_id.clone()))
                .or_else(|| non_empty_option(payload.symbol_id.clone()))
                .ok_or_else(|| {
                    anyhow!(
                        "node assertions without `nodeId` require one of `label`, `path`, `artefactId`, or `symbolId`"
                    )
                })?;
            Ok(format!("node:{node_kind}:{label}"))
        }
        kind if kind == ArchitectureGraphTargetKind::Edge.as_str() => {
            if let Some(edge_id) = non_empty_option(payload.edge_id.clone()) {
                return Ok(format!("edge:{edge_id}"));
            }
            let edge_kind = payload
                .edge_kind
                .clone()
                .and_then(non_empty_string)
                .map(normalise_fact_kind)
                .ok_or_else(|| {
                    anyhow!("edge assertions require `edgeKind` when `edgeId` is absent")
                })?;
            let from = non_empty_option(payload.from_node_id.clone())
                .ok_or_else(|| anyhow!("edge assertions without `edgeId` require `fromNodeId`"))?;
            let to = non_empty_option(payload.to_node_id.clone())
                .ok_or_else(|| anyhow!("edge assertions without `edgeId` require `toNodeId`"))?;
            Ok(format!("edge:{edge_kind}:{from}:{to}"))
        }
        other => bail!("unsupported architecture graph target kind `{other}`"),
    }
}

fn normalise_action(value: &str) -> Result<String> {
    let normalised = normalise_fact_kind(value);
    match normalised.as_str() {
        action
            if action == ArchitectureGraphAssertionAction::Assert.as_str()
                || action == ArchitectureGraphAssertionAction::Suppress.as_str()
                || action == ArchitectureGraphAssertionAction::Annotate.as_str() =>
        {
            Ok(normalised)
        }
        _ => bail!("unsupported architecture graph assertion action `{value}`"),
    }
}

fn normalise_target_kind(value: &str) -> Result<String> {
    let normalised = normalise_fact_kind(value);
    match normalised.as_str() {
        kind if kind == ArchitectureGraphTargetKind::Node.as_str()
            || kind == ArchitectureGraphTargetKind::Edge.as_str() =>
        {
            Ok(normalised)
        }
        _ => bail!("unsupported architecture graph target kind `{value}`"),
    }
}

fn normalise_fact_kind(value: impl AsRef<str>) -> String {
    let mut out = String::new();
    let mut previous_was_lower_or_digit = false;
    for ch in value.as_ref().trim().chars() {
        if ch == '-' || ch == ' ' {
            if !out.ends_with('_') && !out.is_empty() {
                out.push('_');
            }
            previous_was_lower_or_digit = false;
            continue;
        }
        if ch == '_' {
            if !out.ends_with('_') && !out.is_empty() {
                out.push('_');
            }
            previous_was_lower_or_digit = false;
            continue;
        }
        if ch.is_ascii_uppercase() {
            if previous_was_lower_or_digit && !out.ends_with('_') {
                out.push('_');
            }
            out.push(ch);
            previous_was_lower_or_digit = false;
            continue;
        }
        out.push(ch.to_ascii_uppercase());
        previous_was_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
    }
    out.trim_matches('_').to_string()
}

fn require_non_empty(value: String, field: &str) -> Result<String> {
    non_empty_string(value).ok_or_else(|| anyhow!("{field} must not be empty"))
}

fn non_empty_option(value: Option<String>) -> Option<String> {
    value.and_then(non_empty_string)
}

fn non_empty_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assertion_payload_requires_reason() {
        let err = build_assertion(
            "repo",
            AssertArchitectureGraphFactPayload {
                assertion_id: None,
                action: "ASSERT".to_string(),
                target_kind: "NODE".to_string(),
                node_id: Some("node-1".to_string()),
                node_kind: None,
                edge_id: None,
                edge_kind: None,
                from_node_id: None,
                to_node_id: None,
                label: None,
                artefact_id: None,
                symbol_id: None,
                path: None,
                entry_kind: None,
                reason: " ".to_string(),
                source: None,
                confidence: None,
                provenance: None,
                evidence: None,
                properties: None,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("reason"));
    }
}
