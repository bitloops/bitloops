use super::*;

impl GraphBuilder {
    pub(super) fn add_synthesised_facts(&mut self, value: Value) -> Result<(usize, usize)> {
        let output: StructuredArchitectureFacts =
            serde_json::from_value(value).context("parse structured architecture facts")?;
        let mut nodes = Vec::new();
        let mut known_node_ids = self.nodes.keys().cloned().collect::<BTreeSet<_>>();
        for node in output.nodes {
            let kind = parse_node_kind(&node.kind)?;
            let identity = non_empty_string("node.identity", node.identity)?;
            let label = non_empty_string("node.label", node.label)?;
            let confidence = valid_confidence("node.confidence", node.confidence)?;
            let node_id = node_id(&self.repo_id, kind, &identity);
            known_node_ids.insert(node_id.clone());
            nodes.push(ArchitectureGraphNodeFact {
                repo_id: self.repo_id.clone(),
                node_id,
                node_kind: kind.as_str().to_string(),
                label,
                artefact_id: node.artefact_id.filter(|value| !value.trim().is_empty()),
                symbol_id: node.symbol_id.filter(|value| !value.trim().is_empty()),
                path: node.path.filter(|value| !value.trim().is_empty()),
                entry_kind: node.entry_kind.filter(|value| !value.trim().is_empty()),
                source_kind: "AGENT_SYNTHESIS".to_string(),
                confidence,
                provenance: self.provenance("agent_fact_synthesis"),
                evidence: node.evidence.unwrap_or_else(|| json!([])),
                properties: node.properties.unwrap_or_else(|| json!({})),
                last_observed_generation: Some(self.generation),
            });
        }

        let mut edges = Vec::new();
        for edge in output.edges {
            let kind = parse_edge_kind(&edge.kind)?;
            let from_node_id = resolve_structured_endpoint(
                "edge.from",
                edge.from,
                &known_node_ids,
                &self.repo_id,
            )?;
            let to_node_id =
                resolve_structured_endpoint("edge.to", edge.to, &known_node_ids, &self.repo_id)?;
            let confidence = valid_confidence("edge.confidence", edge.confidence)?;
            let edge_id =
                edge_id_for_kind(&self.repo_id, kind.as_str(), &from_node_id, &to_node_id);
            edges.push(ArchitectureGraphEdgeFact {
                repo_id: self.repo_id.clone(),
                edge_id,
                edge_kind: kind.as_str().to_string(),
                from_node_id,
                to_node_id,
                source_kind: "AGENT_SYNTHESIS".to_string(),
                confidence,
                provenance: self.provenance("agent_fact_synthesis"),
                evidence: edge.evidence.unwrap_or_else(|| json!([])),
                properties: edge.properties.unwrap_or_else(|| json!({})),
                last_observed_generation: Some(self.generation),
            });
        }

        let counts = (nodes.len(), edges.len());
        for node in nodes {
            self.upsert_node(node);
        }
        for edge in edges {
            match self.edges.get(&edge.edge_id) {
                Some(existing) if existing.confidence >= edge.confidence => {}
                _ => {
                    self.edges.insert(edge.edge_id.clone(), edge);
                }
            }
        }
        Ok(counts)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StructuredArchitectureFacts {
    #[serde(default)]
    nodes: Vec<StructuredArchitectureNode>,
    #[serde(default)]
    edges: Vec<StructuredArchitectureEdge>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StructuredArchitectureNode {
    kind: String,
    identity: String,
    label: String,
    confidence: f64,
    #[serde(default)]
    artefact_id: Option<String>,
    #[serde(default)]
    symbol_id: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    entry_kind: Option<String>,
    #[serde(default)]
    evidence: Option<Value>,
    #[serde(default)]
    properties: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StructuredArchitectureEdge {
    kind: String,
    from: StructuredArchitectureEndpoint,
    to: StructuredArchitectureEndpoint,
    confidence: f64,
    #[serde(default)]
    evidence: Option<Value>,
    #[serde(default)]
    properties: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StructuredArchitectureEndpoint {
    #[serde(default)]
    node_id: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    identity: Option<String>,
}

pub(super) async fn add_agent_synthesised_facts(
    context: &CurrentStateConsumerContext,
    request: &CurrentStateConsumerRequest,
    builder: &mut GraphBuilder,
) -> Result<Option<(usize, usize)>> {
    if !context
        .inference
        .has_slot(ARCHITECTURE_GRAPH_FACT_SYNTHESIS_SLOT)
    {
        return Ok(None);
    }
    let service = context
        .inference
        .structured_generation(ARCHITECTURE_GRAPH_FACT_SYNTHESIS_SLOT)
        .context("resolving architecture fact_synthesis inference slot")?;
    let mut metadata = Map::new();
    metadata.insert(
        "capability_id".to_string(),
        Value::String(ARCHITECTURE_GRAPH_CAPABILITY_ID.to_string()),
    );
    metadata.insert(
        "slot_name".to_string(),
        Value::String(ARCHITECTURE_GRAPH_FACT_SYNTHESIS_SLOT.to_string()),
    );
    metadata.insert(
        "repo_id".to_string(),
        Value::String(request.repo_id.clone()),
    );
    metadata.insert(
        "generation".to_string(),
        Value::Number(request.to_generation_seq_inclusive.into()),
    );

    let response = service.generate(StructuredGenerationRequest {
        system_prompt: architecture_fact_synthesis_system_prompt().to_string(),
        user_prompt: architecture_fact_synthesis_user_prompt(request, builder),
        json_schema: architecture_fact_synthesis_schema(),
        workspace_path: Some(request.repo_root.display().to_string()),
        metadata,
    })?;

    builder.add_synthesised_facts(response).map(Some)
}

fn architecture_fact_synthesis_system_prompt() -> &'static str {
    "You synthesise architecture graph facts from a repository snapshot. Return only facts supported by the supplied evidence. Do not edit files. Do not invent node ids; use existing node ids only for existing edge endpoints."
}

pub(super) fn architecture_fact_synthesis_user_prompt(
    request: &CurrentStateConsumerRequest,
    builder: &GraphBuilder,
) -> String {
    const PROMPT_SNAPSHOT_NODE_LIMIT: usize = 80;
    const PROMPT_CHANGED_PATH_LIMIT: usize = 80;

    let nodes = builder
        .nodes
        .values()
        .take(PROMPT_SNAPSHOT_NODE_LIMIT)
        .map(|node| {
            json!({
                "node_id": &node.node_id,
                "kind": &node.node_kind,
                "label": &node.label,
                "path": &node.path,
                "artefact_id": &node.artefact_id,
            })
        })
        .collect::<Vec<_>>();
    let changed_paths = request
        .affected_paths
        .iter()
        .chain(request.file_upserts.iter().map(|file| &file.path))
        .take(PROMPT_CHANGED_PATH_LIMIT)
        .cloned()
        .collect::<Vec<_>>();

    json!({
        "task": "Return structured architecture nodes and edges that are strongly supported by this current-state snapshot.",
        "repo_id": &request.repo_id,
        "generation": request.to_generation_seq_inclusive,
        "changed_paths": changed_paths,
        "existing_nodes": nodes,
        "rules": [
            "Use SCREAMING_SNAKE_CASE kind values.",
            "Use confidence between 0 and 1.",
            "For an existing edge endpoint, use {\"node_id\":\"...\"} from existing_nodes.",
            "For an edge endpoint created in this same response, use {\"kind\":\"DOMAIN\",\"identity\":\"payments\"}; the host derives the node id.",
            "Prefer no fact over a weak or unsupported fact."
        ]
    })
    .to_string()
}

fn architecture_fact_synthesis_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "nodes": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "kind": { "type": "string", "enum": node_kind_names() },
                        "identity": { "type": "string", "minLength": 1 },
                        "label": { "type": "string", "minLength": 1 },
                        "confidence": { "type": "number", "minimum": 0, "maximum": 1 },
                        "artefact_id": { "type": ["string", "null"] },
                        "symbol_id": { "type": ["string", "null"] },
                        "path": { "type": ["string", "null"] },
                        "entry_kind": { "type": ["string", "null"] },
                        "evidence": {},
                        "properties": {}
                    },
                    "required": ["kind", "identity", "label", "confidence"],
                    "additionalProperties": false
                }
            },
            "edges": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "kind": { "type": "string", "enum": edge_kind_names() },
                        "from": structured_endpoint_schema(),
                        "to": structured_endpoint_schema(),
                        "confidence": { "type": "number", "minimum": 0, "maximum": 1 },
                        "evidence": {},
                        "properties": {}
                    },
                    "required": ["kind", "from", "to", "confidence"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["nodes", "edges"],
        "additionalProperties": false
    })
}

