use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::host::devql::{RelationalStorage, deterministic_uuid, esc_pg, sql_json_value, sql_now};

use super::types::{
    NAVIGATION_VIEW_DEFINITIONS, NavigationPrimitiveKind, NavigationViewDefinition,
};

pub const NAVIGATION_HASH_VERSION: &str = "navigation-context-v1";
pub const NAVIGATION_MATERIALISATION_FORMAT: &str = "json+markdown";
pub const NAVIGATION_MATERIALISATION_VERSION: &str = "navigation-context-materialisation-v1";

#[derive(Debug, Clone, PartialEq)]
pub struct NavigationPrimitiveFact {
    pub repo_id: String,
    pub primitive_id: String,
    pub primitive_kind: String,
    pub identity_key: String,
    pub label: String,
    pub path: Option<String>,
    pub artefact_id: Option<String>,
    pub symbol_id: Option<String>,
    pub source_kind: String,
    pub confidence: f64,
    pub primitive_hash: String,
    pub properties: Value,
    pub provenance: Value,
    pub last_observed_generation: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NavigationEdgeFact {
    pub repo_id: String,
    pub edge_id: String,
    pub edge_kind: String,
    pub from_primitive_id: String,
    pub to_primitive_id: String,
    pub source_kind: String,
    pub confidence: f64,
    pub edge_hash: String,
    pub properties: Value,
    pub provenance: Value,
    pub last_observed_generation: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NavigationFacts {
    pub primitives: Vec<NavigationPrimitiveFact>,
    pub edges: Vec<NavigationEdgeFact>,
}

impl NavigationFacts {
    pub fn empty() -> Self {
        Self {
            primitives: Vec::new(),
            edges: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationViewAcceptance {
    pub acceptance_id: String,
    pub view_id: String,
    pub previous_accepted_signature: String,
    pub accepted_signature: String,
    pub current_signature: String,
    pub status: String,
    pub source: String,
    pub reason: Option<String>,
    pub materialised_ref: Option<String>,
    pub accepted_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NavigationViewMaterialisation {
    pub materialisation_id: String,
    pub materialised_ref: String,
    pub view_id: String,
    pub view_kind: String,
    pub label: String,
    pub accepted_signature: String,
    pub current_signature: String,
    pub status: String,
    pub materialisation_format: String,
    pub materialisation_version: String,
    pub payload: Value,
    pub rendered_text: String,
    pub primitive_count: i32,
    pub edge_count: i32,
    pub materialised_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExistingViewState {
    accepted_signature: String,
}

pub fn primitive_id(repo_id: &str, kind: NavigationPrimitiveKind, identity: &str) -> String {
    deterministic_uuid(&format!(
        "navigation_context|primitive|{repo_id}|{}|{identity}",
        kind.as_str()
    ))
}

pub fn edge_id(
    repo_id: &str,
    kind: &str,
    from_primitive_id: &str,
    to_primitive_id: &str,
) -> String {
    deterministic_uuid(&format!(
        "navigation_context|edge|{repo_id}|{kind}|{from_primitive_id}|{to_primitive_id}"
    ))
}

pub fn stable_hash(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(NAVIGATION_HASH_VERSION.as_bytes());
    hasher.update(b"\0");
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update(b"\0");
    }
    hex::encode(hasher.finalize())
}

pub async fn replace_navigation_context_current(
    relational: &RelationalStorage,
    repo_id: &str,
    facts: NavigationFacts,
    generation_seq: u64,
    warnings: &[String],
    metrics: Value,
) -> Result<()> {
    let existing_views = load_existing_view_states(relational, repo_id).await?;
    let existing_dependencies = load_existing_view_dependencies(relational, repo_id).await?;
    let view_materialisations =
        build_view_materialisations(&facts.primitives, &existing_views, &existing_dependencies);

    let mut statements = Vec::new();
    statements.push(format!(
        "DELETE FROM navigation_context_view_dependencies_current WHERE repo_id = {};",
        sql_text(repo_id)
    ));
    statements.push(format!(
        "DELETE FROM navigation_context_edges_current WHERE repo_id = {};",
        sql_text(repo_id)
    ));
    statements.push(format!(
        "DELETE FROM navigation_context_primitives_current WHERE repo_id = {};",
        sql_text(repo_id)
    ));

    for primitive in facts.primitives {
        statements.push(insert_primitive_sql(relational, &primitive));
    }
    for edge in facts.edges {
        statements.push(insert_edge_sql(relational, &edge));
    }
    for view in view_materialisations {
        statements.push(insert_view_sql(relational, repo_id, &view));
        for dependency in view.dependencies {
            statements.push(insert_view_dependency_sql(
                relational,
                repo_id,
                &view.view_id,
                &dependency,
            ));
        }
    }
    let generation_text = generation_seq.to_string();
    let metrics_text = metrics.to_string();
    let warnings_text = json!(warnings).to_string();
    let run_signature = stable_hash(&[repo_id, &generation_text, &metrics_text, &warnings_text]);
    statements.push(format!(
        "INSERT INTO navigation_context_views_current (
            repo_id, view_id, view_kind, label, view_query_version, dependency_query_json,
            accepted_signature, current_signature, status, stale_reason_json, materialised_ref,
            last_observed_generation, updated_at
        ) VALUES ({repo_id}, '__run__', 'RUN_STATUS', 'Navigation context run status', '1', {dependency_query},
            {signature}, {signature}, 'fresh', {stale_reason}, NULL, {generation}, {now})
        ON CONFLICT(repo_id, view_id) DO UPDATE SET
            current_signature = excluded.current_signature,
            status = excluded.status,
            stale_reason_json = excluded.stale_reason_json,
            last_observed_generation = excluded.last_observed_generation,
            updated_at = excluded.updated_at;",
        repo_id = sql_text(repo_id),
        dependency_query = sql_json_value(relational, &json!({"kind": "run_status"})),
        signature = sql_text(&run_signature),
        stale_reason = sql_json_value(
            relational,
            &json!({
                "warnings": warnings,
                "metrics": metrics,
            }),
        ),
        generation = generation_seq,
        now = sql_now(relational),
    ));

    relational
        .exec_serialized_batch_transactional(&statements)
        .await
        .context("replacing navigation context current facts")
}

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

pub async fn accept_navigation_context_view(
    relational: &RelationalStorage,
    repo_id: &str,
    view_id: &str,
    expected_current_signature: Option<&str>,
    source: Option<&str>,
    reason: Option<&str>,
    materialised_ref: Option<&str>,
) -> Result<Option<NavigationViewAcceptance>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT view_id, accepted_signature, current_signature, status, materialised_ref \
             FROM navigation_context_views_current \
             WHERE repo_id = {} AND view_id = {}",
            sql_text(repo_id),
            sql_text(view_id)
        ))
        .await
        .context("loading navigation context view before acceptance")?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    let Some(current_signature) = row.get("current_signature").and_then(Value::as_str) else {
        bail!("navigation context view `{view_id}` is missing current_signature");
    };
    let expected_current_signature = normalise_optional_text(expected_current_signature);
    if let Some(expected) = expected_current_signature.as_deref()
        && expected != current_signature
    {
        bail!(
            "navigation context view `{view_id}` current signature changed from `{expected}` to `{current_signature}`"
        );
    }
    let previous_accepted_signature = row
        .get("accepted_signature")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let existing_materialised_ref = row
        .get("materialised_ref")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let source = normalise_optional_text(source).unwrap_or_else(|| "devql_mutation".to_string());
    let reason = normalise_optional_text(reason);
    let materialised_ref_input = normalise_optional_text(materialised_ref);
    let materialised_ref = materialised_ref_input.clone().or(existing_materialised_ref);
    let acceptance_id = format!(
        "navigation-context-acceptance-{}",
        uuid::Uuid::new_v4().simple()
    );
    let accepted_at = chrono::Utc::now().to_rfc3339();
    let acceptance = json!({
        "accepted": true,
        "acceptanceId": &acceptance_id,
        "source": &source,
        "reason": &reason,
        "materialisedRef": &materialised_ref,
        "acceptedAt": &accepted_at,
    });
    let materialised_ref_update = materialised_ref_input
        .as_deref()
        .map(|value| format!("materialised_ref = {}, ", sql_text(value)))
        .unwrap_or_default();
    relational
        .exec_serialized_batch_transactional(&[
            format!(
                "UPDATE navigation_context_views_current SET \
                accepted_signature = current_signature, \
                status = 'fresh', \
                stale_reason_json = {acceptance}, \
                {materialised_ref_update}\
                updated_at = {now} \
             WHERE repo_id = {repo_id} AND view_id = {view_id};",
                acceptance = sql_json_value(relational, &acceptance),
                materialised_ref_update = materialised_ref_update,
                now = sql_now(relational),
                repo_id = sql_text(repo_id),
                view_id = sql_text(view_id),
            ),
            format!(
                "INSERT INTO navigation_context_view_acceptance_history (
                    repo_id, acceptance_id, view_id, previous_accepted_signature,
                    accepted_signature, current_signature, expected_current_signature,
                    source, reason, materialised_ref, accepted_at
                ) VALUES (
                    {repo_id}, {acceptance_id}, {view_id}, {previous_accepted_signature},
                    {accepted_signature}, {current_signature}, {expected_current_signature},
                    {source}, {reason}, {materialised_ref}, {accepted_at}
                );",
                repo_id = sql_text(repo_id),
                acceptance_id = sql_text(&acceptance_id),
                view_id = sql_text(view_id),
                previous_accepted_signature = sql_text(&previous_accepted_signature),
                accepted_signature = sql_text(current_signature),
                current_signature = sql_text(current_signature),
                expected_current_signature = sql_opt_text(expected_current_signature.as_deref()),
                source = sql_text(&source),
                reason = sql_opt_text(reason.as_deref()),
                materialised_ref = sql_opt_text(materialised_ref.as_deref()),
                accepted_at = sql_text(&accepted_at),
            ),
        ])
        .await
        .context("accepting navigation context view signature")?;

    Ok(Some(NavigationViewAcceptance {
        acceptance_id,
        view_id: view_id.to_string(),
        previous_accepted_signature,
        accepted_signature: current_signature.to_string(),
        current_signature: current_signature.to_string(),
        status: "fresh".to_string(),
        source,
        reason,
        materialised_ref,
        accepted_at,
    }))
}

#[derive(Debug, Clone, PartialEq)]
struct ViewDependency {
    primitive_id: String,
    primitive_kind: String,
    primitive_hash: String,
    label: String,
    path: Option<String>,
    source_kind: String,
}

#[derive(Debug, Clone, PartialEq)]
struct ExistingViewDependency {
    primitive_hash: String,
    primitive_kind: String,
    label: Option<String>,
    path: Option<String>,
    source_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DependencyChangeKind {
    Added,
    Removed,
    HashChanged,
}

#[derive(Debug, Clone, PartialEq)]
struct DependencyChange {
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
struct ViewMaterialisation {
    view_id: String,
    view_kind: String,
    label: String,
    view_query_version: String,
    dependency_query: Value,
    accepted_signature: String,
    current_signature: String,
    status: String,
    stale_reason: Value,
    dependencies: Vec<ViewDependency>,
}

fn build_view_materialisations(
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

fn build_view_materialisation(
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

fn changed_dependencies(
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

fn dependency_change_json(change: &DependencyChange) -> Value {
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

async fn load_existing_view_states(
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

async fn load_existing_view_dependencies(
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

async fn load_materialisation_primitives(
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

async fn load_materialisation_edges(
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

fn canonical_primitive_json(row: Value) -> Result<Value> {
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

fn canonical_edge_json(row: Value) -> Result<Value> {
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

fn counts_by_string_field(rows: &[Value], key: &str) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::<String, usize>::new();
    for row in rows {
        if let Some(value) = row.get(key).and_then(Value::as_str) {
            *counts.entry(value.to_string()).or_default() += 1;
        }
    }
    counts
}

fn render_materialised_view(payload: &Value) -> String {
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

fn insert_primitive_sql(
    relational: &RelationalStorage,
    primitive: &NavigationPrimitiveFact,
) -> String {
    format!(
        "INSERT INTO navigation_context_primitives_current (
            repo_id, primitive_id, primitive_kind, identity_key, label, path, artefact_id, symbol_id,
            source_kind, confidence, primitive_hash, hash_version, properties_json,
            provenance_json, last_observed_generation, updated_at
        ) VALUES ({repo_id}, {primitive_id}, {primitive_kind}, {identity_key}, {label}, {path}, {artefact_id}, {symbol_id},
            {source_kind}, {confidence}, {primitive_hash}, {hash_version}, {properties},
            {provenance}, {generation}, {now});",
        repo_id = sql_text(&primitive.repo_id),
        primitive_id = sql_text(&primitive.primitive_id),
        primitive_kind = sql_text(&primitive.primitive_kind),
        identity_key = sql_text(&primitive.identity_key),
        label = sql_text(&primitive.label),
        path = sql_opt_text(primitive.path.as_deref()),
        artefact_id = sql_opt_text(primitive.artefact_id.as_deref()),
        symbol_id = sql_opt_text(primitive.symbol_id.as_deref()),
        source_kind = sql_text(&primitive.source_kind),
        confidence = primitive.confidence,
        primitive_hash = sql_text(&primitive.primitive_hash),
        hash_version = sql_text(NAVIGATION_HASH_VERSION),
        properties = sql_json_value(relational, &primitive.properties),
        provenance = sql_json_value(relational, &primitive.provenance),
        generation = sql_opt_u64(primitive.last_observed_generation),
        now = sql_now(relational),
    )
}

fn insert_edge_sql(relational: &RelationalStorage, edge: &NavigationEdgeFact) -> String {
    format!(
        "INSERT INTO navigation_context_edges_current (
            repo_id, edge_id, edge_kind, from_primitive_id, to_primitive_id, source_kind,
            confidence, edge_hash, hash_version, properties_json, provenance_json,
            last_observed_generation, updated_at
        ) VALUES ({repo_id}, {edge_id}, {edge_kind}, {from_primitive_id}, {to_primitive_id}, {source_kind},
            {confidence}, {edge_hash}, {hash_version}, {properties}, {provenance}, {generation}, {now});",
        repo_id = sql_text(&edge.repo_id),
        edge_id = sql_text(&edge.edge_id),
        edge_kind = sql_text(&edge.edge_kind),
        from_primitive_id = sql_text(&edge.from_primitive_id),
        to_primitive_id = sql_text(&edge.to_primitive_id),
        source_kind = sql_text(&edge.source_kind),
        confidence = edge.confidence,
        edge_hash = sql_text(&edge.edge_hash),
        hash_version = sql_text(NAVIGATION_HASH_VERSION),
        properties = sql_json_value(relational, &edge.properties),
        provenance = sql_json_value(relational, &edge.provenance),
        generation = sql_opt_u64(edge.last_observed_generation),
        now = sql_now(relational),
    )
}

fn insert_view_sql(
    relational: &RelationalStorage,
    repo_id: &str,
    view: &ViewMaterialisation,
) -> String {
    format!(
        "INSERT INTO navigation_context_views_current (
            repo_id, view_id, view_kind, label, view_query_version, dependency_query_json,
            accepted_signature, current_signature, status, stale_reason_json, materialised_ref,
            last_observed_generation, updated_at
        ) VALUES ({repo_id}, {view_id}, {view_kind}, {label}, {query_version}, {dependency_query},
            {accepted_signature}, {current_signature}, {status}, {stale_reason}, NULL, NULL, {now})
        ON CONFLICT(repo_id, view_id) DO UPDATE SET
            view_kind = excluded.view_kind,
            label = excluded.label,
            view_query_version = excluded.view_query_version,
            dependency_query_json = excluded.dependency_query_json,
            current_signature = excluded.current_signature,
            status = excluded.status,
            stale_reason_json = excluded.stale_reason_json,
            updated_at = excluded.updated_at;",
        repo_id = sql_text(repo_id),
        view_id = sql_text(&view.view_id),
        view_kind = sql_text(&view.view_kind),
        label = sql_text(&view.label),
        query_version = sql_text(&view.view_query_version),
        dependency_query = sql_json_value(relational, &view.dependency_query),
        accepted_signature = sql_text(&view.accepted_signature),
        current_signature = sql_text(&view.current_signature),
        status = sql_text(&view.status),
        stale_reason = sql_json_value(relational, &view.stale_reason),
        now = sql_now(relational),
    )
}

fn insert_view_dependency_sql(
    relational: &RelationalStorage,
    repo_id: &str,
    view_id: &str,
    dependency: &ViewDependency,
) -> String {
    format!(
        "INSERT INTO navigation_context_view_dependencies_current (
            repo_id, view_id, primitive_id, primitive_kind, primitive_hash, dependency_role, updated_at
        ) VALUES ({repo_id}, {view_id}, {primitive_id}, {primitive_kind}, {primitive_hash}, 'signature_input', {now});",
        repo_id = sql_text(repo_id),
        view_id = sql_text(view_id),
        primitive_id = sql_text(&dependency.primitive_id),
        primitive_kind = sql_text(&dependency.primitive_kind),
        primitive_hash = sql_text(&dependency.primitive_hash),
        now = sql_now(relational),
    )
}

fn string_field(row: &Value, key: &str) -> Result<String> {
    optional_string_field(row, key).ok_or_else(|| anyhow::anyhow!("missing `{key}`"))
}

fn optional_string_field(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn optional_i64(row: &Value, key: &str) -> Option<i64> {
    row.get(key).and_then(Value::as_i64)
}

fn json_column(row: &Value, key: &str) -> Result<Value> {
    match row.get(key) {
        Some(Value::String(raw)) => {
            serde_json::from_str(raw).with_context(|| format!("parsing `{key}` JSON"))
        }
        Some(value) => Ok(value.clone()),
        None => Ok(Value::Null),
    }
}

fn count_i32(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn short_hash(value: &str) -> String {
    value.chars().take(12).collect()
}

fn sql_list(values: &[&str]) -> String {
    values
        .iter()
        .map(|value| sql_text(value))
        .collect::<Vec<_>>()
        .join(", ")
}

fn sql_text(value: &str) -> String {
    format!("'{}'", esc_pg(value))
}

fn normalise_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn sql_opt_text(value: Option<&str>) -> String {
    value.map(sql_text).unwrap_or_else(|| "NULL".to_string())
}

fn sql_opt_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NULL".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability_packs::navigation_context::schema::navigation_context_sqlite_schema_sql;
    use crate::host::devql::RelationalStorage;
    use tempfile::tempdir;

    fn primitive(kind: NavigationPrimitiveKind, id: &str, hash: &str) -> NavigationPrimitiveFact {
        NavigationPrimitiveFact {
            repo_id: "repo".to_string(),
            primitive_id: id.to_string(),
            primitive_kind: kind.as_str().to_string(),
            identity_key: id.to_string(),
            label: id.to_string(),
            path: Some(format!("src/{id}.rs")),
            artefact_id: None,
            symbol_id: None,
            source_kind: "TEST".to_string(),
            confidence: 1.0,
            primitive_hash: hash.to_string(),
            properties: json!({}),
            provenance: json!({}),
            last_observed_generation: Some(1),
        }
    }

    #[test]
    fn stable_hash_changes_when_input_changes() {
        assert_eq!(stable_hash(&["a", "b"]), stable_hash(&["a", "b"]));
        assert_ne!(stable_hash(&["a", "b"]), stable_hash(&["a", "c"]));
    }

    #[test]
    fn view_materialisation_marks_changed_signature_stale() {
        let primitives = vec![primitive(
            NavigationPrimitiveKind::Symbol,
            "symbol-1",
            "new",
        )];
        let mut existing_views = BTreeMap::new();
        existing_views.insert(
            "architecture_map".to_string(),
            ExistingViewState {
                accepted_signature: "old-signature".to_string(),
            },
        );
        let mut existing_deps = BTreeMap::new();
        existing_deps.insert(
            "architecture_map".to_string(),
            BTreeMap::from([(
                "symbol-1".to_string(),
                ExistingViewDependency {
                    primitive_hash: "old".to_string(),
                    primitive_kind: NavigationPrimitiveKind::Symbol.as_str().to_string(),
                    label: Some("symbol-1".to_string()),
                    path: Some("src/symbol-1.rs".to_string()),
                    source_kind: Some("TEST".to_string()),
                },
            )]),
        );

        let view = build_view_materialisation(
            NAVIGATION_VIEW_DEFINITIONS
                .iter()
                .find(|view| view.view_id == "architecture_map")
                .expect("architecture map view definition"),
            &primitives,
            &existing_views,
            &existing_deps,
        );

        assert_eq!(view.status, "stale");
        assert_eq!(
            view.stale_reason["changedPrimitiveIds"],
            json!(["symbol-1"])
        );
        assert_eq!(view.stale_reason["changeCount"], json!(1));
        assert_eq!(
            view.stale_reason["changedPrimitives"][0],
            json!({
                "primitiveId": "symbol-1",
                "primitiveKind": "SYMBOL",
                "label": "symbol-1",
                "path": "src/symbol-1.rs",
                "sourceKind": "TEST",
                "changeKind": "hash_changed",
                "previousHash": "old",
                "currentHash": "new",
            })
        );
    }

    #[tokio::test]
    async fn accept_navigation_context_view_rebaselines_current_signature() -> Result<()> {
        let temp = tempdir()?;
        let sqlite_path = temp.path().join("navigation.sqlite");
        rusqlite::Connection::open(&sqlite_path)?;
        let relational = RelationalStorage::local_only(sqlite_path);
        relational
            .exec(navigation_context_sqlite_schema_sql())
            .await?;
        relational
            .exec(
                "INSERT INTO navigation_context_views_current (
                    repo_id, view_id, view_kind, label, view_query_version, dependency_query_json,
                    accepted_signature, current_signature, status, stale_reason_json, materialised_ref,
                    last_observed_generation, updated_at
                ) VALUES (
                    'repo', 'architecture_map', 'ARCHITECTURE_MAP', 'Architecture map', '1', '{}',
                    'old-signature', 'new-signature', 'stale', '{}', NULL, 7, '2026-05-03T00:00:00Z'
                );",
            )
            .await?;

        let accepted = accept_navigation_context_view(
            &relational,
            "repo",
            "architecture_map",
            Some("new-signature"),
            Some("test"),
            Some("reviewed"),
            Some("docs/navigation/architecture.md"),
        )
        .await?
        .expect("view should exist");

        assert!(
            accepted
                .acceptance_id
                .starts_with("navigation-context-acceptance-")
        );
        assert_eq!(accepted.previous_accepted_signature, "old-signature");
        assert_eq!(accepted.accepted_signature, "new-signature");
        assert_eq!(
            accepted.materialised_ref.as_deref(),
            Some("docs/navigation/architecture.md")
        );
        let rows = relational
            .query_rows(
                "SELECT accepted_signature, current_signature, status, materialised_ref \
                 FROM navigation_context_views_current \
                 WHERE repo_id = 'repo' AND view_id = 'architecture_map'",
            )
            .await?;
        assert_eq!(rows[0]["accepted_signature"], json!("new-signature"));
        assert_eq!(rows[0]["current_signature"], json!("new-signature"));
        assert_eq!(rows[0]["status"], json!("fresh"));
        assert_eq!(
            rows[0]["materialised_ref"],
            json!("docs/navigation/architecture.md")
        );

        let history = relational
            .query_rows(
                "SELECT view_id, previous_accepted_signature, accepted_signature, \
                        current_signature, expected_current_signature, source, reason, materialised_ref \
                 FROM navigation_context_view_acceptance_history \
                 WHERE repo_id = 'repo' AND view_id = 'architecture_map'",
            )
            .await?;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0]["view_id"], json!("architecture_map"));
        assert_eq!(
            history[0]["previous_accepted_signature"],
            json!("old-signature")
        );
        assert_eq!(history[0]["accepted_signature"], json!("new-signature"));
        assert_eq!(
            history[0]["expected_current_signature"],
            json!("new-signature")
        );
        assert_eq!(history[0]["source"], json!("test"));
        assert_eq!(history[0]["reason"], json!("reviewed"));
        assert_eq!(
            history[0]["materialised_ref"],
            json!("docs/navigation/architecture.md")
        );
        Ok(())
    }

    #[tokio::test]
    async fn materialise_navigation_context_view_stores_snapshot_and_updates_ref() -> Result<()> {
        let temp = tempdir()?;
        let sqlite_path = temp.path().join("navigation.sqlite");
        rusqlite::Connection::open(&sqlite_path)?;
        let relational = RelationalStorage::local_only(sqlite_path);
        relational
            .exec(navigation_context_sqlite_schema_sql())
            .await?;
        relational
            .exec(
                "INSERT INTO navigation_context_views_current (
                    repo_id, view_id, view_kind, label, view_query_version, dependency_query_json,
                    accepted_signature, current_signature, status, stale_reason_json, materialised_ref,
                    last_observed_generation, updated_at
                ) VALUES (
                    'repo', 'architecture_map', 'ARCHITECTURE_MAP', 'Architecture map', '1',
                    '{\"primitiveKinds\":[\"SYMBOL\"]}', 'old-signature', 'new-signature',
                    'stale', '{\"reason\":\"dependency_signature_changed\"}', NULL, 7,
                    '2026-05-03T00:00:00Z'
                );
                INSERT INTO navigation_context_primitives_current (
                    repo_id, primitive_id, primitive_kind, identity_key, label, path, artefact_id,
                    symbol_id, source_kind, confidence, primitive_hash, hash_version,
                    properties_json, provenance_json, last_observed_generation, updated_at
                ) VALUES (
                    'repo', 'symbol-1', 'SYMBOL', 'symbol:render', 'render', 'src/render.rs',
                    'artefact-1', 'symbol-id-1', 'TEST', 1.0, 'primitive-hash',
                    'navigation-context-v1', '{\"signature\":\"fn render()\"}',
                    '{\"source\":\"test\"}', 7, '2026-05-03T00:00:00Z'
                );
                INSERT INTO navigation_context_view_dependencies_current (
                    repo_id, view_id, primitive_id, primitive_kind, primitive_hash, dependency_role, updated_at
                ) VALUES (
                    'repo', 'architecture_map', 'symbol-1', 'SYMBOL', 'primitive-hash',
                    'signature_input', '2026-05-03T00:00:00Z'
                );",
            )
            .await?;

        let materialised = materialise_navigation_context_view(
            &relational,
            "repo",
            "architecture_map",
            Some("new-signature"),
        )
        .await?
        .expect("view should exist");

        assert_eq!(materialised.view_id, "architecture_map");
        assert_eq!(materialised.current_signature, "new-signature");
        assert_eq!(materialised.primitive_count, 1);
        assert_eq!(materialised.edge_count, 0);
        assert!(
            materialised
                .materialised_ref
                .starts_with("navigation-context://materialisations/")
        );
        assert_eq!(
            materialised.payload["primitives"][0]["properties"],
            json!({"signature": "fn render()"})
        );
        assert!(materialised.rendered_text.contains("# Architecture map"));
        assert!(
            materialised
                .rendered_text
                .contains("SYMBOL: render (src/render.rs)")
        );

        let rows = relational
            .query_rows(
                "SELECT materialised_ref FROM navigation_context_views_current \
                 WHERE repo_id = 'repo' AND view_id = 'architecture_map'",
            )
            .await?;
        assert_eq!(
            rows[0]["materialised_ref"],
            json!(materialised.materialised_ref)
        );

        let snapshots = relational
            .query_rows(
                "SELECT materialisation_id, materialised_ref, current_signature, primitive_count, edge_count, payload_json \
                 FROM navigation_context_materialised_views \
                 WHERE repo_id = 'repo' AND view_id = 'architecture_map'",
            )
            .await?;
        assert_eq!(snapshots.len(), 1);
        assert_eq!(
            snapshots[0]["materialisation_id"],
            json!(materialised.materialisation_id)
        );
        assert_eq!(
            snapshots[0]["materialised_ref"],
            json!(materialised.materialised_ref)
        );
        assert_eq!(snapshots[0]["current_signature"], json!("new-signature"));
        assert_eq!(snapshots[0]["primitive_count"], json!(1));
        assert_eq!(snapshots[0]["edge_count"], json!(0));
        let payload: Value = serde_json::from_str(
            snapshots[0]["payload_json"]
                .as_str()
                .expect("payload JSON should be stored as text"),
        )?;
        assert_eq!(payload["view"]["viewId"], json!("architecture_map"));
        Ok(())
    }
}
