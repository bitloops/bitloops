use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::host::devql::{RelationalStorage, deterministic_uuid, esc_pg, sql_json_value, sql_now};

use super::types::{
    NAVIGATION_VIEW_DEFINITIONS, NavigationPrimitiveKind, NavigationViewDefinition,
};

mod materialisation;

pub use materialisation::materialise_navigation_context_view;
#[cfg(test)]
use materialisation::{ExistingViewDependency, build_view_materialisation};
use materialisation::{
    ViewDependency, ViewMaterialisation, build_view_materialisations,
    load_existing_view_dependencies, load_existing_view_states,
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
mod tests;
