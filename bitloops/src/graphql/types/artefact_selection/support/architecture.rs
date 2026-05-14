use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

use crate::graphql::context::{
    ArchitectureGraphTargetOverview, ArchitectureRoleOverviewAssignment,
    ArchitectureRoleTargetOverview,
};
use crate::graphql::types::ArchitectureGraphNodeKind;

use super::super::stages::{
    ArchitectureOverviewStageData, ArchitectureRoleAssignmentItem, ArchitectureRoleInfo,
    ArchitectureRoleTarget,
};
use super::schemas::ARCHITECTURE_OVERVIEW_SCHEMA;

pub(in crate::graphql::types::artefact_selection) fn build_architecture_overview_stage(
    overview: ArchitectureRoleTargetOverview,
    graph_context_available: bool,
) -> ArchitectureOverviewStageData {
    if !overview.available {
        let reason = overview
            .reason
            .as_deref()
            .unwrap_or("no_matching_architecture_role_assignments");
        return ArchitectureOverviewStageData::unavailable(
            overview.selected_artefact_count,
            reason,
            graph_context_available,
        );
    }

    let summary = architecture_role_overview_summary(&overview, graph_context_available);
    ArchitectureOverviewStageData {
        summary,
        expand_hint: Some(architecture_overview_expand_hint()),
        schema: Some(ARCHITECTURE_OVERVIEW_SCHEMA.to_string()),
    }
}

fn architecture_role_overview_summary(
    overview: &ArchitectureRoleTargetOverview,
    graph_context_available: bool,
) -> Value {
    json!({
        "available": overview.available,
        "reason": overview.reason,
        "selectedArtefactCount": overview.selected_artefact_count,
        "assignedSelectedArtefactCount": overview.assigned_artefact_ids.len(),
        "unassignedSelectedArtefactCount": overview
            .selected_artefact_count
            .saturating_sub(overview.assigned_artefact_ids.len()),
        "roleAssignmentCount": overview.assignments.len(),
        "roleCount": overview
            .assignments
            .iter()
            .map(|assignment| assignment.role_id.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        "familyCounts": count_strings(overview.assignments.iter().filter_map(|assignment| assignment.family.as_deref())),
        "sourceCounts": count_strings(overview.assignments.iter().map(|assignment| assignment.source.as_str())),
        "targetKindCounts": count_strings(overview.assignments.iter().map(|assignment| assignment.target_kind.as_str())),
        "confidence": confidence_summary(overview.assignments.iter().map(|assignment| assignment.confidence)),
        "primaryRoles": grouped_role_summaries(&overview.assignments),
        "graphContextAvailable": graph_context_available,
    })
}

pub(in crate::graphql::types::artefact_selection) fn architecture_overview_expand_hint() -> Value {
    json!({
        "intent": "Inspect architecture role assignments for the selected artefacts.",
        "template": "bitloops devql query '{ selectArtefacts(by: <selector>) { architectureRoles { overview items(first: 20) { role { canonicalKey displayName family description } target { path symbolFqn canonicalKind } confidence source status } } architectureGraphContext { overview nodes(first: 20) { id kind label path } edges(first: 20) { id kind fromNodeId toNodeId } } } }'"
    })
}

#[derive(Debug)]
struct RoleSummaryAccumulator<'a> {
    role_id: &'a str,
    canonical_key: &'a str,
    display_name: &'a str,
    description: &'a str,
    family: Option<&'a str>,
    assignments: Vec<&'a ArchitectureRoleOverviewAssignment>,
}

fn grouped_role_summaries(assignments: &[ArchitectureRoleOverviewAssignment]) -> Vec<Value> {
    let mut grouped = BTreeMap::<String, RoleSummaryAccumulator<'_>>::new();
    for assignment in assignments {
        grouped
            .entry(assignment.role_id.clone())
            .and_modify(|entry| entry.assignments.push(assignment))
            .or_insert_with(|| RoleSummaryAccumulator {
                role_id: &assignment.role_id,
                canonical_key: &assignment.canonical_key,
                display_name: &assignment.display_name,
                description: &assignment.description,
                family: assignment.family.as_deref(),
                assignments: vec![assignment],
            });
    }

    let mut summaries = grouped
        .into_values()
        .map(role_summary_json)
        .collect::<Vec<_>>();
    summaries.sort_by(compare_role_summary_json);
    summaries.truncate(5);
    summaries
}

fn role_summary_json(role: RoleSummaryAccumulator<'_>) -> Value {
    json!({
        "roleId": role.role_id,
        "canonicalKey": role.canonical_key,
        "displayName": role.display_name,
        "family": role.family,
        "description": role.description,
        "assignmentCount": role.assignments.len(),
        "confidence": confidence_summary(role.assignments.iter().map(|assignment| assignment.confidence)),
        "targetKinds": count_strings(role.assignments.iter().map(|assignment| assignment.target_kind.as_str())),
        "sources": count_strings(role.assignments.iter().map(|assignment| assignment.source.as_str())),
        "targets": role
            .assignments
            .iter()
            .take(5)
            .map(|assignment| architecture_role_target_json(assignment))
            .collect::<Vec<_>>(),
    })
}

