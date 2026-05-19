use std::collections::BTreeSet;

use anyhow::{Result, anyhow};
use serde_json::Value;

use super::DevqlGraphqlContext;
use crate::graphql::ResolverScope;

#[derive(Debug, Clone)]
pub(crate) struct ArchitectureRoleOverviewAssignment {
    pub(crate) assignment_id: String,
    pub(crate) role_id: String,
    pub(crate) canonical_key: String,
    pub(crate) display_name: String,
    pub(crate) description: String,
    pub(crate) family: Option<String>,
    pub(crate) target_kind: String,
    pub(crate) artefact_id: Option<String>,
    pub(crate) symbol_id: Option<String>,
    pub(crate) path: String,
    pub(crate) symbol_fqn: Option<String>,
    pub(crate) canonical_kind: Option<String>,
    pub(crate) priority: String,
    pub(crate) status: String,
    pub(crate) source: String,
    pub(crate) confidence: f64,
    pub(crate) classifier_version: String,
    pub(crate) rule_version: Option<i64>,
}

#[derive(Debug, Clone)]
pub(crate) struct ArchitectureRoleTargetOverview {
    pub(crate) available: bool,
    pub(crate) reason: Option<String>,
    pub(crate) selected_artefact_count: usize,
    pub(crate) assigned_artefact_ids: Vec<String>,
    pub(crate) assignments: Vec<ArchitectureRoleOverviewAssignment>,
}

impl ArchitectureRoleTargetOverview {
    pub(crate) fn unavailable(selected_artefact_count: usize, reason: &str) -> Self {
        Self {
            available: false,
            reason: Some(reason.to_string()),
            selected_artefact_count,
            assigned_artefact_ids: Vec::new(),
            assignments: Vec::new(),
        }
    }
}

impl DevqlGraphqlContext {
    pub(crate) async fn architecture_role_overview_for_targets(
        &self,
        scope: &ResolverScope,
        artefact_ids: &[String],
        symbol_ids: &[String],
        paths: &[String],
    ) -> Result<ArchitectureRoleTargetOverview> {
        architecture_role_overview_for_targets(self, scope, artefact_ids, symbol_ids, paths).await
    }
}

async fn architecture_role_overview_for_targets(
    context: &DevqlGraphqlContext,
    scope: &ResolverScope,
    artefact_ids: &[String],
    symbol_ids: &[String],
    paths: &[String],
) -> Result<ArchitectureRoleTargetOverview> {
    let selected_artefact_count = artefact_ids.len();
    if artefact_ids.is_empty() && symbol_ids.is_empty() && paths.is_empty() {
        return Ok(ArchitectureRoleTargetOverview::unavailable(
            selected_artefact_count,
            "empty_selection",
        ));
    }
    if scope.temporal_scope().is_some() {
        return Ok(ArchitectureRoleTargetOverview::unavailable(
            selected_artefact_count,
            "unsupported_scope",
        ));
    }

    let repo_id = context.repo_id_for_scope(scope)?;
    let artefact_branch_predicate = if artefacts_current_has_branch_column(context).await? {
        format!(
            "          AND artefact.branch = {}\n",
            sql_text(&context.current_branch_name(scope))
        )
    } else {
        String::new()
    };
    let filters = role_assignment_target_filters(artefact_ids, symbol_ids, paths);
    if filters.is_empty() {
        return Ok(ArchitectureRoleTargetOverview::unavailable(
            selected_artefact_count,
            "empty_selection",
        ));
    }

    let sql = format!(
        "SELECT assignment.assignment_id, assignment.role_id, assignment.target_kind,
                assignment.artefact_id, assignment.symbol_id, assignment.path,
                assignment.priority, assignment.status, assignment.source, assignment.confidence,
                assignment.classifier_version, assignment.rule_version,
                role.canonical_key, role.display_name, role.description, role.family,
                artefact.symbol_fqn, artefact.canonical_kind
         FROM architecture_role_assignments_current assignment
         JOIN architecture_roles role
           ON role.repo_id = assignment.repo_id AND role.role_id = assignment.role_id
         LEFT JOIN artefacts_current artefact
           ON artefact.repo_id = assignment.repo_id
{}
          AND (
            artefact.artefact_id = assignment.artefact_id
            OR artefact.symbol_id = assignment.symbol_id
          )
         WHERE assignment.repo_id = {}
           AND assignment.status = 'active'
           AND ({})
         ORDER BY assignment.path ASC, assignment.priority ASC, assignment.confidence DESC, role.canonical_key ASC, assignment.assignment_id ASC",
        artefact_branch_predicate,
        sql_text(&repo_id),
        filters.join(" OR "),
    );

    let rows = match context.query_devql_sqlite_rows(&sql).await {
        Ok(rows) => rows,
        Err(err) if is_missing_architecture_role_table_error(&err) => {
            return Ok(ArchitectureRoleTargetOverview::unavailable(
                selected_artefact_count,
                "missing_architecture_role_tables",
            ));
        }
        Err(err) => return Err(err),
    };

    let assignments = rows
        .iter()
        .map(role_assignment_from_row)
        .collect::<Result<Vec<_>>>()?;
    Ok(role_overview_from_assignments(
        selected_artefact_count,
        artefact_ids,
        symbol_ids,
        assignments,
    ))
}

