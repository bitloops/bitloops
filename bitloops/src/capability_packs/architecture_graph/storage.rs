use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::host::devql::{RelationalStorage, deterministic_uuid, esc_pg, sql_json_value, sql_now};

use super::types::{
    ArchitectureGraphAssertionAction, ArchitectureGraphEdgeKind, ArchitectureGraphNodeKind,
    ArchitectureGraphTargetKind,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ArchitectureGraphNodeFact {
    pub repo_id: String,
    pub node_id: String,
    pub node_kind: String,
    pub label: String,
    pub artefact_id: Option<String>,
    pub symbol_id: Option<String>,
    pub path: Option<String>,
    pub entry_kind: Option<String>,
    pub source_kind: String,
    pub confidence: f64,
    pub provenance: Value,
    pub evidence: Value,
    pub properties: Value,
    pub last_observed_generation: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArchitectureGraphEdgeFact {
    pub repo_id: String,
    pub edge_id: String,
    pub edge_kind: String,
    pub from_node_id: String,
    pub to_node_id: String,
    pub source_kind: String,
    pub confidence: f64,
    pub provenance: Value,
    pub evidence: Value,
    pub properties: Value,
    pub last_observed_generation: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArchitectureGraphFacts {
    pub nodes: Vec<ArchitectureGraphNodeFact>,
    pub edges: Vec<ArchitectureGraphEdgeFact>,
}

impl ArchitectureGraphFacts {
    pub fn empty() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArchitectureGraphAssertion {
    pub assertion_id: String,
    pub repo_id: String,
    pub action: String,
    pub target_kind: String,
    pub node_id: Option<String>,
    pub node_kind: Option<String>,
    pub edge_id: Option<String>,
    pub edge_kind: Option<String>,
    pub from_node_id: Option<String>,
    pub to_node_id: Option<String>,
    pub label: Option<String>,
    pub artefact_id: Option<String>,
    pub symbol_id: Option<String>,
    pub path: Option<String>,
    pub entry_kind: Option<String>,
    pub reason: String,
    pub source: String,
    pub confidence: Option<f64>,
    pub provenance: Value,
    pub evidence: Value,
    pub properties: Value,
}

pub fn node_id(repo_id: &str, kind: ArchitectureGraphNodeKind, identity: &str) -> String {
    deterministic_uuid(&format!(
        "architecture_graph|node|{repo_id}|{}|{identity}",
        kind.as_str()
    ))
}

pub fn system_node_id(system_key: &str) -> String {
    deterministic_uuid(&format!("architecture_graph|system|{system_key}"))
}

pub fn deployment_unit_node_id(repo_id: &str, deployment_kind: &str, identity: &str) -> String {
    deterministic_uuid(&format!(
        "architecture_graph|deployment_unit|{repo_id}|{deployment_kind}|{identity}"
    ))
}

pub fn container_node_id(repo_id: &str, container_key: &str) -> String {
    deterministic_uuid(&format!(
        "architecture_graph|container|{repo_id}|{container_key}"
    ))
}

pub fn component_node_id(repo_id: &str, container_id: &str, component_key: &str) -> String {
    deterministic_uuid(&format!(
        "architecture_graph|component|{repo_id}|{container_id}|{component_key}"
    ))
}

pub fn edge_id(
    repo_id: &str,
    kind: ArchitectureGraphEdgeKind,
    from_node_id: &str,
    to_node_id: &str,
) -> String {
    edge_id_for_kind(repo_id, kind.as_str(), from_node_id, to_node_id)
}

pub fn edge_id_for_kind(repo_id: &str, kind: &str, from_node_id: &str, to_node_id: &str) -> String {
    deterministic_uuid(&format!(
        "architecture_graph|edge|{repo_id}|{kind}|{from_node_id}|{to_node_id}"
    ))
}

pub fn assertion_id(repo_id: &str, action: &str, target_kind: &str, identity: &str) -> String {
    deterministic_uuid(&format!(
        "architecture_graph|assertion|{repo_id}|{action}|{target_kind}|{identity}"
    ))
}

pub async fn replace_computed_graph(
    relational: &RelationalStorage,
    repo_id: &str,
    facts: ArchitectureGraphFacts,
    generation_seq: u64,
    warnings: &[String],
    metrics: Value,
) -> Result<()> {
    relational
        .replace_architecture_graph_current(repo_id, facts, generation_seq, warnings, metrics)
        .await
        .context("replacing architecture graph computed facts")
}

pub async fn insert_assertion(
    relational: &RelationalStorage,
    assertion: &ArchitectureGraphAssertion,
) -> Result<()> {
    relational
        .exec_serialized(&insert_assertion_sql(relational, assertion))
        .await
        .context("inserting architecture graph assertion")
}

pub async fn revoke_assertion(
    relational: &RelationalStorage,
    repo_id: &str,
    assertion_id: &str,
) -> Result<bool> {
    let before = relational
        .query_rows(&format!(
            "SELECT assertion_id FROM architecture_graph_assertions \
             WHERE repo_id = {} AND assertion_id = {} AND revoked_at IS NULL LIMIT 1",
            sql_text(repo_id),
            sql_text(assertion_id)
        ))
        .await?;
    if before.is_empty() {
        return Ok(false);
    }
    relational
        .exec_serialized(&format!(
            "UPDATE architecture_graph_assertions \
             SET revoked_at = {now}, updated_at = {now} \
             WHERE repo_id = {repo_id} AND assertion_id = {assertion_id};",
            now = sql_now(relational),
            repo_id = sql_text(repo_id),
            assertion_id = sql_text(assertion_id),
        ))
        .await?;
    Ok(true)
}

fn insert_assertion_sql(
    relational: &RelationalStorage,
    assertion: &ArchitectureGraphAssertion,
) -> String {
    format!(
        "INSERT INTO architecture_graph_assertions (
            assertion_id, repo_id, action, target_kind, node_id, node_kind, edge_id, edge_kind,
            from_node_id, to_node_id, label, artefact_id, symbol_id, path, entry_kind,
            reason, source, confidence, provenance_json, evidence_json, properties_json,
            created_at, updated_at
        ) VALUES (
            {assertion_id}, {repo_id}, {action}, {target_kind}, {node_id}, {node_kind}, {edge_id}, {edge_kind},
            {from_node_id}, {to_node_id}, {label}, {artefact_id}, {symbol_id}, {path}, {entry_kind},
            {reason}, {source}, {confidence}, {provenance}, {evidence}, {properties}, {now}, {now}
        )
        ON CONFLICT(assertion_id) DO UPDATE SET
            reason = excluded.reason,
            source = excluded.source,
            confidence = excluded.confidence,
            provenance_json = excluded.provenance_json,
            evidence_json = excluded.evidence_json,
            properties_json = excluded.properties_json,
            revoked_at = NULL,
            updated_at = excluded.updated_at;",
        assertion_id = sql_text(&assertion.assertion_id),
        repo_id = sql_text(&assertion.repo_id),
        action = sql_text(&assertion.action),
        target_kind = sql_text(&assertion.target_kind),
        node_id = sql_opt_text(assertion.node_id.as_deref()),
        node_kind = sql_opt_text(assertion.node_kind.as_deref()),
        edge_id = sql_opt_text(assertion.edge_id.as_deref()),
        edge_kind = sql_opt_text(assertion.edge_kind.as_deref()),
        from_node_id = sql_opt_text(assertion.from_node_id.as_deref()),
        to_node_id = sql_opt_text(assertion.to_node_id.as_deref()),
        label = sql_opt_text(assertion.label.as_deref()),
        artefact_id = sql_opt_text(assertion.artefact_id.as_deref()),
        symbol_id = sql_opt_text(assertion.symbol_id.as_deref()),
        path = sql_opt_text(assertion.path.as_deref()),
        entry_kind = sql_opt_text(assertion.entry_kind.as_deref()),
        reason = sql_text(&assertion.reason),
        source = sql_text(&assertion.source),
        confidence = assertion
            .confidence
            .map(|value| value.to_string())
            .unwrap_or_else(|| "NULL".to_string()),
        provenance = sql_json_value(relational, &assertion.provenance),
        evidence = sql_json_value(relational, &assertion.evidence),
        properties = sql_json_value(relational, &assertion.properties),
        now = sql_now(relational),
    )
}

pub fn assertion_from_node(
    repo_id: &str,
    action: ArchitectureGraphAssertionAction,
    node_id: String,
    node_kind: ArchitectureGraphNodeKind,
    label: Option<String>,
    reason: String,
    source: String,
) -> ArchitectureGraphAssertion {
    let identity = format!("{}|{}|{}", action.as_str(), node_kind.as_str(), node_id);
    ArchitectureGraphAssertion {
        assertion_id: assertion_id(
            repo_id,
            action.as_str(),
            ArchitectureGraphTargetKind::Node.as_str(),
            &identity,
        ),
        repo_id: repo_id.to_string(),
        action: action.as_str().to_string(),
        target_kind: ArchitectureGraphTargetKind::Node.as_str().to_string(),
        node_id: Some(node_id),
        node_kind: Some(node_kind.as_str().to_string()),
        edge_id: None,
        edge_kind: None,
        from_node_id: None,
        to_node_id: None,
        label,
        artefact_id: None,
        symbol_id: None,
        path: None,
        entry_kind: None,
        reason,
        source,
        confidence: None,
        provenance: json!({ "source": "devql_mutation" }),
        evidence: json!([]),
        properties: json!({}),
    }
}

fn sql_text(value: &str) -> String {
    format!("'{}'", esc_pg(value))
}

fn sql_opt_text(value: Option<&str>) -> String {
    value.map(sql_text).unwrap_or_else(|| "NULL".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_ids_are_deterministic() {
        let left = node_id(
            "repo",
            ArchitectureGraphNodeKind::EntryPoint,
            "src/main.rs::main",
        );
        let right = node_id(
            "repo",
            ArchitectureGraphNodeKind::EntryPoint,
            "src/main.rs::main",
        );
        assert_eq!(left, right);
    }

    #[test]
    fn c4_ids_use_expected_identity_scopes() {
        let shared_system_left = system_node_id("bitloops.platform");
        let shared_system_right = system_node_id("bitloops.platform");
        assert_eq!(shared_system_left, shared_system_right);

        let left_container = container_node_id("repo-a", "cli");
        let right_container = container_node_id("repo-b", "cli");
        assert_ne!(left_container, right_container);

        let deployment = deployment_unit_node_id("repo-a", "cargo_bin", "crates/cli");
        assert_eq!(
            deployment,
            deployment_unit_node_id("repo-a", "cargo_bin", "crates/cli")
        );
    }

    #[test]
    fn assertion_identity_changes_by_action() {
        let assert_id = assertion_id("repo", "ASSERT", "NODE", "node-1");
        let suppress_id = assertion_id("repo", "SUPPRESS", "NODE", "node-1");
        assert_ne!(assert_id, suppress_id);
    }
}
