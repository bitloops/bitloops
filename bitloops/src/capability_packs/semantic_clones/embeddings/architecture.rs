use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};

use anyhow::Result;
use serde_json::json;

use crate::capability_packs::semantic_clones::features::SemanticFeatureInput;
use crate::host::devql::{RelationalStorage, esc_pg, sql_string_list_pg};

use super::text::normalize_whitespace;
use super::types::{ArchitectureRoleEmbeddingRole, SymbolEmbeddingInput};

pub fn build_architecture_embedding_text(input: &SymbolEmbeddingInput) -> String {
    let mut lines = vec![
        format!("kind: {}", normalize_whitespace(&input.canonical_kind)),
        format!("language: {}", normalize_whitespace(&input.language)),
        format!("name: {}", normalize_whitespace(&input.name)),
        format!("path: {}", normalize_whitespace(&input.path)),
        "architecture_roles:".to_string(),
    ];
    for role in sorted_architecture_roles(&input.architecture_roles) {
        lines.push(format!(
            "- canonical_key: {}",
            normalize_whitespace(&role.canonical_key)
        ));
        lines.push(format!(
            "  display_name: {}",
            normalize_whitespace(&role.display_name)
        ));
        lines.push(format!("  family: {}", normalize_whitespace(&role.family)));
        lines.push(format!(
            "  description: {}",
            normalize_whitespace(&role.description)
        ));
    }
    lines.join("\n")
}

pub fn architecture_roles_hash_value(roles: &[ArchitectureRoleEmbeddingRole]) -> serde_json::Value {
    json!(
        sorted_architecture_roles(roles)
            .into_iter()
            .map(|role| {
                json!({
                    "role_id": role.role_id,
                    "assignment_id": role.assignment_id,
                    "canonical_key": normalize_whitespace(&role.canonical_key),
                    "display_name": normalize_whitespace(&role.display_name),
                    "family": normalize_whitespace(&role.family),
                    "description": normalize_whitespace(&role.description),
                    "priority": normalize_whitespace(&role.priority),
                    "confidence": role.confidence,
                })
            })
            .collect::<Vec<_>>()
    )
}

fn sorted_architecture_roles(
    roles: &[ArchitectureRoleEmbeddingRole],
) -> Vec<ArchitectureRoleEmbeddingRole> {
    let mut seen = BTreeSet::new();
    let mut sorted = roles
        .iter()
        .filter(|role| {
            seen.insert((
                role.role_id.clone(),
                role.canonical_key.to_ascii_lowercase(),
            ))
        })
        .cloned()
        .collect::<Vec<_>>();
    sorted.sort_by(compare_architecture_roles);
    sorted
}