async fn artefacts_current_has_branch_column(context: &DevqlGraphqlContext) -> Result<bool> {
    let rows = context
        .query_devql_sqlite_rows("PRAGMA table_info(artefacts_current)")
        .await?;
    Ok(rows
        .iter()
        .any(|row| row.get("name").and_then(Value::as_str) == Some("branch")))
}

fn role_overview_from_assignments(
    selected_artefact_count: usize,
    artefact_ids: &[String],
    symbol_ids: &[String],
    mut assignments: Vec<ArchitectureRoleOverviewAssignment>,
) -> ArchitectureRoleTargetOverview {
    assignments.sort_by(|left, right| {
        role_assignment_sort_key(left).cmp(&role_assignment_sort_key(right))
    });

    if assignments.is_empty() {
        return ArchitectureRoleTargetOverview::unavailable(
            selected_artefact_count,
            "no_matching_architecture_role_assignments",
        );
    }

    let artefact_ids = artefact_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let symbol_ids = symbol_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let assigned_artefact_ids = assignments
        .iter()
        .filter_map(|assignment| {
            assignment
                .artefact_id
                .as_deref()
                .filter(|id| artefact_ids.contains(id))
                .map(str::to_string)
                .or_else(|| {
                    assignment
                        .symbol_id
                        .as_deref()
                        .filter(|id| symbol_ids.contains(id))
                        .and_then(|_| assignment.artefact_id.clone())
                })
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    ArchitectureRoleTargetOverview {
        available: true,
        reason: None,
        selected_artefact_count,
        assigned_artefact_ids,
        assignments,
    }
}

fn role_assignment_sort_key(
    assignment: &ArchitectureRoleOverviewAssignment,
) -> (
    u8,
    u8,
    std::cmp::Reverse<OrderedConfidence>,
    String,
    String,
    String,
) {
    (
        status_rank(&assignment.status),
        priority_rank(&assignment.priority),
        std::cmp::Reverse(OrderedConfidence(assignment.confidence)),
        assignment.path.clone(),
        assignment.canonical_key.clone(),
        assignment.assignment_id.clone(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct OrderedConfidence(f64);

impl Eq for OrderedConfidence {}

impl PartialOrd for OrderedConfidence {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedConfidence {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

fn status_rank(status: &str) -> u8 {
    match status {
        "active" => 0,
        "needs_review" => 1,
        "stale" => 2,
        "rejected" => 3,
        _ => 4,
    }
}

fn priority_rank(priority: &str) -> u8 {
    match priority {
        "primary" => 0,
        "secondary" => 1,
        _ => 2,
    }
}

fn role_assignment_target_filters(
    artefact_ids: &[String],
    symbol_ids: &[String],
    paths: &[String],
) -> Vec<String> {
    let mut filters = Vec::new();
    if !artefact_ids.is_empty() {
        filters.push(format!(
            "assignment.artefact_id IN ({})",
            sql_string_list(artefact_ids)
        ));
    }
    if !symbol_ids.is_empty() {
        filters.push(format!(
            "assignment.symbol_id IN ({})",
            sql_string_list(symbol_ids)
        ));
    }
    if !paths.is_empty() {
        filters.push(format!("assignment.path IN ({})", sql_string_list(paths)));
    }
    filters
}

fn sql_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| sql_text(value))
        .collect::<Vec<_>>()
        .join(", ")
}

fn sql_text(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn is_missing_architecture_role_table_error(err: &anyhow::Error) -> bool {
    let message = err.to_string();
    message.contains("architecture_role_assignments_current")
        || message.contains("architecture_roles")
}

fn role_assignment_from_row(row: &Value) -> Result<ArchitectureRoleOverviewAssignment> {
    Ok(ArchitectureRoleOverviewAssignment {
        assignment_id: required_string(row, "assignment_id")?,
        role_id: required_string(row, "role_id")?,
        canonical_key: required_string(row, "canonical_key")?,
        display_name: required_string(row, "display_name")?,
        description: required_string(row, "description")?,
        family: optional_string(row, "family"),
        target_kind: required_string(row, "target_kind")?,
        artefact_id: optional_string(row, "artefact_id"),
        symbol_id: optional_string(row, "symbol_id"),
        path: required_string(row, "path")?,
        symbol_fqn: optional_string(row, "symbol_fqn"),
        canonical_kind: optional_string(row, "canonical_kind"),
        priority: required_string(row, "priority")?,
        status: required_string(row, "status")?,
        source: required_string(row, "source")?,
        confidence: row
            .get("confidence")
            .and_then(Value::as_f64)
            .ok_or_else(|| anyhow!("missing `confidence` in architecture role assignment row"))?,
        classifier_version: required_string(row, "classifier_version")?,
        rule_version: row.get("rule_version").and_then(Value::as_i64),
    })
}

fn required_string(row: &Value, field: &str) -> Result<String> {
    row.get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("missing `{field}` in architecture role assignment row"))
}

fn optional_string(row: &Value, field: &str) -> Option<String> {
    row.get(field).and_then(Value::as_str).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assignment(
        assignment_id: &str,
        artefact_id: Option<&str>,
        symbol_id: Option<&str>,
        target_kind: &str,
    ) -> ArchitectureRoleOverviewAssignment {
        ArchitectureRoleOverviewAssignment {
            assignment_id: assignment_id.to_string(),
            role_id: "role-http".to_string(),
            canonical_key: "http_api_surface".to_string(),
            display_name: "HTTP API Surface".to_string(),
            description: "HTTP handlers".to_string(),
            family: Some("entrypoint".to_string()),
            target_kind: target_kind.to_string(),
            artefact_id: artefact_id.map(str::to_string),
            symbol_id: symbol_id.map(str::to_string),
            path: "src/api/users_handler.rs".to_string(),
            symbol_fqn: Some("src/api/users_handler.rs::create_user_http_handler".to_string()),
            canonical_kind: Some("function".to_string()),
            priority: "primary".to_string(),
            status: "active".to_string(),
            source: "rule".to_string(),
            confidence: 1.0,
            classifier_version: "architecture_roles.deterministic.contract.v1".to_string(),
            rule_version: None,
        }
    }

    #[test]
    fn role_overview_collects_assigned_selected_artefact_ids() {
        let assignments = vec![
            assignment(
                "a1",
                Some("artefact-create"),
                Some("symbol-create"),
                "artefact",
            ),
            assignment("a2", None, None, "file"),
        ];

        let overview = role_overview_from_assignments(
            3,
            &["artefact-create".to_string(), "artefact-file".to_string()],
            &["symbol-create".to_string()],
            assignments,
        );

        assert!(overview.available);
        assert_eq!(overview.assigned_artefact_ids, vec!["artefact-create"]);
        assert_eq!(overview.assignments.len(), 2);
    }
}
