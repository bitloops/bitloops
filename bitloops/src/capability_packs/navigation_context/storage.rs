use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::host::devql::{RelationalStorage, deterministic_uuid, esc_pg, sql_json_value, sql_now};

use super::types::{
    NAVIGATION_VIEW_DEFINITIONS, NavigationPrimitiveKind, NavigationViewDefinition,
};

pub const NAVIGATION_HASH_VERSION: &str = "navigation-context-v1";

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

#[derive(Debug, Clone, PartialEq)]
struct ViewDependency {
    primitive_id: String,
    primitive_kind: String,
    primitive_hash: String,
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
    existing_dependencies: &BTreeMap<String, BTreeMap<String, String>>,
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
    existing_dependencies: &BTreeMap<String, BTreeMap<String, String>>,
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
    let changed_primitives =
        changed_dependencies(existing_dependencies.get(definition.view_id), &dependencies);
    let stale_reason = if status == "fresh" {
        json!({})
    } else {
        json!({
            "reason": "dependency_signature_changed",
            "changedPrimitiveIds": changed_primitives,
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
    previous: Option<&BTreeMap<String, String>>,
    current: &[ViewDependency],
) -> Vec<String> {
    let Some(previous) = previous else {
        return Vec::new();
    };
    let current_by_id = current
        .iter()
        .map(|dependency| {
            (
                dependency.primitive_id.clone(),
                dependency.primitive_hash.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut changed = current_by_id
        .iter()
        .filter_map(|(id, hash)| {
            (previous
                .get(id)
                .map_or(true, |previous_hash| previous_hash != hash))
            .then(|| id.clone())
        })
        .collect::<BTreeSet<_>>();
    for id in previous.keys() {
        if !current_by_id.contains_key(id) {
            changed.insert(id.clone());
        }
    }
    changed.into_iter().collect()
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
) -> Result<BTreeMap<String, BTreeMap<String, String>>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT view_id, primitive_id, primitive_hash FROM navigation_context_view_dependencies_current WHERE repo_id = {}",
            sql_text(repo_id)
        ))
        .await
        .unwrap_or_default();
    let mut out = BTreeMap::<String, BTreeMap<String, String>>::new();
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
        out.entry(view_id.to_string())
            .or_default()
            .insert(primitive_id.to_string(), primitive_hash.to_string());
    }
    Ok(out)
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

fn sql_text(value: &str) -> String {
    format!("'{}'", esc_pg(value))
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

    fn primitive(kind: NavigationPrimitiveKind, id: &str, hash: &str) -> NavigationPrimitiveFact {
        NavigationPrimitiveFact {
            repo_id: "repo".to_string(),
            primitive_id: id.to_string(),
            primitive_kind: kind.as_str().to_string(),
            identity_key: id.to_string(),
            label: id.to_string(),
            path: None,
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
            BTreeMap::from([("symbol-1".to_string(), "old".to_string())]),
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
    }
}
