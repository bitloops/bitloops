use super::*;

pub(super) async fn load_computed_nodes(
    context: &DevqlGraphqlContext,
    repo_id: &str,
    scope: &ResolverScope,
    filter: Option<&ArchitectureGraphFilterInput>,
) -> Result<BTreeMap<String, ArchitectureGraphNode>> {
    let mut clauses = vec![format!("repo_id = {}", sql_text(repo_id))];
    if let Some(node_kind) = filter.and_then(|filter| filter.node_kind) {
        clauses.push(format!("node_kind = {}", sql_text(node_kind.as_db())));
    }
    if let Some(source_kind) = filter.and_then(|filter| filter.source_kind.as_deref()) {
        clauses.push(format!("source_kind = {}", sql_text(source_kind)));
    }
    let sql = format!(
        "SELECT node_id, node_kind, label, artefact_id, symbol_id, path, entry_kind, source_kind, confidence, \
                provenance_json, evidence_json, properties_json \
         FROM architecture_graph_nodes_current WHERE {} ORDER BY node_id",
        clauses.join(" AND ")
    );
    let rows = context.query_devql_sqlite_rows(&sql).await?;
    let mut nodes = BTreeMap::new();
    for row in rows {
        let node = node_from_row(&row)?;
        if !node_path_in_scope(&node, scope, filter) {
            continue;
        }
        nodes.insert(node.id.clone(), node);
    }
    Ok(nodes)
}

pub(super) async fn load_computed_edges(
    context: &DevqlGraphqlContext,
    repo_id: &str,
    filter: Option<&ArchitectureGraphFilterInput>,
) -> Result<BTreeMap<String, ArchitectureGraphEdge>> {
    let mut clauses = vec![format!("repo_id = {}", sql_text(repo_id))];
    if let Some(edge_kind) = filter.and_then(|filter| filter.edge_kind) {
        clauses.push(format!("edge_kind = {}", sql_text(edge_kind.as_db())));
    }
    if let Some(source_kind) = filter.and_then(|filter| filter.source_kind.as_deref()) {
        clauses.push(format!("source_kind = {}", sql_text(source_kind)));
    }
    let sql = format!(
        "SELECT edge_id, edge_kind, from_node_id, to_node_id, source_kind, confidence, \
                provenance_json, evidence_json, properties_json \
         FROM architecture_graph_edges_current WHERE {} ORDER BY edge_id",
        clauses.join(" AND ")
    );
    let rows = context.query_devql_sqlite_rows(&sql).await?;
    let mut edges = BTreeMap::new();
    for row in rows {
        let edge = edge_from_row(&row)?;
        edges.insert(edge.id.clone(), edge);
    }
    Ok(edges)
}

pub(super) async fn load_assertions(
    context: &DevqlGraphqlContext,
    repo_id: &str,
) -> Result<Vec<AssertionRecord>> {
    let sql = format!(
        "SELECT assertion_id, action, target_kind, node_id, node_kind, edge_id, edge_kind, \
                from_node_id, to_node_id, label, artefact_id, symbol_id, path, entry_kind, \
                reason, source, confidence, provenance_json, evidence_json, properties_json \
         FROM architecture_graph_assertions \
         WHERE repo_id = {} AND revoked_at IS NULL ORDER BY created_at ASC, assertion_id ASC",
        sql_text(repo_id)
    );
    context
        .query_devql_sqlite_rows(&sql)
        .await?
        .into_iter()
        .map(|row| assertion_from_row(&row))
        .collect()
}

pub(super) fn apply_assertions(
    nodes: &mut BTreeMap<String, ArchitectureGraphNode>,
    edges: &mut BTreeMap<String, ArchitectureGraphEdge>,
    assertions: Vec<AssertionRecord>,
) {
    for assertion in assertions {
        match (assertion.action, assertion.target_kind) {
            (ArchitectureGraphAssertionAction::Suppress, ArchitectureGraphTargetKind::Node) => {
                if let Some(node_id) = assertion.node_id.as_ref()
                    && let Some(node) = nodes.get_mut(node_id)
                {
                    node.suppressed = true;
                    node.effective = false;
                    node.annotations.push(assertion.summary());
                }
            }
            (ArchitectureGraphAssertionAction::Suppress, ArchitectureGraphTargetKind::Edge) => {
                if let Some(edge_id) = assertion.edge_id.as_ref()
                    && let Some(edge) = edges.get_mut(edge_id)
                {
                    edge.suppressed = true;
                    edge.effective = false;
                    edge.annotations.push(assertion.summary());
                }
            }
            (ArchitectureGraphAssertionAction::Annotate, ArchitectureGraphTargetKind::Node) => {
                if let Some(node_id) = assertion.node_id.as_ref()
                    && let Some(node) = nodes.get_mut(node_id)
                {
                    node.annotations.push(assertion.summary());
                }
            }
            (ArchitectureGraphAssertionAction::Annotate, ArchitectureGraphTargetKind::Edge) => {
                if let Some(edge_id) = assertion.edge_id.as_ref()
                    && let Some(edge) = edges.get_mut(edge_id)
                {
                    edge.annotations.push(assertion.summary());
                }
            }
            (ArchitectureGraphAssertionAction::Assert, ArchitectureGraphTargetKind::Node) => {
                apply_node_assertion(nodes, assertion);
            }
            (ArchitectureGraphAssertionAction::Assert, ArchitectureGraphTargetKind::Edge) => {
                apply_edge_assertion(edges, assertion);
            }
        }
    }
}

