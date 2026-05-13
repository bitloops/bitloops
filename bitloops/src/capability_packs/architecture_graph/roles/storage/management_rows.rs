use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use crate::host::devql::esc_pg;

use super::management::{
    ArchitectureRoleAliasRecord, ArchitectureRoleAssignmentMigrationRecord,
    ArchitectureRoleProposalRecord, ArchitectureRoleRecord, ArchitectureRoleRuleRecord,
};

pub(super) fn parse_role_row(row: &Value) -> Result<ArchitectureRoleRecord> {
    Ok(ArchitectureRoleRecord {
        role_id: string_field(row, "role_id")?,
        repo_id: string_field(row, "repo_id")?,
        canonical_key: string_field(row, "canonical_key")?,
        display_name: string_field(row, "display_name")?,
        description: string_field(row, "description")?,
        family: optional_string_field(row, "family"),
        lifecycle_status: string_field(row, "lifecycle_status")?,
        provenance: json_text_field(row, "provenance_json")?,
        evidence: json_text_field(row, "evidence_json")?,
        metadata: json_text_field(row, "metadata_json")?,
    })
}

pub(super) fn parse_alias_row(row: &Value) -> Result<ArchitectureRoleAliasRecord> {
    Ok(ArchitectureRoleAliasRecord {
        alias_id: string_field(row, "alias_id")?,
        repo_id: string_field(row, "repo_id")?,
        role_id: string_field(row, "role_id")?,
        alias_key: string_field(row, "alias_key")?,
        alias_normalized: string_field(row, "alias_normalized")?,
        source_kind: string_field(row, "source_kind")?,
        metadata: json_text_field(row, "metadata_json")?,
    })
}

pub(super) fn parse_rule_row(row: &Value) -> Result<ArchitectureRoleRuleRecord> {
    Ok(ArchitectureRoleRuleRecord {
        rule_id: string_field(row, "rule_id")?,
        repo_id: string_field(row, "repo_id")?,
        role_id: string_field(row, "role_id")?,
        version: u64_field(row, "version")?,
        lifecycle_status: string_field(row, "lifecycle_status")?,
        canonical_hash: string_field(row, "canonical_hash")?,
        candidate_selector: json_text_field(row, "candidate_selector_json")?,
        positive_conditions: json_text_field(row, "positive_conditions_json")?,
        negative_conditions: json_text_field(row, "negative_conditions_json")?,
        score: json_text_field(row, "score_json")?,
        provenance: json_text_field(row, "provenance_json")?,
        evidence: json_text_field(row, "evidence_json")?,
        metadata: json_text_field(row, "metadata_json")?,
        supersedes_rule_id: optional_string_field(row, "supersedes_rule_id"),
    })
}

pub(super) fn parse_proposal_row(row: &Value) -> Result<ArchitectureRoleProposalRecord> {
    Ok(ArchitectureRoleProposalRecord {
        proposal_id: string_field(row, "proposal_id")?,
        repo_id: string_field(row, "repo_id")?,
        proposal_type: string_field(row, "proposal_type")?,
        status: string_field(row, "status")?,
        request_payload: json_text_field(row, "request_payload_json")?,
        preview_payload: json_text_field(row, "preview_payload_json")?,
        result_payload: json_text_field(row, "result_payload_json")?,
        provenance: json_text_field(row, "provenance_json")?,
        applied_at: optional_string_field(row, "applied_at"),
    })
}

pub(super) fn parse_migration_row(
    row: &Value,
) -> Result<ArchitectureRoleAssignmentMigrationRecord> {
    Ok(ArchitectureRoleAssignmentMigrationRecord {
        migration_id: string_field(row, "migration_id")?,
        repo_id: string_field(row, "repo_id")?,
        proposal_id: string_field(row, "proposal_id")?,
        migration_type: string_field(row, "migration_type")?,
        status: string_field(row, "status")?,
        source_role_id: optional_string_field(row, "source_role_id"),
        target_role_id: optional_string_field(row, "target_role_id"),
        summary: json_text_field(row, "summary_json")?,
    })
}

fn string_field(row: &Value, key: &str) -> Result<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| anyhow!("row missing string field `{key}`"))
}

fn optional_string_field(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .filter(|value| !value.is_empty())
}

fn u64_field(row: &Value, key: &str) -> Result<u64> {
    row.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("row missing numeric field `{key}`"))
}

fn json_text_field(row: &Value, key: &str) -> Result<Value> {
    match row.get(key) {
        Some(Value::String(text)) => {
            serde_json::from_str(text).with_context(|| format!("parsing JSON field `{key}`"))
        }
        Some(Value::Null) | None => Ok(Value::Null),
        Some(other) => Ok(other.clone()),
    }
}

pub(super) fn sql_text(value: &str) -> String {
    format!("'{}'", esc_pg(value))
}

pub(super) fn sql_opt_text(value: Option<&str>) -> String {
    value.map(sql_text).unwrap_or_else(|| "NULL".to_string())
}
