use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use super::DevqlGraphqlContext;
use crate::graphql::ResolverScope;
use crate::graphql::types::navigation_context::json_scalar;
use crate::graphql::types::{
    NavigationContextFilterInput, NavigationContextSnapshot, NavigationContextView,
    NavigationContextViewAcceptance, NavigationContextViewDependency, NavigationContextViewStatus,
    NavigationEdge, NavigationPrimitive, NavigationPrimitiveKind,
};
use crate::host::devql::esc_pg;

impl DevqlGraphqlContext {
    pub(crate) async fn list_navigation_context(
        &self,
        scope: &ResolverScope,
        filter: Option<&NavigationContextFilterInput>,
        first: Option<usize>,
        after: Option<&str>,
    ) -> Result<NavigationContextSnapshot> {
        if scope.temporal_scope().is_some() {
            return Err(anyhow!(
                "`navigationContext` does not support historical or temporary `asOf(...)` scopes"
            ));
        }

        let repo_id = self.repo_id_for_scope(scope)?;
        let mut views = load_navigation_views(self, &repo_id, filter).await?;
        let view_ids = views
            .iter()
            .map(|view| view.view_id.as_str())
            .collect::<Vec<_>>();
        let mut dependencies = load_navigation_dependencies(self, &repo_id, &view_ids).await?;
        let mut acceptance_history =
            load_navigation_acceptance_history(self, &repo_id, &view_ids).await?;

        if let Some(primitive_kind) = filter.and_then(|filter| filter.primitive_kind) {
            views.retain(|view| {
                dependencies.get(&view.view_id).is_some_and(|dependencies| {
                    dependencies
                        .iter()
                        .any(|dependency| dependency.primitive_kind == primitive_kind)
                })
            });
        }
        let view_ids = views
            .iter()
            .map(|view| view.view_id.clone())
            .collect::<BTreeSet<_>>();
        dependencies.retain(|view_id, _| view_ids.contains(view_id));
        for view in &mut views {
            view.dependencies = dependencies.remove(&view.view_id).unwrap_or_default();
            view.acceptance_history = acceptance_history.remove(&view.view_id).unwrap_or_default();
        }

        let view_filter_active = filter.is_some_and(|filter| {
            filter
                .view_id
                .as_deref()
                .map(str::trim)
                .is_some_and(|view_id| !view_id.is_empty())
                || filter.view_status.is_some()
                || filter.primitive_kind.is_some()
        });
        let view_scoped_primitive_ids = if view_filter_active || !views.is_empty() {
            Some(
                views
                    .iter()
                    .flat_map(|view| view.dependencies.iter())
                    .map(|dependency| dependency.primitive_id.clone())
                    .collect::<BTreeSet<_>>(),
            )
        } else {
            None
        };
        let mut primitives =
            load_navigation_primitives(self, &repo_id, filter, view_scoped_primitive_ids.as_ref())
                .await?;
        primitives.retain(|primitive| primitive_path_in_scope(primitive, scope, filter));
        primitives.sort_by(|left, right| left.id.cmp(&right.id));
        if let Some(after) = after {
            primitives = primitives
                .into_iter()
                .skip_while(|primitive| primitive.id != after)
                .skip(1)
                .collect();
        }
        let total_primitives = primitives.len();
        if let Some(limit) = first {
            primitives.truncate(limit);
        }
        let primitive_ids = primitives
            .iter()
            .map(|primitive| primitive.id.clone())
            .collect::<BTreeSet<_>>();
        let mut edges = load_navigation_edges(self, &repo_id, filter).await?;
        edges.retain(|edge| {
            primitive_ids.contains(&edge.from_primitive_id)
                && primitive_ids.contains(&edge.to_primitive_id)
        });
        edges.sort_by(|left, right| left.id.cmp(&right.id));
        let total_edges = edges.len();
        let total_views = views.len();

        Ok(NavigationContextSnapshot::new(
            views,
            primitives,
            edges,
            total_views,
            total_primitives,
            total_edges,
        ))
    }
}