pub(super) fn apply_node_assertion(
    nodes: &mut BTreeMap<String, ArchitectureGraphNode>,
    assertion: AssertionRecord,
) {
    let Some(node_id) = assertion.node_id.clone() else {
        return;
    };
    if let Some(node) = nodes.get_mut(&node_id) {
        node.asserted = true;
        node.asserted_provenance = Json(assertion.provenance.clone());
        node.provenance = Json(merge_provenance(
            &node.computed_provenance.0,
            &assertion.provenance,
        ));
        node.annotations.push(assertion.summary());
        return;
    }
    let Some(kind) = assertion.node_kind else {
        return;
    };
    let label = assertion
        .label
        .clone()
        .or_else(|| assertion.path.clone())
        .or_else(|| assertion.artefact_id.clone())
        .unwrap_or_else(|| node_id.clone());
    nodes.insert(
        node_id.clone(),
        ArchitectureGraphNode {
            id: node_id,
            kind,
            label,
            artefact_id: assertion.artefact_id,
            symbol_id: assertion.symbol_id,
            path: assertion.path,
            entry_kind: assertion.entry_kind,
            source_kind: assertion.source.clone(),
            confidence: assertion.confidence.unwrap_or(0.85),
            computed: false,
            asserted: true,
            suppressed: false,
            effective: true,
            provenance: Json(assertion.provenance.clone()),
            computed_provenance: Json(Value::Null),
            asserted_provenance: Json(assertion.provenance),
            evidence: Json(assertion.evidence),
            properties: Json(assertion.properties),
            annotations: Vec::new(),
        },
    );
}

pub(super) fn apply_edge_assertion(
    edges: &mut BTreeMap<String, ArchitectureGraphEdge>,
    assertion: AssertionRecord,
) {
    let Some(edge_id) = assertion.edge_id.clone() else {
        return;
    };
    if let Some(edge) = edges.get_mut(&edge_id) {
        edge.asserted = true;
        edge.asserted_provenance = Json(assertion.provenance.clone());
        edge.provenance = Json(merge_provenance(
            &edge.computed_provenance.0,
            &assertion.provenance,
        ));
        edge.annotations.push(assertion.summary());
        return;
    }
    let (Some(kind), Some(from_node_id), Some(to_node_id)) = (
        assertion.edge_kind,
        assertion.from_node_id.clone(),
        assertion.to_node_id.clone(),
    ) else {
        return;
    };
    edges.insert(
        edge_id.clone(),
        ArchitectureGraphEdge {
            id: edge_id,
            kind,
            from_node_id,
            to_node_id,
            source_kind: assertion.source.clone(),
            confidence: assertion.confidence.unwrap_or(0.85),
            computed: false,
            asserted: true,
            suppressed: false,
            effective: true,
            provenance: Json(assertion.provenance.clone()),
            computed_provenance: Json(Value::Null),
            asserted_provenance: Json(assertion.provenance),
            evidence: Json(assertion.evidence),
            properties: Json(assertion.properties),
            annotations: Vec::new(),
        },
    );
}

pub(super) fn node_from_row(row: &Value) -> Result<ArchitectureGraphNode> {
    let id = required_string(row, "node_id")?;
    let kind = ArchitectureGraphNodeKind::from_db(&required_string(row, "node_kind")?)
        .with_context(|| format!("unknown architecture graph node kind for `{id}`"))?;
    let provenance = json_column(row, "provenance_json")?;
    Ok(ArchitectureGraphNode {
        id,
        kind,
        label: required_string(row, "label")?,
        artefact_id: optional_string(row, "artefact_id"),
        symbol_id: optional_string(row, "symbol_id"),
        path: optional_string(row, "path"),
        entry_kind: optional_string(row, "entry_kind"),
        source_kind: required_string(row, "source_kind")?,
        confidence: number_field(row, "confidence").unwrap_or(1.0),
        computed: true,
        asserted: false,
        suppressed: false,
        effective: true,
        provenance: Json(provenance.clone()),
        computed_provenance: Json(provenance),
        asserted_provenance: Json(Value::Null),
        evidence: Json(json_column(row, "evidence_json")?),
        properties: Json(json_column(row, "properties_json")?),
        annotations: Vec::new(),
    })
}

pub(super) fn edge_from_row(row: &Value) -> Result<ArchitectureGraphEdge> {
    let id = required_string(row, "edge_id")?;
    let kind = ArchitectureGraphEdgeKind::from_db(&required_string(row, "edge_kind")?)
        .with_context(|| format!("unknown architecture graph edge kind for `{id}`"))?;
    let provenance = json_column(row, "provenance_json")?;
    Ok(ArchitectureGraphEdge {
        id,
        kind,
        from_node_id: required_string(row, "from_node_id")?,
        to_node_id: required_string(row, "to_node_id")?,
        source_kind: required_string(row, "source_kind")?,
        confidence: number_field(row, "confidence").unwrap_or(1.0),
        computed: true,
        asserted: false,
        suppressed: false,
        effective: true,
        provenance: Json(provenance.clone()),
        computed_provenance: Json(provenance),
        asserted_provenance: Json(Value::Null),
        evidence: Json(json_column(row, "evidence_json")?),
        properties: Json(json_column(row, "properties_json")?),
        annotations: Vec::new(),
    })
}