fn compare_architecture_roles(
    left: &ArchitectureRoleEmbeddingRole,
    right: &ArchitectureRoleEmbeddingRole,
) -> Ordering {
    priority_rank(&left.priority)
        .cmp(&priority_rank(&right.priority))
        .then_with(|| {
            right
                .confidence
                .partial_cmp(&left.confidence)
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| left.canonical_key.cmp(&right.canonical_key))
        .then_with(|| left.assignment_id.cmp(&right.assignment_id))
}

fn priority_rank(priority: &str) -> (u8, String) {
    match priority.trim().to_ascii_lowercase().as_str() {
        "primary" => (0, "primary".to_string()),
        "secondary" => (1, "secondary".to_string()),
        other => (2, other.to_string()),
    }
}

pub async fn load_architecture_roles_for_embedding_inputs(
    relational: &RelationalStorage,
    repo_id: &str,
    inputs: &[SemanticFeatureInput],
) -> Result<HashMap<String, Vec<ArchitectureRoleEmbeddingRole>>> {
    let artefact_ids = unique_non_empty(inputs.iter().map(|input| input.artefact_id.as_str()));
    let symbol_ids = unique_non_empty(inputs.iter().filter_map(|input| input.symbol_id.as_deref()));
    let paths = unique_non_empty(inputs.iter().map(|input| input.path.as_str()));
    if artefact_ids.is_empty() && symbol_ids.is_empty() && paths.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = relational
        .query_rows(&format!(
            "SELECT \
               a.assignment_id, \
               a.role_id, \
               a.priority, \
               a.confidence, \
               r.canonical_key, \
               r.display_name, \
               r.family, \
               r.description, \
               CASE \
                 WHEN a.target_kind = 'artefact' THEN a.artefact_id \
                 WHEN a.target_kind = 'symbol' THEN COALESCE(a.artefact_id, target.artefact_id) \
                 WHEN a.target_kind = 'file' THEN target.artefact_id \
                 ELSE NULL \
               END AS resolved_artefact_id \
             FROM architecture_role_assignments_current a \
             JOIN architecture_roles r \
               ON r.repo_id = a.repo_id \
              AND r.role_id = a.role_id \
             LEFT JOIN artefacts_current target \
               ON target.repo_id = a.repo_id \
              AND ( \
                   (a.target_kind = 'symbol' AND a.artefact_id IS NULL AND target.symbol_id = a.symbol_id) \
                OR (a.target_kind = 'file' AND target.path = a.path AND LOWER(COALESCE(target.canonical_kind, COALESCE(target.language_kind, ''))) = 'file') \
              ) \
             WHERE a.repo_id = '{repo_id}' \
               AND a.status = 'active' \
               AND r.lifecycle_status = 'active' \
               AND ({scope_filter}) \
             ORDER BY resolved_artefact_id, a.priority, a.confidence DESC, r.canonical_key, a.assignment_id",
            repo_id = esc_pg(repo_id),
            scope_filter = architecture_role_scope_filter(&artefact_ids, &symbol_ids, &paths),
        ))
        .await?;

    let input_artefact_ids = artefact_ids.into_iter().collect::<BTreeSet<_>>();
    let mut roles_by_artefact = HashMap::<String, Vec<ArchitectureRoleEmbeddingRole>>::new();
    for row in rows {
        let Some(artefact_id) = row
            .get("resolved_artefact_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .filter(|artefact_id| input_artefact_ids.contains(artefact_id))
        else {
            continue;
        };
        let Some(role) = architecture_role_from_row(&row) else {
            continue;
        };
        roles_by_artefact.entry(artefact_id).or_default().push(role);
    }
    Ok(roles_by_artefact)
}

fn unique_non_empty<'a>(values: impl Iterator<Item = &'a str>) -> Vec<String> {
    values
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn architecture_role_scope_filter(
    artefact_ids: &[String],
    symbol_ids: &[String],
    paths: &[String],
) -> String {
    let mut filters = Vec::new();
    if !artefact_ids.is_empty() {
        filters.push(format!(
            "a.artefact_id IN ({})",
            sql_string_list_pg(artefact_ids)
        ));
    }
    if !symbol_ids.is_empty() {
        filters.push(format!(
            "a.symbol_id IN ({})",
            sql_string_list_pg(symbol_ids)
        ));
    }
    if !paths.is_empty() {
        filters.push(format!("a.path IN ({})", sql_string_list_pg(paths)));
    }
    filters.join(" OR ")
}

fn architecture_role_from_row(row: &serde_json::Value) -> Option<ArchitectureRoleEmbeddingRole> {
    Some(ArchitectureRoleEmbeddingRole {
        role_id: string_field(row, "role_id")?,
        assignment_id: string_field(row, "assignment_id")?,
        canonical_key: string_field(row, "canonical_key")?,
        display_name: string_field(row, "display_name")?,
        family: string_field(row, "family")?,
        description: string_field(row, "description").unwrap_or_default(),
        priority: string_field(row, "priority")?,
        confidence: row.get("confidence")?.as_f64()?,
    })
}

fn string_field(row: &serde_json::Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}
