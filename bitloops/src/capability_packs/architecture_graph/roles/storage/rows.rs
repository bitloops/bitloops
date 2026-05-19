use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};

use crate::capability_packs::architecture_graph::roles::taxonomy::{
    ArchitectureArtefactFact, ArchitectureRole, ArchitectureRoleAssignment,
    ArchitectureRoleDetectionRule, AssignmentPriority, AssignmentSource, AssignmentStatus,
    RoleLifecycle, RoleRuleLifecycle, RoleTarget, TargetKind,
};

pub(super) fn role_from_row(row: Value) -> Result<ArchitectureRole> {
    let provenance = row_json(&row, "provenance_json", json!({}))?;
    Ok(ArchitectureRole {
        repo_id: row_string(&row, "repo_id")?,
        role_id: row_string(&row, "role_id")?,
        family: row_string(&row, "family")?,
        slug: row_string(&row, "slug")?,
        display_name: row_string(&row, "display_name")?,
        description: row_string(&row, "description")?,
        lifecycle: role_lifecycle_from_db(&row_string(&row, "lifecycle")?)?,
        provenance,
    })
}

pub(super) fn assignment_from_row(row: Value) -> Result<ArchitectureRoleAssignment> {
    let target = RoleTarget {
        target_kind: target_kind_from_db(&row_string(&row, "target_kind")?)?,
        artefact_id: row_opt_string(&row, "artefact_id"),
        symbol_id: row_opt_string(&row, "symbol_id"),
        path: row_string(&row, "path")?,
    };
    Ok(ArchitectureRoleAssignment {
        repo_id: row_string(&row, "repo_id")?,
        assignment_id: row_string(&row, "assignment_id")?,
        role_id: row_string(&row, "role_id")?,
        target,
        priority: assignment_priority_from_db(&row_string(&row, "priority")?)?,
        status: assignment_status_from_db(&row_string(&row, "status")?)?,
        source: assignment_source_from_db(&row_string(&row, "source")?)?,
        confidence: row_f64(&row, "confidence")?,
        evidence: row_json(&row, "evidence_json", json!([]))?,
        provenance: row_json(&row, "provenance_json", json!({}))?,
        classifier_version: row_string(&row, "classifier_version")?,
        rule_version: row.get("rule_version").and_then(Value::as_i64),
        generation_seq: row_u64(&row, "generation_seq")?,
    })
}

pub(super) fn fact_from_row(row: Value) -> Result<ArchitectureArtefactFact> {
    Ok(ArchitectureArtefactFact {
        repo_id: row_string(&row, "repo_id")?,
        fact_id: row_string(&row, "fact_id")?,
        target: RoleTarget {
            target_kind: target_kind_from_db(&row_string(&row, "target_kind")?)?,
            artefact_id: row_opt_string(&row, "artefact_id"),
            symbol_id: row_opt_string(&row, "symbol_id"),
            path: row_string(&row, "path")?,
        },
        language: row_opt_string(&row, "language"),
        fact_kind: row_string(&row, "fact_kind")?,
        fact_key: row_string(&row, "fact_key")?,
        fact_value: row_string(&row, "fact_value")?,
        source: row_string(&row, "source")?,
        confidence: row_f64(&row, "confidence")?,
        evidence: row_json(&row, "evidence_json", json!([]))?,
        generation_seq: row_u64(&row, "generation_seq")?,
    })
}

pub(super) fn detection_rule_from_row(row: Value) -> Result<ArchitectureRoleDetectionRule> {
    Ok(ArchitectureRoleDetectionRule {
        repo_id: row_string(&row, "repo_id")?,
        rule_id: row_string(&row, "rule_id")?,
        role_id: row_string(&row, "role_id")?,
        version: row_i64(&row, "version")?,
        lifecycle: role_rule_lifecycle_from_db(&row_string(&row, "lifecycle")?)?,
        priority: row_i64(&row, "priority")?,
        score: row_f64(&row, "score")?,
        candidate_selector: row_json(&row, "candidate_selector_json", json!({}))?,
        positive_conditions: row_json(&row, "positive_conditions_json", json!([]))?,
        negative_conditions: row_json(&row, "negative_conditions_json", json!([]))?,
        provenance: row_json(&row, "provenance_json", json!({}))?,
    })
}