fn structured_endpoint_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "node_id": { "type": ["string", "null"], "minLength": 1 },
            "kind": { "type": ["string", "null"], "enum": node_kind_names_with_null() },
            "identity": { "type": ["string", "null"], "minLength": 1 }
        },
        "additionalProperties": false
    })
}

fn resolve_structured_endpoint(
    field: &str,
    endpoint: StructuredArchitectureEndpoint,
    known_node_ids: &BTreeSet<String>,
    repo_id: &str,
) -> Result<String> {
    let has_node_id = endpoint
        .node_id
        .as_ref()
        .is_some_and(|value| !value.trim().is_empty());
    let has_kind = endpoint
        .kind
        .as_ref()
        .is_some_and(|value| !value.trim().is_empty());
    let has_identity = endpoint
        .identity
        .as_ref()
        .is_some_and(|value| !value.trim().is_empty());

    if has_node_id && (has_kind || has_identity) {
        bail!("{field} must use either node_id or kind plus identity, not both");
    }

    if has_node_id {
        let node_id = non_empty_string(
            &format!("{field}.node_id"),
            endpoint.node_id.expect("checked above"),
        )?;
        if !known_node_ids.contains(&node_id) {
            bail!("{field}.node_id `{node_id}` does not reference a known node");
        }
        return Ok(node_id);
    }

    if has_kind != has_identity {
        bail!("{field} must include both kind and identity when node_id is absent");
    }

    if has_kind {
        let kind = parse_node_kind(endpoint.kind.as_deref().expect("checked above"))?;
        let identity = non_empty_string(
            &format!("{field}.identity"),
            endpoint.identity.expect("checked above"),
        )?;
        let node_id = node_id(repo_id, kind, &identity);
        if !known_node_ids.contains(&node_id) {
            bail!(
                "{field} kind `{}` identity `{identity}` does not reference a known node",
                kind.as_str()
            );
        }
        return Ok(node_id);
    }

    bail!("{field} must include either node_id or kind plus identity")
}