fn compare_role_summary_json(left: &Value, right: &Value) -> std::cmp::Ordering {
    right["assignmentCount"]
        .as_u64()
        .unwrap_or(0)
        .cmp(&left["assignmentCount"].as_u64().unwrap_or(0))
        .then_with(|| {
            left["canonicalKey"]
                .as_str()
                .unwrap_or_default()
                .cmp(right["canonicalKey"].as_str().unwrap_or_default())
        })
        .then_with(|| {
            left["roleId"]
                .as_str()
                .unwrap_or_default()
                .cmp(right["roleId"].as_str().unwrap_or_default())
        })
}

fn count_strings<'a>(values: impl Iterator<Item = &'a str>) -> Value {
    let mut counts = serde_json::Map::new();
    for value in values {
        let next = counts.get(value).and_then(Value::as_u64).unwrap_or(0) + 1;
        counts.insert(value.to_string(), json!(next));
    }
    Value::Object(counts)
}

fn confidence_summary(values: impl Iterator<Item = f64>) -> Value {
    let values = values.collect::<Vec<_>>();
    if values.is_empty() {
        return Value::Null;
    }
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let avg = values.iter().sum::<f64>() / values.len() as f64;
    json!({
        "min": round_confidence(min),
        "avg": round_confidence(avg),
        "max": round_confidence(max),
    })
}

fn round_confidence(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn architecture_role_target_json(assignment: &ArchitectureRoleOverviewAssignment) -> Value {
    json!({
        "assignmentId": assignment.assignment_id,
        "targetKind": assignment.target_kind,
        "path": assignment.path,
        "artefactId": assignment.artefact_id,
        "symbolId": assignment.symbol_id,
        "symbolFqn": assignment.symbol_fqn,
        "canonicalKind": assignment.canonical_kind,
        "priority": assignment.priority,
        "status": assignment.status,
        "source": assignment.source,
        "confidence": round_confidence(assignment.confidence),
    })
}

pub(in crate::graphql::types::artefact_selection) fn architecture_role_assignment_item(
    assignment: &ArchitectureRoleOverviewAssignment,
) -> ArchitectureRoleAssignmentItem {
    ArchitectureRoleAssignmentItem {
        assignment_id: assignment.assignment_id.clone(),
        role: ArchitectureRoleInfo {
            role_id: assignment.role_id.clone(),
            canonical_key: assignment.canonical_key.clone(),
            display_name: assignment.display_name.clone(),
            family: assignment.family.clone(),
            description: assignment.description.clone(),
        },
        target: ArchitectureRoleTarget {
            target_kind: assignment.target_kind.clone(),
            path: assignment.path.clone(),
            artefact_id: assignment.artefact_id.clone(),
            symbol_id: assignment.symbol_id.clone(),
            symbol_fqn: assignment.symbol_fqn.clone(),
            canonical_kind: assignment.canonical_kind.clone(),
        },
        priority: assignment.priority.clone(),
        status: assignment.status.clone(),
        source: assignment.source.clone(),
        confidence: round_confidence(assignment.confidence),
        classifier_version: assignment.classifier_version.clone(),
        rule_version: assignment.rule_version,
    }
}

pub(in crate::graphql::types::artefact_selection) fn build_architecture_graph_context_summary(
    overview: &ArchitectureGraphTargetOverview,
) -> Value {
    if !overview.available {
        let reason = overview
            .reason
            .as_deref()
            .unwrap_or("no_matching_architecture_facts");
        return json!({
            "available": false,
            "reason": reason,
            "selectedArtefactCount": overview.selected_artefact_count,
            "matchedArtefactCount": 0,
            "directNodeCount": 0,
            "relatedNodeCount": 0,
            "edgeCount": 0,
            "nodeKinds": {},
            "entryPointCount": 0,
            "componentCount": 0,
            "containerCount": 0,
            "assertedCount": 0,
            "suppressedCount": 0,
        });
    }

    let mut node_kinds = serde_json::Map::new();
    let mut entry_point_count = 0usize;
    let mut component_count = 0usize;
    let mut container_count = 0usize;
    let mut asserted_count = 0usize;
    let mut suppressed_count = 0usize;
    for node in &overview.nodes {
        let kind = node.kind.as_db();
        let next = node_kinds
            .get(kind)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
            + 1;
        node_kinds.insert(kind.to_string(), json!(next));
        match node.kind {
            ArchitectureGraphNodeKind::EntryPoint => entry_point_count += 1,
            ArchitectureGraphNodeKind::Component => component_count += 1,
            ArchitectureGraphNodeKind::Container => container_count += 1,
            _ => {}
        }
        if node.asserted {
            asserted_count += 1;
        }
        if node.suppressed {
            suppressed_count += 1;
        }
    }

    json!({
        "available": true,
        "reason": null,
        "selectedArtefactCount": overview.selected_artefact_count,
        "matchedArtefactCount": overview.matched_artefact_ids.len(),
        "directNodeCount": overview.direct_node_count,
        "relatedNodeCount": overview.nodes.len(),
        "edgeCount": overview.edges.len(),
        "nodeKinds": Value::Object(node_kinds),
        "entryPointCount": entry_point_count,
        "componentCount": component_count,
        "containerCount": container_count,
        "assertedCount": asserted_count,
        "suppressedCount": suppressed_count,
    })
}