pub(super) fn assertion_from_row(row: &Value) -> Result<AssertionRecord> {
    let action = ArchitectureGraphAssertionAction::from_db(&required_string(row, "action")?)
        .context("unknown architecture graph assertion action")?;
    let target_kind = ArchitectureGraphTargetKind::from_db(&required_string(row, "target_kind")?)
        .context("unknown architecture graph assertion target kind")?;
    Ok(AssertionRecord {
        id: required_string(row, "assertion_id")?,
        action,
        target_kind,
        node_id: optional_string(row, "node_id"),
        node_kind: optional_string(row, "node_kind")
            .and_then(|kind| ArchitectureGraphNodeKind::from_db(&kind)),
        edge_id: optional_string(row, "edge_id"),
        edge_kind: optional_string(row, "edge_kind")
            .and_then(|kind| ArchitectureGraphEdgeKind::from_db(&kind)),
        from_node_id: optional_string(row, "from_node_id"),
        to_node_id: optional_string(row, "to_node_id"),
        label: optional_string(row, "label"),
        artefact_id: optional_string(row, "artefact_id"),
        symbol_id: optional_string(row, "symbol_id"),
        path: optional_string(row, "path"),
        entry_kind: optional_string(row, "entry_kind"),
        reason: required_string(row, "reason")?,
        source: required_string(row, "source")?,
        confidence: number_field(row, "confidence"),
        provenance: json_column(row, "provenance_json")?,
        evidence: json_column(row, "evidence_json")?,
        properties: json_column(row, "properties_json")?,
    })
}

#[derive(Debug, Clone)]
pub(super) struct AssertionRecord {
    pub(super) id: String,
    pub(super) action: ArchitectureGraphAssertionAction,
    pub(super) target_kind: ArchitectureGraphTargetKind,
    pub(super) node_id: Option<String>,
    pub(super) node_kind: Option<ArchitectureGraphNodeKind>,
    pub(super) edge_id: Option<String>,
    pub(super) edge_kind: Option<ArchitectureGraphEdgeKind>,
    pub(super) from_node_id: Option<String>,
    pub(super) to_node_id: Option<String>,
    pub(super) label: Option<String>,
    pub(super) artefact_id: Option<String>,
    pub(super) symbol_id: Option<String>,
    pub(super) path: Option<String>,
    pub(super) entry_kind: Option<String>,
    pub(super) reason: String,
    pub(super) source: String,
    pub(super) confidence: Option<f64>,
    pub(super) provenance: Value,
    pub(super) evidence: Value,
    pub(super) properties: Value,
}

impl AssertionRecord {
    fn summary(&self) -> ArchitectureGraphAssertionSummary {
        ArchitectureGraphAssertionSummary {
            id: self.id.clone(),
            action: self.action,
            target_kind: self.target_kind,
            reason: self.reason.clone(),
            source: self.source.clone(),
            provenance: Json(self.provenance.clone()),
            evidence: Json(self.evidence.clone()),
            properties: Json(self.properties.clone()),
        }
    }
}

pub(super) fn node_path_in_scope(
    node: &ArchitectureGraphNode,
    scope: &ResolverScope,
    filter: Option<&ArchitectureGraphFilterInput>,
) -> bool {
    let Some(path) = node.path.as_deref() else {
        return filter.and_then(|filter| filter.path.as_deref()).is_none();
    };
    if !scope.contains_repo_path(path) {
        return false;
    }
    let Some(filter_path) = filter.and_then(|filter| filter.path.as_deref()) else {
        return true;
    };
    path == filter_path
        || path
            .strip_prefix(filter_path)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

pub(super) fn merge_provenance(computed: &Value, asserted: &Value) -> Value {
    serde_json::json!({
        "computed": computed,
        "asserted": asserted,
    })
}

pub(super) fn required_string(row: &Value, key: &str) -> Result<String> {
    optional_string(row, key).ok_or_else(|| anyhow!("missing `{key}` in architecture graph row"))
}

pub(super) fn optional_string(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

pub(super) fn number_field(row: &Value, key: &str) -> Option<f64> {
    row.get(key).and_then(Value::as_f64)
}

pub(super) fn json_column(row: &Value, key: &str) -> Result<Value> {
    match row.get(key) {
        Some(Value::String(raw)) => {
            serde_json::from_str(raw).with_context(|| format!("parsing `{key}` JSON"))
        }
        Some(value) => Ok(value.clone()),
        None => Ok(Value::Null),
    }
}

pub(super) fn sql_text(value: &str) -> String {
    format!("'{}'", esc_pg(value))
}

pub(super) fn graph_count(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}