fn non_empty_string(field: &str, value: String) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("{field} must not be empty");
    }
    Ok(trimmed.to_string())
}

fn valid_confidence(field: &str, value: f64) -> Result<f64> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        bail!("{field} must be a finite number between 0 and 1");
    }
    Ok(value)
}

fn parse_node_kind(raw: &str) -> Result<ArchitectureGraphNodeKind> {
    match raw.trim() {
        "SYSTEM" => Ok(ArchitectureGraphNodeKind::System),
        "DEPLOYMENT_UNIT" => Ok(ArchitectureGraphNodeKind::DeploymentUnit),
        "CONTAINER" => Ok(ArchitectureGraphNodeKind::Container),
        "COMPONENT" => Ok(ArchitectureGraphNodeKind::Component),
        "DOMAIN" => Ok(ArchitectureGraphNodeKind::Domain),
        "ENTITY" => Ok(ArchitectureGraphNodeKind::Entity),
        "CAPABILITY" => Ok(ArchitectureGraphNodeKind::Capability),
        "ENTRY_POINT" => Ok(ArchitectureGraphNodeKind::EntryPoint),
        "FLOW" => Ok(ArchitectureGraphNodeKind::Flow),
        "NODE" => Ok(ArchitectureGraphNodeKind::Node),
        "PERSISTENCE_OBJECT" => Ok(ArchitectureGraphNodeKind::PersistenceObject),
        "EVENT" => Ok(ArchitectureGraphNodeKind::Event),
        "EXTERNAL_SYSTEM" => Ok(ArchitectureGraphNodeKind::ExternalSystem),
        "CONTRACT" => Ok(ArchitectureGraphNodeKind::Contract),
        "TEST" => Ok(ArchitectureGraphNodeKind::Test),
        "CHANGE_UNIT" => Ok(ArchitectureGraphNodeKind::ChangeUnit),
        "RISK_SIGNAL" => Ok(ArchitectureGraphNodeKind::RiskSignal),
        other => bail!("unsupported node kind `{other}`"),
    }
}