fn assignment_priority_from_db(value: &str) -> Result<AssignmentPriority> {
    match value {
        "primary" => Ok(AssignmentPriority::Primary),
        "secondary" => Ok(AssignmentPriority::Secondary),
        other => Err(anyhow!(
            "unknown architecture role assignment priority `{other}`"
        )),
    }
}

fn assignment_source_from_db(value: &str) -> Result<AssignmentSource> {
    match value {
        "rule" => Ok(AssignmentSource::Rule),
        "llm" => Ok(AssignmentSource::Llm),
        "human" => Ok(AssignmentSource::Human),
        "migration" => Ok(AssignmentSource::Migration),
        other => Err(anyhow!(
            "unknown architecture role assignment source `{other}`"
        )),
    }
}

fn assignment_status_from_db(value: &str) -> Result<AssignmentStatus> {
    match value {
        "active" => Ok(AssignmentStatus::Active),
        "stale" => Ok(AssignmentStatus::Stale),
        "needs_review" => Ok(AssignmentStatus::NeedsReview),
        "rejected" => Ok(AssignmentStatus::Rejected),
        other => Err(anyhow!(
            "unknown architecture role assignment status `{other}`"
        )),
    }
}

fn role_lifecycle_from_db(value: &str) -> Result<RoleLifecycle> {
    match value {
        "active" => Ok(RoleLifecycle::Active),
        "deprecated" => Ok(RoleLifecycle::Deprecated),
        "removed" => Ok(RoleLifecycle::Removed),
        other => Err(anyhow!("unknown architecture role lifecycle `{other}`")),
    }
}

fn role_rule_lifecycle_from_db(value: &str) -> Result<RoleRuleLifecycle> {
    match value {
        "draft" => Ok(RoleRuleLifecycle::Draft),
        "active" => Ok(RoleRuleLifecycle::Active),
        "disabled" => Ok(RoleRuleLifecycle::Disabled),
        "deprecated" => Ok(RoleRuleLifecycle::Deprecated),
        other => Err(anyhow!(
            "unknown architecture role rule lifecycle `{other}`"
        )),
    }
}

fn target_kind_from_db(value: &str) -> Result<TargetKind> {
    match value {
        "file" => Ok(TargetKind::File),
        "artefact" => Ok(TargetKind::Artefact),
        "symbol" => Ok(TargetKind::Symbol),
        other => Err(anyhow!("unknown architecture role target kind `{other}`")),
    }
}

pub(super) fn sql_text(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub(super) fn sql_opt_text(value: Option<&str>) -> String {
    value.map(sql_text).unwrap_or_else(|| "NULL".to_string())
}

pub(super) fn sql_opt_i64(value: Option<i64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "NULL".to_string())
}

fn row_string(row: &Value, key: &str) -> Result<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("architecture role row missing string field `{key}`"))
}

fn row_opt_string(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn row_f64(row: &Value, key: &str) -> Result<f64> {
    row.get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow!("architecture role row missing numeric field `{key}`"))
}

fn row_u64(row: &Value, key: &str) -> Result<u64> {
    row.get(key)
        .and_then(Value::as_i64)
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| anyhow!("architecture role row missing unsigned integer field `{key}`"))
}

fn row_i64(row: &Value, key: &str) -> Result<i64> {
    row.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("architecture role row missing integer field `{key}`"))
}

fn row_json(row: &Value, key: &str, default: Value) -> Result<Value> {
    match row.get(key) {
        Some(Value::String(raw)) if !raw.trim().is_empty() => serde_json::from_str(raw)
            .with_context(|| format!("parsing architecture role JSON field `{key}`")),
        Some(Value::String(_)) | Some(Value::Null) | None => Ok(default),
        Some(value) => Ok(value.clone()),
    }
}
