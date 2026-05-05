use super::*;

pub async fn materialise_navigation_context_view(
    relational: &RelationalStorage,
    repo_id: &str,
    view_id: &str,
    expected_current_signature: Option<&str>,
) -> Result<Option<NavigationViewMaterialisation>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT view_id, view_kind, label, view_query_version, dependency_query_json, \
                    accepted_signature, current_signature, status, stale_reason_json, materialised_ref, \
                    last_observed_generation, updated_at \
             FROM navigation_context_views_current \
             WHERE repo_id = {} AND view_id = {}",
            sql_text(repo_id),
            sql_text(view_id)
        ))
        .await
        .context("loading navigation context view before materialisation")?;
    let Some(view) = rows.into_iter().next() else {
        return Ok(None);
    };
    let current_signature = string_field(&view, "current_signature")?;
    let expected_current_signature = normalise_optional_text(expected_current_signature);
    if let Some(expected) = expected_current_signature.as_deref()
        && expected != current_signature
    {
        bail!(
            "navigation context view `{view_id}` current signature changed from `{expected}` to `{current_signature}`"
        );
    }

    let primitives = load_materialisation_primitives(relational, repo_id, view_id).await?;
    let primitive_ids = primitives
        .iter()
        .filter_map(|primitive| {
            primitive
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();
    let edges = load_materialisation_edges(relational, repo_id, &primitive_ids).await?;
    let primitive_counts_by_kind = counts_by_string_field(&primitives, "kind");
    let edge_counts_by_kind = counts_by_string_field(&edges, "kind");
    let materialisation_id = deterministic_uuid(&format!(
        "navigation_context|materialisation|{repo_id}|{view_id}|{current_signature}|{NAVIGATION_MATERIALISATION_FORMAT}|{NAVIGATION_MATERIALISATION_VERSION}"
    ));
    let materialised_ref = format!("navigation-context://materialisations/{materialisation_id}");
    let materialised_at = chrono::Utc::now().to_rfc3339();
    let dependency_query = json_column(&view, "dependency_query_json")?;
    let stale_reason = json_column(&view, "stale_reason_json")?;
    let view_payload = json!({
        "viewId": string_field(&view, "view_id")?,
        "viewKind": string_field(&view, "view_kind")?,
        "label": string_field(&view, "label")?,
        "viewQueryVersion": string_field(&view, "view_query_version")?,
        "dependencyQuery": &dependency_query,
        "acceptedSignature": string_field(&view, "accepted_signature")?,
        "currentSignature": &current_signature,
        "status": string_field(&view, "status")?,
        "staleReason": &stale_reason,
        "lastObservedGeneration": optional_i64(&view, "last_observed_generation"),
        "updatedAt": string_field(&view, "updated_at")?,
    });
    let payload = json!({
        "schemaVersion": NAVIGATION_MATERIALISATION_VERSION,
        "materialisationId": &materialisation_id,
        "materialisedRef": &materialised_ref,
        "repoId": repo_id,
        "materialisedAt": &materialised_at,
        "materialisationFormat": NAVIGATION_MATERIALISATION_FORMAT,
        "view": &view_payload,
        "summary": {
            "primitiveCount": primitives.len(),
            "edgeCount": edges.len(),
            "primitiveCountsByKind": primitive_counts_by_kind,
            "edgeCountsByKind": edge_counts_by_kind,
        },
        "primitives": &primitives,
        "edges": &edges,
    });
    let rendered_text = render_materialised_view(&payload);
    let primitive_count = count_i32(primitives.len());
    let edge_count = count_i32(edges.len());

    relational
        .exec_serialized_batch_transactional(&[
            format!(
                "INSERT INTO navigation_context_materialised_views (
                    repo_id, materialisation_id, materialised_ref, view_id, view_kind, label,
                    view_query_version, accepted_signature, current_signature, status,
                    materialisation_format, materialisation_version, dependency_query_json,
                    payload_json, rendered_text, primitive_count, edge_count, materialised_at
                ) VALUES (
                    {repo_id}, {materialisation_id}, {materialised_ref}, {view_id}, {view_kind}, {label},
                    {view_query_version}, {accepted_signature}, {current_signature}, {status},
                    {materialisation_format}, {materialisation_version}, {dependency_query},
                    {payload}, {rendered_text}, {primitive_count}, {edge_count}, {materialised_at}
                )
                ON CONFLICT(repo_id, materialisation_id) DO UPDATE SET
                    materialised_ref = excluded.materialised_ref,
                    view_kind = excluded.view_kind,
                    label = excluded.label,
                    view_query_version = excluded.view_query_version,
                    accepted_signature = excluded.accepted_signature,
                    current_signature = excluded.current_signature,
                    status = excluded.status,
                    dependency_query_json = excluded.dependency_query_json,
                    payload_json = excluded.payload_json,
                    rendered_text = excluded.rendered_text,
                    primitive_count = excluded.primitive_count,
                    edge_count = excluded.edge_count,
                    materialised_at = excluded.materialised_at;",
                repo_id = sql_text(repo_id),
                materialisation_id = sql_text(&materialisation_id),
                materialised_ref = sql_text(&materialised_ref),
                view_id = sql_text(view_id),
                view_kind = sql_text(string_field(&view, "view_kind")?.as_str()),
                label = sql_text(string_field(&view, "label")?.as_str()),
                view_query_version = sql_text(string_field(&view, "view_query_version")?.as_str()),
                accepted_signature = sql_text(string_field(&view, "accepted_signature")?.as_str()),
                current_signature = sql_text(&current_signature),
                status = sql_text(string_field(&view, "status")?.as_str()),
                materialisation_format = sql_text(NAVIGATION_MATERIALISATION_FORMAT),
                materialisation_version = sql_text(NAVIGATION_MATERIALISATION_VERSION),
                dependency_query = sql_json_value(relational, &dependency_query),
                payload = sql_json_value(relational, &payload),
                rendered_text = sql_text(&rendered_text),
                primitive_count = primitive_count,
                edge_count = edge_count,
                materialised_at = sql_text(&materialised_at),
            ),
            format!(
                "UPDATE navigation_context_views_current \
                 SET materialised_ref = {materialised_ref}, updated_at = {now} \
                 WHERE repo_id = {repo_id} AND view_id = {view_id} AND current_signature = {current_signature};",
                materialised_ref = sql_text(&materialised_ref),
                now = sql_now(relational),
                repo_id = sql_text(repo_id),
                view_id = sql_text(view_id),
                current_signature = sql_text(&current_signature),
            ),
        ])
        .await
        .context("materialising navigation context view")?;

    Ok(Some(NavigationViewMaterialisation {
        materialisation_id,
        materialised_ref,
        view_id: view_id.to_string(),
        view_kind: string_field(&view, "view_kind")?,
        label: string_field(&view, "label")?,
        accepted_signature: string_field(&view, "accepted_signature")?,
        current_signature,
        status: string_field(&view, "status")?,
        materialisation_format: NAVIGATION_MATERIALISATION_FORMAT.to_string(),
        materialisation_version: NAVIGATION_MATERIALISATION_VERSION.to_string(),
        payload,
        rendered_text,
        primitive_count,
        edge_count,
        materialised_at,
    }))
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ViewDependency {
    pub(super) primitive_id: String,
    pub(super) primitive_kind: String,
    pub(super) primitive_hash: String,
    pub(super) label: String,
    pub(super) path: Option<String>,
    pub(super) source_kind: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ExistingViewDependency {
    pub(super) primitive_hash: String,
    pub(super) primitive_kind: String,
    pub(super) label: Option<String>,
    pub(super) path: Option<String>,
    pub(super) source_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DependencyChangeKind {
    Added,
    Removed,
    HashChanged,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct DependencyChange {
    primitive_id: String,
    primitive_kind: String,
    label: Option<String>,
    path: Option<String>,
    source_kind: Option<String>,
    change_kind: DependencyChangeKind,
    previous_hash: Option<String>,
    current_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ViewMaterialisation {
    pub(super) view_id: String,
    pub(super) view_kind: String,
    pub(super) label: String,
    pub(super) view_query_version: String,
    pub(super) dependency_query: Value,
    pub(super) accepted_signature: String,
    pub(super) current_signature: String,
    pub(super) status: String,
    pub(super) stale_reason: Value,
    pub(super) dependencies: Vec<ViewDependency>,
}

pub(super) fn build_view_materialisations(
    primitives: &[NavigationPrimitiveFact],
    existing_views: &BTreeMap<String, ExistingViewState>,
    existing_dependencies: &BTreeMap<String, BTreeMap<String, ExistingViewDependency>>,
) -> Vec<ViewMaterialisation> {
    NAVIGATION_VIEW_DEFINITIONS
        .iter()
        .map(|definition| {
            build_view_materialisation(
                definition,
                primitives,
                existing_views,
                existing_dependencies,
            )
        })
        .collect()
}

pub(super) fn build_view_materialisation(
    definition: &NavigationViewDefinition,
    primitives: &[NavigationPrimitiveFact],
    existing_views: &BTreeMap<String, ExistingViewState>,
    existing_dependencies: &BTreeMap<String, BTreeMap<String, ExistingViewDependency>>,
) -> ViewMaterialisation {
    let allowed_kinds = definition
        .primitive_kinds
        .iter()
        .map(|kind| kind.as_str())
        .collect::<BTreeSet<_>>();
    let mut dependencies = primitives
        .iter()
        .filter(|primitive| allowed_kinds.contains(primitive.primitive_kind.as_str()))
        .map(|primitive| ViewDependency {
            primitive_id: primitive.primitive_id.clone(),
            primitive_kind: primitive.primitive_kind.clone(),
            primitive_hash: primitive.primitive_hash.clone(),
            label: primitive.label.clone(),
            path: primitive.path.clone(),
            source_kind: primitive.source_kind.clone(),
        })
        .collect::<Vec<_>>();
    dependencies.sort_by(|left, right| left.primitive_id.cmp(&right.primitive_id));
    let dependency_signature_parts = dependencies
        .iter()
        .flat_map(|dependency| {
            [
                dependency.primitive_id.as_str(),
                dependency.primitive_kind.as_str(),
                dependency.primitive_hash.as_str(),
            ]
        })
        .collect::<Vec<_>>();
    let mut signature_parts = vec![definition.view_id, definition.query_version];
    signature_parts.extend(dependency_signature_parts);
    let current_signature = stable_hash(&signature_parts);
    let accepted_signature = existing_views
        .get(definition.view_id)
        .map(|state| state.accepted_signature.clone())
        .unwrap_or_else(|| current_signature.clone());
    let status = if accepted_signature == current_signature {
        "fresh"
    } else {
        "stale"
    }
    .to_string();
    let dependency_changes =
        changed_dependencies(existing_dependencies.get(definition.view_id), &dependencies);
    let changed_primitive_ids = dependency_changes
        .iter()
        .map(|change| change.primitive_id.clone())
        .collect::<Vec<_>>();
    let stale_reason = if status == "fresh" {
        json!({})
    } else {
        json!({
            "reason": "dependency_signature_changed",
            "changedPrimitiveIds": changed_primitive_ids,
            "changedPrimitives": dependency_changes
                .iter()
                .map(dependency_change_json)
                .collect::<Vec<_>>(),
            "changeCount": dependency_changes.len(),
        })
    };

    ViewMaterialisation {
        view_id: definition.view_id.to_string(),
        view_kind: definition.view_kind.to_string(),
        label: definition.label.to_string(),
        view_query_version: definition.query_version.to_string(),
        dependency_query: json!({
            "primitiveKinds": definition
                .primitive_kinds
                .iter()
                .map(|kind| kind.as_str())
                .collect::<Vec<_>>(),
        }),
        accepted_signature,
        current_signature,
        status,
        stale_reason,
        dependencies,
    }
}

pub(super) fn changed_dependencies(
    previous: Option<&BTreeMap<String, ExistingViewDependency>>,
    current: &[ViewDependency],
) -> Vec<DependencyChange> {
    let Some(previous) = previous else {
        return Vec::new();
    };
    let current_by_id = current
        .iter()
        .map(|dependency| (dependency.primitive_id.clone(), dependency))
        .collect::<BTreeMap<_, _>>();

    let mut changed = current_by_id
        .iter()
        .filter_map(|(id, current_dependency)| match previous.get(id) {
            None => Some(DependencyChange {
                primitive_id: id.clone(),
                primitive_kind: current_dependency.primitive_kind.clone(),
                label: Some(current_dependency.label.clone()),
                path: current_dependency.path.clone(),
                source_kind: Some(current_dependency.source_kind.clone()),
                change_kind: DependencyChangeKind::Added,
                previous_hash: None,
                current_hash: Some(current_dependency.primitive_hash.clone()),
            }),
            Some(previous_dependency)
                if previous_dependency.primitive_hash != current_dependency.primitive_hash =>
            {
                Some(DependencyChange {
                    primitive_id: id.clone(),
                    primitive_kind: current_dependency.primitive_kind.clone(),
                    label: Some(current_dependency.label.clone()),
                    path: current_dependency
                        .path
                        .clone()
                        .or_else(|| previous_dependency.path.clone()),
                    source_kind: Some(current_dependency.source_kind.clone())
                        .or_else(|| previous_dependency.source_kind.clone()),
                    change_kind: DependencyChangeKind::HashChanged,
                    previous_hash: Some(previous_dependency.primitive_hash.clone()),
                    current_hash: Some(current_dependency.primitive_hash.clone()),
                })
            }
            Some(_) => None,
        })
        .collect::<Vec<_>>();
    for (id, previous_dependency) in previous {
        if !current_by_id.contains_key(id) {
            changed.push(DependencyChange {
                primitive_id: id.clone(),
                primitive_kind: previous_dependency.primitive_kind.clone(),
                label: previous_dependency.label.clone(),
                path: previous_dependency.path.clone(),
                source_kind: previous_dependency.source_kind.clone(),
                change_kind: DependencyChangeKind::Removed,
                previous_hash: Some(previous_dependency.primitive_hash.clone()),
                current_hash: None,
            });
        }
    }
    changed.sort_by(|left, right| left.primitive_id.cmp(&right.primitive_id));
    changed
}

pub(super) fn dependency_change_json(change: &DependencyChange) -> Value {
    let change_kind = match change.change_kind {
        DependencyChangeKind::Added => "added",
        DependencyChangeKind::Removed => "removed",
        DependencyChangeKind::HashChanged => "hash_changed",
    };
    json!({
        "primitiveId": &change.primitive_id,
        "primitiveKind": &change.primitive_kind,
        "label": &change.label,
        "path": &change.path,
        "sourceKind": &change.source_kind,
        "changeKind": change_kind,
        "previousHash": &change.previous_hash,
        "currentHash": &change.current_hash,
    })
}

pub(super) async fn load_existing_view_states(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<BTreeMap<String, ExistingViewState>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT view_id, accepted_signature FROM navigation_context_views_current WHERE repo_id = {}",
            sql_text(repo_id)
        ))
        .await
        .unwrap_or_default();
    let mut out = BTreeMap::new();
    for row in rows {
        let Some(view_id) = row.get("view_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(accepted_signature) = row.get("accepted_signature").and_then(Value::as_str) else {
            continue;
        };
        out.insert(
            view_id.to_string(),
            ExistingViewState {
                accepted_signature: accepted_signature.to_string(),
            },
        );
    }
    Ok(out)
}

pub(super) async fn load_existing_view_dependencies(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<BTreeMap<String, BTreeMap<String, ExistingViewDependency>>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT d.view_id, d.primitive_id, d.primitive_kind, d.primitive_hash, \
                    p.label, p.path, p.source_kind \
             FROM navigation_context_view_dependencies_current d \
             LEFT JOIN navigation_context_primitives_current p \
                ON p.repo_id = d.repo_id AND p.primitive_id = d.primitive_id \
             WHERE d.repo_id = {}",
            sql_text(repo_id)
        ))
        .await
        .unwrap_or_default();
    let mut out = BTreeMap::<String, BTreeMap<String, ExistingViewDependency>>::new();
    for row in rows {
        let Some(view_id) = row.get("view_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(primitive_id) = row.get("primitive_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(primitive_hash) = row.get("primitive_hash").and_then(Value::as_str) else {
            continue;
        };
        let Some(primitive_kind) = row.get("primitive_kind").and_then(Value::as_str) else {
            continue;
        };
        out.entry(view_id.to_string()).or_default().insert(
            primitive_id.to_string(),
            ExistingViewDependency {
                primitive_hash: primitive_hash.to_string(),
                primitive_kind: primitive_kind.to_string(),
                label: row
                    .get("label")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                path: row
                    .get("path")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                source_kind: row
                    .get("source_kind")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
            },
        );
    }
    Ok(out)
}

pub(super) async fn load_materialisation_primitives(
    relational: &RelationalStorage,
    repo_id: &str,
    view_id: &str,
) -> Result<Vec<Value>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT p.primitive_id, p.primitive_kind, p.identity_key, p.label, p.path, p.artefact_id, p.symbol_id, \
                    p.source_kind, p.confidence, p.primitive_hash, p.hash_version, p.properties_json, \
                    p.provenance_json, p.last_observed_generation, p.updated_at, d.dependency_role \
             FROM navigation_context_view_dependencies_current d \
             JOIN navigation_context_primitives_current p \
                ON p.repo_id = d.repo_id AND p.primitive_id = d.primitive_id \
             WHERE d.repo_id = {} AND d.view_id = {} \
             ORDER BY p.primitive_kind, COALESCE(p.path, ''), p.label, p.primitive_id",
            sql_text(repo_id),
            sql_text(view_id)
        ))
        .await
        .context("loading navigation context primitives for materialisation")?;
    rows.into_iter().map(canonical_primitive_json).collect()
}