fn parse_edge_kind(raw: &str) -> Result<ArchitectureGraphEdgeKind> {
    match raw.trim() {
        "CONTAINS" => Ok(ArchitectureGraphEdgeKind::Contains),
        "OWNS" => Ok(ArchitectureGraphEdgeKind::Owns),
        "EXPOSES" => Ok(ArchitectureGraphEdgeKind::Exposes),
        "PRODUCES" => Ok(ArchitectureGraphEdgeKind::Produces),
        "REALISES" => Ok(ArchitectureGraphEdgeKind::Realises),
        "TRIGGERS" => Ok(ArchitectureGraphEdgeKind::Triggers),
        "TRAVERSES" => Ok(ArchitectureGraphEdgeKind::Traverses),
        "READS" => Ok(ArchitectureGraphEdgeKind::Reads),
        "WRITES" => Ok(ArchitectureGraphEdgeKind::Writes),
        "EMITS" => Ok(ArchitectureGraphEdgeKind::Emits),
        "CALLS" => Ok(ArchitectureGraphEdgeKind::Calls),
        "IMPLEMENTS" => Ok(ArchitectureGraphEdgeKind::Implements),
        "DEPENDS_ON" => Ok(ArchitectureGraphEdgeKind::DependsOn),
        "VERIFIED_BY" => Ok(ArchitectureGraphEdgeKind::VerifiedBy),
        "STORES" => Ok(ArchitectureGraphEdgeKind::Stores),
        "MODIFIES" => Ok(ArchitectureGraphEdgeKind::Modifies),
        "IMPACTS" => Ok(ArchitectureGraphEdgeKind::Impacts),
        "SCORES" => Ok(ArchitectureGraphEdgeKind::Scores),
        other => bail!("unsupported edge kind `{other}`"),
    }
}

fn node_kind_names() -> Vec<&'static str> {
    [
        ArchitectureGraphNodeKind::System,
        ArchitectureGraphNodeKind::DeploymentUnit,
        ArchitectureGraphNodeKind::Container,
        ArchitectureGraphNodeKind::Component,
        ArchitectureGraphNodeKind::Domain,
        ArchitectureGraphNodeKind::Entity,
        ArchitectureGraphNodeKind::Capability,
        ArchitectureGraphNodeKind::EntryPoint,
        ArchitectureGraphNodeKind::Flow,
        ArchitectureGraphNodeKind::Node,
        ArchitectureGraphNodeKind::PersistenceObject,
        ArchitectureGraphNodeKind::Event,
        ArchitectureGraphNodeKind::ExternalSystem,
        ArchitectureGraphNodeKind::Contract,
        ArchitectureGraphNodeKind::Test,
        ArchitectureGraphNodeKind::ChangeUnit,
        ArchitectureGraphNodeKind::RiskSignal,
    ]
    .iter()
    .map(|kind| kind.as_str())
    .collect()
}

fn node_kind_names_with_null() -> Vec<Value> {
    node_kind_names()
        .into_iter()
        .map(|kind| Value::String(kind.to_string()))
        .chain(std::iter::once(Value::Null))
        .collect()
}

fn edge_kind_names() -> Vec<&'static str> {
    [
        ArchitectureGraphEdgeKind::Contains,
        ArchitectureGraphEdgeKind::Owns,
        ArchitectureGraphEdgeKind::Exposes,
        ArchitectureGraphEdgeKind::Produces,
        ArchitectureGraphEdgeKind::Realises,
        ArchitectureGraphEdgeKind::Triggers,
        ArchitectureGraphEdgeKind::Traverses,
        ArchitectureGraphEdgeKind::Reads,
        ArchitectureGraphEdgeKind::Writes,
        ArchitectureGraphEdgeKind::Emits,
        ArchitectureGraphEdgeKind::Calls,
        ArchitectureGraphEdgeKind::Implements,
        ArchitectureGraphEdgeKind::DependsOn,
        ArchitectureGraphEdgeKind::VerifiedBy,
        ArchitectureGraphEdgeKind::Stores,
        ArchitectureGraphEdgeKind::Modifies,
        ArchitectureGraphEdgeKind::Impacts,
        ArchitectureGraphEdgeKind::Scores,
    ]
    .iter()
    .map(|kind| kind.as_str())
    .collect()
}