async fn load_navigation_views(
    context: &DevqlGraphqlContext,
    repo_id: &str,
    filter: Option<&NavigationContextFilterInput>,
) -> Result<Vec<NavigationContextView>> {
    let mut clauses = vec![format!("repo_id = {}", sql_text(repo_id))];
    if let Some(view_id) = filter
        .and_then(|filter| filter.view_id.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        clauses.push(format!("view_id = {}", sql_text(view_id)));
    }
    if let Some(status) = filter.and_then(|filter| filter.view_status) {
        clauses.push(format!("status = {}", sql_text(status.as_db())));
    }
    let sql = format!(
        "SELECT view_id, view_kind, label, view_query_version, dependency_query_json, \
                accepted_signature, current_signature, status, stale_reason_json, materialised_ref, \
                last_observed_generation, updated_at \
         FROM navigation_context_views_current WHERE {} ORDER BY view_id",
        clauses.join(" AND ")
    );
    context
        .query_devql_sqlite_rows(&sql)
        .await?
        .into_iter()
        .map(|row| view_from_row(&row))
        .collect()
}

async fn load_navigation_dependencies(
    context: &DevqlGraphqlContext,
    repo_id: &str,
    view_ids: &[&str],
) -> Result<BTreeMap<String, Vec<NavigationContextViewDependency>>> {
    if view_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let sql = format!(
        "SELECT view_id, primitive_id, primitive_kind, primitive_hash, dependency_role, updated_at \
         FROM navigation_context_view_dependencies_current \
         WHERE repo_id = {} AND view_id IN ({}) ORDER BY view_id, primitive_id",
        sql_text(repo_id),
        sql_list(view_ids)
    );
    let mut out = BTreeMap::<String, Vec<NavigationContextViewDependency>>::new();
    for row in context.query_devql_sqlite_rows(&sql).await? {
        let dependency = dependency_from_row(&row)?;
        out.entry(dependency.view_id.clone())
            .or_default()
            .push(dependency);
    }
    Ok(out)
}

async fn load_navigation_acceptance_history(
    context: &DevqlGraphqlContext,
    repo_id: &str,
    view_ids: &[&str],
) -> Result<BTreeMap<String, Vec<NavigationContextViewAcceptance>>> {
    if view_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let sql = format!(
        "SELECT acceptance_id, view_id, previous_accepted_signature, accepted_signature, \
                current_signature, expected_current_signature, source, reason, materialised_ref, accepted_at \
         FROM navigation_context_view_acceptance_history \
         WHERE repo_id = {} AND view_id IN ({}) ORDER BY view_id, accepted_at DESC, acceptance_id DESC",
        sql_text(repo_id),
        sql_list(view_ids)
    );
    let mut out = BTreeMap::<String, Vec<NavigationContextViewAcceptance>>::new();
    for row in context.query_devql_sqlite_rows(&sql).await? {
        let acceptance = acceptance_from_row(&row)?;
        out.entry(acceptance.view_id.clone())
            .or_default()
            .push(acceptance);
    }
    Ok(out)
}

async fn load_navigation_primitives(
    context: &DevqlGraphqlContext,
    repo_id: &str,
    filter: Option<&NavigationContextFilterInput>,
    view_scoped_primitive_ids: Option<&BTreeSet<String>>,
) -> Result<Vec<NavigationPrimitive>> {
    if view_scoped_primitive_ids.is_some_and(BTreeSet::is_empty) {
        return Ok(Vec::new());
    }
    let mut clauses = vec![format!("repo_id = {}", sql_text(repo_id))];
    if let Some(ids) = view_scoped_primitive_ids {
        clauses.push(format!(
            "primitive_id IN ({})",
            sql_list(
                ids.iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .as_slice()
            )
        ));
    }
    if let Some(kind) = filter.and_then(|filter| filter.primitive_kind) {
        clauses.push(format!("primitive_kind = {}", sql_text(kind.as_db())));
    }
    if let Some(source_kind) = filter
        .and_then(|filter| filter.source_kind.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        clauses.push(format!("source_kind = {}", sql_text(source_kind)));
    }
    let sql = format!(
        "SELECT primitive_id, primitive_kind, identity_key, label, path, artefact_id, symbol_id, \
                source_kind, confidence, primitive_hash, hash_version, properties_json, provenance_json, \
                last_observed_generation, updated_at \
         FROM navigation_context_primitives_current WHERE {} ORDER BY primitive_id",
        clauses.join(" AND ")
    );
    context
        .query_devql_sqlite_rows(&sql)
        .await?
        .into_iter()
        .map(|row| primitive_from_row(&row))
        .collect()
}