pub(super) async fn load_materialisation_edges(
    relational: &RelationalStorage,
    repo_id: &str,
    primitive_ids: &[String],
) -> Result<Vec<Value>> {
    if primitive_ids.is_empty() {
        return Ok(Vec::new());
    }
    let primitive_id_refs = primitive_ids.iter().map(String::as_str).collect::<Vec<_>>();
    let id_list = sql_list(&primitive_id_refs);
    let rows = relational
        .query_rows(&format!(
            "SELECT edge_id, edge_kind, from_primitive_id, to_primitive_id, source_kind, confidence, \
                    edge_hash, hash_version, properties_json, provenance_json, last_observed_generation, updated_at \
             FROM navigation_context_edges_current \
             WHERE repo_id = {} AND from_primitive_id IN ({id_list}) AND to_primitive_id IN ({id_list}) \
             ORDER BY edge_kind, from_primitive_id, to_primitive_id, edge_id",
            sql_text(repo_id),
        ))
        .await
        .context("loading navigation context edges for materialisation")?;
    rows.into_iter().map(canonical_edge_json).collect()
}

pub(super) fn canonical_primitive_json(row: Value) -> Result<Value> {
    Ok(json!({
        "id": string_field(&row, "primitive_id")?,
        "kind": string_field(&row, "primitive_kind")?,
        "identityKey": string_field(&row, "identity_key")?,
        "label": string_field(&row, "label")?,
        "path": optional_string_field(&row, "path"),
        "artefactId": optional_string_field(&row, "artefact_id"),
        "symbolId": optional_string_field(&row, "symbol_id"),
        "sourceKind": string_field(&row, "source_kind")?,
        "confidence": row.get("confidence").and_then(Value::as_f64).unwrap_or(0.0),
        "primitiveHash": string_field(&row, "primitive_hash")?,
        "hashVersion": string_field(&row, "hash_version")?,
        "properties": json_column(&row, "properties_json")?,
        "provenance": json_column(&row, "provenance_json")?,
        "dependencyRole": string_field(&row, "dependency_role")?,
        "lastObservedGeneration": optional_i64(&row, "last_observed_generation"),
        "updatedAt": string_field(&row, "updated_at")?,
    }))
}