async fn load_navigation_edges(
    context: &DevqlGraphqlContext,
    repo_id: &str,
    filter: Option<&NavigationContextFilterInput>,
) -> Result<Vec<NavigationEdge>> {
    let mut clauses = vec![format!("repo_id = {}", sql_text(repo_id))];
    if let Some(edge_kind) = filter
        .and_then(|filter| filter.edge_kind.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        clauses.push(format!("edge_kind = {}", sql_text(edge_kind)));
    }
    if let Some(source_kind) = filter
        .and_then(|filter| filter.source_kind.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        clauses.push(format!("source_kind = {}", sql_text(source_kind)));
    }
    let sql = format!(
        "SELECT edge_id, edge_kind, from_primitive_id, to_primitive_id, source_kind, confidence, \
                edge_hash, hash_version, properties_json, provenance_json, last_observed_generation, updated_at \
         FROM navigation_context_edges_current WHERE {} ORDER BY edge_id",
        clauses.join(" AND ")
    );
    context
        .query_devql_sqlite_rows(&sql)
        .await?
        .into_iter()
        .map(|row| edge_from_row(&row))
        .collect()
}

fn primitive_path_in_scope(
    primitive: &NavigationPrimitive,
    scope: &ResolverScope,
    filter: Option<&NavigationContextFilterInput>,
) -> bool {
    let Some(path) = primitive.path.as_deref() else {
        return filter.and_then(|filter| filter.path.as_deref()).is_none();
    };
    if !scope.contains_repo_path(path) {
        return false;
    }
    let Some(filter_path) = filter.and_then(|filter| filter.path.as_deref()) else {
        return true;
    };
    let filter_path = filter_path.trim();
    path == filter_path
        || path
            .strip_prefix(filter_path)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn view_from_row(row: &Value) -> Result<NavigationContextView> {
    let status = required_string(row, "status")?;
    Ok(NavigationContextView {
        view_id: required_string(row, "view_id")?,
        view_kind: required_string(row, "view_kind")?,
        label: required_string(row, "label")?,
        view_query_version: required_string(row, "view_query_version")?,
        dependency_query: json_scalar(json_column(row, "dependency_query_json")?),
        accepted_signature: required_string(row, "accepted_signature")?,
        current_signature: required_string(row, "current_signature")?,
        status: NavigationContextViewStatus::from_db(&status)
            .ok_or_else(|| anyhow!("unknown navigation context view status `{status}`"))?,
        stale_reason: json_scalar(json_column(row, "stale_reason_json")?),
        materialised_ref: optional_string(row, "materialised_ref"),
        last_observed_generation: optional_i32(row, "last_observed_generation"),
        updated_at: required_string(row, "updated_at")?,
        dependencies: Vec::new(),
        acceptance_history: Vec::new(),
    })
}

fn dependency_from_row(row: &Value) -> Result<NavigationContextViewDependency> {
    let primitive_kind = required_string(row, "primitive_kind")?;
    Ok(NavigationContextViewDependency {
        view_id: required_string(row, "view_id")?,
        primitive_id: required_string(row, "primitive_id")?,
        primitive_kind: NavigationPrimitiveKind::from_db(&primitive_kind)
            .ok_or_else(|| anyhow!("unknown navigation primitive kind `{primitive_kind}`"))?,
        primitive_hash: required_string(row, "primitive_hash")?,
        dependency_role: required_string(row, "dependency_role")?,
        updated_at: required_string(row, "updated_at")?,
    })
}

fn acceptance_from_row(row: &Value) -> Result<NavigationContextViewAcceptance> {
    Ok(NavigationContextViewAcceptance {
        acceptance_id: required_string(row, "acceptance_id")?,
        view_id: required_string(row, "view_id")?,
        previous_accepted_signature: required_string(row, "previous_accepted_signature")?,
        accepted_signature: required_string(row, "accepted_signature")?,
        current_signature: required_string(row, "current_signature")?,
        expected_current_signature: optional_string(row, "expected_current_signature"),
        source: required_string(row, "source")?,
        reason: optional_string(row, "reason"),
        materialised_ref: optional_string(row, "materialised_ref"),
        accepted_at: required_string(row, "accepted_at")?,
    })
}

fn primitive_from_row(row: &Value) -> Result<NavigationPrimitive> {
    let kind = required_string(row, "primitive_kind")?;
    Ok(NavigationPrimitive {
        id: required_string(row, "primitive_id")?,
        kind: NavigationPrimitiveKind::from_db(&kind)
            .ok_or_else(|| anyhow!("unknown navigation primitive kind `{kind}`"))?,
        identity_key: required_string(row, "identity_key")?,
        label: required_string(row, "label")?,
        path: optional_string(row, "path"),
        artefact_id: optional_string(row, "artefact_id"),
        symbol_id: optional_string(row, "symbol_id"),
        source_kind: required_string(row, "source_kind")?,
        confidence: number_field(row, "confidence"),
        primitive_hash: required_string(row, "primitive_hash")?,
        hash_version: required_string(row, "hash_version")?,
        properties: json_scalar(json_column(row, "properties_json")?),
        provenance: json_scalar(json_column(row, "provenance_json")?),
        last_observed_generation: optional_i32(row, "last_observed_generation"),
        updated_at: required_string(row, "updated_at")?,
    })
}

fn edge_from_row(row: &Value) -> Result<NavigationEdge> {
    Ok(NavigationEdge {
        id: required_string(row, "edge_id")?,
        kind: required_string(row, "edge_kind")?,
        from_primitive_id: required_string(row, "from_primitive_id")?,
        to_primitive_id: required_string(row, "to_primitive_id")?,
        source_kind: required_string(row, "source_kind")?,
        confidence: number_field(row, "confidence"),
        edge_hash: required_string(row, "edge_hash")?,
        hash_version: required_string(row, "hash_version")?,
        properties: json_scalar(json_column(row, "properties_json")?),
        provenance: json_scalar(json_column(row, "provenance_json")?),
        last_observed_generation: optional_i32(row, "last_observed_generation"),
        updated_at: required_string(row, "updated_at")?,
    })
}

fn required_string(row: &Value, key: &str) -> Result<String> {
    optional_string(row, key).ok_or_else(|| anyhow!("missing `{key}` in navigation context row"))
}

fn optional_string(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn optional_i32(row: &Value, key: &str) -> Option<i32> {
    row.get(key)
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
}

fn number_field(row: &Value, key: &str) -> f64 {
    row.get(key).and_then(Value::as_f64).unwrap_or(0.0)
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

fn sql_text(value: &str) -> String {
    format!("'{}'", esc_pg(value))
}

fn sql_list(values: &[&str]) -> String {
    values
        .iter()
        .map(|value| sql_text(value))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn primitive_from_row_parses_json_and_kind() {
        let primitive = primitive_from_row(&json!({
            "primitive_id": "p1",
            "primitive_kind": "SYMBOL",
            "identity_key": "symbol:s1",
            "label": "handler",
            "path": "src/main.rs",
            "artefact_id": "a1",
            "symbol_id": "s1",
            "source_kind": "DEVQL_CURRENT_STATE",
            "confidence": 1.0,
            "primitive_hash": "hash",
            "hash_version": "navigation-context-v1",
            "properties_json": "{\"language\":\"rust\"}",
            "provenance_json": "{\"source\":\"test\"}",
            "last_observed_generation": 7,
            "updated_at": "2026-05-03T00:00:00Z",
        }))
        .expect("primitive row should parse");

        assert_eq!(primitive.kind, NavigationPrimitiveKind::Symbol);
        assert_eq!(primitive.path.as_deref(), Some("src/main.rs"));
        assert_eq!(primitive.last_observed_generation, Some(7));
    }

    #[test]
    fn primitive_path_filter_respects_project_scope_and_prefix_filter() {
        let primitive = NavigationPrimitive {
            id: "p1".to_string(),
            kind: NavigationPrimitiveKind::FileBlob,
            identity_key: "file:src/app/main.rs".to_string(),
            label: "src/app/main.rs".to_string(),
            path: Some("src/app/main.rs".to_string()),
            artefact_id: None,
            symbol_id: None,
            source_kind: "DEVQL_CURRENT_STATE".to_string(),
            confidence: 1.0,
            primitive_hash: "hash".to_string(),
            hash_version: "navigation-context-v1".to_string(),
            properties: json_scalar(json!({})),
            provenance: json_scalar(json!({})),
            last_observed_generation: None,
            updated_at: "now".to_string(),
        };
        let scope = ResolverScope::default().with_project_path("src".to_string());
        let filter = NavigationContextFilterInput {
            path: Some("src/app".to_string()),
            ..NavigationContextFilterInput::default()
        };

        assert!(primitive_path_in_scope(&primitive, &scope, Some(&filter)));
    }
}