pub(super) fn canonical_edge_json(row: Value) -> Result<Value> {
    Ok(json!({
        "id": string_field(&row, "edge_id")?,
        "kind": string_field(&row, "edge_kind")?,
        "fromPrimitiveId": string_field(&row, "from_primitive_id")?,
        "toPrimitiveId": string_field(&row, "to_primitive_id")?,
        "sourceKind": string_field(&row, "source_kind")?,
        "confidence": row.get("confidence").and_then(Value::as_f64).unwrap_or(0.0),
        "edgeHash": string_field(&row, "edge_hash")?,
        "hashVersion": string_field(&row, "hash_version")?,
        "properties": json_column(&row, "properties_json")?,
        "provenance": json_column(&row, "provenance_json")?,
        "lastObservedGeneration": optional_i64(&row, "last_observed_generation"),
        "updatedAt": string_field(&row, "updated_at")?,
    }))
}

pub(super) fn counts_by_string_field(rows: &[Value], key: &str) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::<String, usize>::new();
    for row in rows {
        if let Some(value) = row.get(key).and_then(Value::as_str) {
            *counts.entry(value.to_string()).or_default() += 1;
        }
    }
    counts
}

pub(super) fn render_materialised_view(payload: &Value) -> String {
    let view = payload.get("view").unwrap_or(&Value::Null);
    let label = view.get("label").and_then(Value::as_str).unwrap_or("View");
    let view_id = view
        .get("viewId")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let status = view
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let current_signature = view
        .get("currentSignature")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let accepted_signature = view
        .get("acceptedSignature")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let summary = payload.get("summary").unwrap_or(&Value::Null);
    let primitive_count = summary
        .get("primitiveCount")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let edge_count = summary
        .get("edgeCount")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let mut lines = vec![
        format!("# {label}"),
        String::new(),
        format!("- view_id: {view_id}"),
        format!("- status: {status}"),
        format!("- current_signature: {current_signature}"),
        format!("- accepted_signature: {accepted_signature}"),
        format!("- primitives: {primitive_count}"),
        format!("- edges: {edge_count}"),
        String::new(),
        "## Primitive Counts".to_string(),
    ];
    if let Some(counts) = summary
        .get("primitiveCountsByKind")
        .and_then(Value::as_object)
    {
        for (kind, count) in counts {
            lines.push(format!("- {kind}: {}", count.as_u64().unwrap_or_default()));
        }
    }
    lines.push(String::new());
    lines.push("## Primitives".to_string());
    if let Some(primitives) = payload.get("primitives").and_then(Value::as_array) {
        for primitive in primitives {
            let kind = primitive
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("UNKNOWN");
            let label = primitive
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("<unlabelled>");
            let hash = primitive
                .get("primitiveHash")
                .and_then(Value::as_str)
                .map(short_hash)
                .unwrap_or_else(|| "<none>".to_string());
            match primitive.get("path").and_then(Value::as_str) {
                Some(path) => lines.push(format!("- {kind}: {label} ({path}) hash={hash}")),
                None => lines.push(format!("- {kind}: {label} hash={hash}")),
            }
        }
    }
    lines.push(String::new());
    lines.push("## Edges".to_string());
    if let Some(edges) = payload.get("edges").and_then(Value::as_array) {
        for edge in edges {
            let kind = edge
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("UNKNOWN");
            let from = edge
                .get("fromPrimitiveId")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            let to = edge
                .get("toPrimitiveId")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            lines.push(format!("- {kind}: {from} -> {to}"));
        }
    }
    lines.join("\n")
}
