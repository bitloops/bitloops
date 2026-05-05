use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use crate::host::devql::{RelationalStorage, deterministic_uuid, esc_pg, sql_json_value, sql_now};

#[derive(Debug, Clone, PartialEq)]
pub struct ArchitectureRoleRecord {
    pub role_id: String,
    pub repo_id: String,
    pub canonical_key: String,
    pub display_name: String,
    pub description: String,
    pub family: Option<String>,
    pub lifecycle_status: String,
    pub provenance: Value,
    pub evidence: Value,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArchitectureRoleAliasRecord {
    pub alias_id: String,
    pub repo_id: String,
    pub role_id: String,
    pub alias_key: String,
    pub alias_normalized: String,
    pub source_kind: String,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArchitectureRoleRuleRecord {
    pub rule_id: String,
    pub repo_id: String,
    pub role_id: String,
    pub version: u64,
    pub lifecycle_status: String,
    pub canonical_hash: String,
    pub candidate_selector: Value,
    pub positive_conditions: Value,
    pub negative_conditions: Value,
    pub score: Value,
    pub provenance: Value,
    pub evidence: Value,
    pub metadata: Value,
    pub supersedes_rule_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArchitectureRoleAssignmentRecord {
    pub assignment_id: String,
    pub repo_id: String,
    pub artefact_id: String,
    pub role_id: String,
    pub source_kind: String,
    pub confidence: f64,
    pub status: String,
    pub status_reason: String,
    pub rule_id: Option<String>,
    pub migration_id: Option<String>,
    pub migrated_to_assignment_id: Option<String>,
    pub provenance: Value,
    pub evidence: Value,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArchitectureRoleProposalRecord {
    pub proposal_id: String,
    pub repo_id: String,
    pub proposal_type: String,
    pub status: String,
    pub request_payload: Value,
    pub preview_payload: Value,
    pub result_payload: Value,
    pub provenance: Value,
    pub applied_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArchitectureRoleAssignmentMigrationRecord {
    pub migration_id: String,
    pub repo_id: String,
    pub proposal_id: String,
    pub migration_type: String,
    pub status: String,
    pub source_role_id: Option<String>,
    pub target_role_id: Option<String>,
    pub summary: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AliasConflict {
    AlreadyAssignedToDifferentRole {
        alias: String,
        existing_role_id: String,
    },
}

pub fn normalize_role_key(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut previous_was_sep = false;
    for ch in value.trim().chars() {
        let normalized = match ch {
            'A'..='Z' => ch.to_ascii_lowercase(),
            'a'..='z' | '0'..='9' => ch,
            _ => '_',
        };
        if normalized == '_' {
            if previous_was_sep || out.is_empty() {
                continue;
            }
            previous_was_sep = true;
            out.push('_');
        } else {
            previous_was_sep = false;
            out.push(normalized);
        }
    }
    out.trim_matches('_').to_string()
}

pub fn normalize_role_alias(alias: &str) -> String {
    normalize_role_key(alias)
}

pub fn deterministic_role_id(repo_id: &str, canonical_key: &str) -> String {
    deterministic_uuid(&format!(
        "architecture_roles|role|{repo_id}|{}",
        normalize_role_key(canonical_key)
    ))
}

pub fn deterministic_alias_id(repo_id: &str, alias_key: &str) -> String {
    deterministic_uuid(&format!(
        "architecture_roles|alias|{repo_id}|{}",
        normalize_role_alias(alias_key)
    ))
}

pub fn deterministic_rule_id(
    repo_id: &str,
    role_id: &str,
    version: u64,
    canonical_hash: &str,
) -> String {
    deterministic_uuid(&format!(
        "architecture_roles|rule|{repo_id}|{role_id}|{version}|{canonical_hash}"
    ))
}

pub fn deterministic_assignment_id(repo_id: &str, artefact_id: &str, role_id: &str) -> String {
    deterministic_uuid(&format!(
        "architecture_roles|assignment|{repo_id}|{artefact_id}|{role_id}"
    ))
}

pub fn deterministic_proposal_id(repo_id: &str, proposal_type: &str, request_hash: &str) -> String {
    deterministic_uuid(&format!(
        "architecture_roles|proposal|{repo_id}|{proposal_type}|{request_hash}"
    ))
}

pub fn deterministic_migration_id(
    repo_id: &str,
    proposal_id: &str,
    migration_type: &str,
) -> String {
    deterministic_uuid(&format!(
        "architecture_roles|migration|{repo_id}|{proposal_id}|{migration_type}"
    ))
}

pub async fn upsert_role(
    relational: &RelationalStorage,
    role: &ArchitectureRoleRecord,
) -> Result<ArchitectureRoleRecord> {
    let canonical_key = normalize_role_key(&role.canonical_key);
    relational
        .exec_serialized(&format!(
            "INSERT INTO architecture_roles (
                repo_id, role_id, canonical_key, display_name, description, family,
                lifecycle_status, provenance_json, evidence_json, metadata_json, created_at, updated_at
            ) VALUES (
                {repo_id}, {role_id}, {canonical_key}, {display_name}, {description}, {family},
                {lifecycle_status}, {provenance}, {evidence}, {metadata}, {now}, {now}
            )
            ON CONFLICT(repo_id, canonical_key) DO UPDATE SET
                display_name = excluded.display_name,
                description = excluded.description,
                family = excluded.family,
                lifecycle_status = excluded.lifecycle_status,
                provenance_json = excluded.provenance_json,
                evidence_json = excluded.evidence_json,
                metadata_json = excluded.metadata_json,
                updated_at = excluded.updated_at;",
            repo_id = sql_text(&role.repo_id),
            role_id = sql_text(&role.role_id),
            canonical_key = sql_text(&canonical_key),
            display_name = sql_text(&role.display_name),
            description = sql_text(&role.description),
            family = sql_opt_text(role.family.as_deref()),
            lifecycle_status = sql_text(&role.lifecycle_status),
            provenance = sql_json_value(relational, &role.provenance),
            evidence = sql_json_value(relational, &role.evidence),
            metadata = sql_json_value(relational, &role.metadata),
            now = sql_now(relational),
        ))
        .await
        .context("upserting architecture role")?;

    load_role_by_canonical_key(relational, &role.repo_id, &canonical_key)
        .await?
        .ok_or_else(|| anyhow!("role `{canonical_key}` was not found after upsert"))
}

pub async fn load_role_by_id(
    relational: &RelationalStorage,
    repo_id: &str,
    role_id: &str,
) -> Result<Option<ArchitectureRoleRecord>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT repo_id, role_id, canonical_key, display_name, description, family, \
                    lifecycle_status, provenance_json, evidence_json, metadata_json \
             FROM architecture_roles \
             WHERE repo_id = {repo_id} AND role_id = {role_id} \
             LIMIT 1;",
            repo_id = sql_text(repo_id),
            role_id = sql_text(role_id),
        ))
        .await
        .context("loading architecture role by id")?;

    rows.first().map(parse_role_row).transpose()
}

pub async fn load_role_by_canonical_key(
    relational: &RelationalStorage,
    repo_id: &str,
    canonical_key: &str,
) -> Result<Option<ArchitectureRoleRecord>> {
    let canonical_key = normalize_role_key(canonical_key);
    let rows = relational
        .query_rows(&format!(
            "SELECT repo_id, role_id, canonical_key, display_name, description, family, \
                    lifecycle_status, provenance_json, evidence_json, metadata_json \
             FROM architecture_roles \
             WHERE repo_id = {repo_id} AND canonical_key = {canonical_key} \
             LIMIT 1;",
            repo_id = sql_text(repo_id),
            canonical_key = sql_text(&canonical_key),
        ))
        .await
        .context("loading architecture role by canonical key")?;

    rows.first().map(parse_role_row).transpose()
}

pub async fn list_roles(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<Vec<ArchitectureRoleRecord>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT repo_id, role_id, canonical_key, display_name, description, family, \
                    lifecycle_status, provenance_json, evidence_json, metadata_json \
             FROM architecture_roles \
             WHERE repo_id = {repo_id} \
             ORDER BY canonical_key ASC;",
            repo_id = sql_text(repo_id),
        ))
        .await
        .context("listing architecture roles")?;
    rows.iter().map(parse_role_row).collect()
}

pub async fn load_role_by_alias(
    relational: &RelationalStorage,
    repo_id: &str,
    alias: &str,
) -> Result<Option<ArchitectureRoleRecord>> {
    let alias = normalize_role_alias(alias);
    let rows = relational
        .query_rows(&format!(
            "SELECT r.repo_id, r.role_id, r.canonical_key, r.display_name, r.description, r.family, \
                    r.lifecycle_status, r.provenance_json, r.evidence_json, r.metadata_json \
             FROM architecture_role_aliases a \
             JOIN architecture_roles r ON r.repo_id = a.repo_id AND r.role_id = a.role_id \
             WHERE a.repo_id = {repo_id} AND a.alias_normalized = {alias} \
             LIMIT 1;",
            repo_id = sql_text(repo_id),
            alias = sql_text(&alias),
        ))
        .await
        .context("loading architecture role by alias")?;

    rows.first().map(parse_role_row).transpose()
}

pub async fn list_role_aliases(
    relational: &RelationalStorage,
    repo_id: &str,
    role_id: &str,
) -> Result<Vec<ArchitectureRoleAliasRecord>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT repo_id, alias_id, role_id, alias_key, alias_normalized, source_kind, metadata_json \
             FROM architecture_role_aliases \
             WHERE repo_id = {repo_id} AND role_id = {role_id} \
             ORDER BY alias_key ASC;",
            repo_id = sql_text(repo_id),
            role_id = sql_text(role_id),
        ))
        .await
        .context("listing architecture role aliases")?;
    rows.iter().map(parse_alias_row).collect()
}

pub async fn create_role_alias(
    relational: &RelationalStorage,
    alias: &ArchitectureRoleAliasRecord,
) -> Result<std::result::Result<(), AliasConflict>> {
    let alias_normalized = normalize_role_alias(&alias.alias_key);
    let existing_rows = relational
        .query_rows(&format!(
            "SELECT role_id FROM architecture_role_aliases \
             WHERE repo_id = {repo_id} AND alias_normalized = {alias_normalized} \
             LIMIT 1;",
            repo_id = sql_text(&alias.repo_id),
            alias_normalized = sql_text(&alias_normalized),
        ))
        .await
        .context("checking existing architecture alias")?;

    if let Some(existing_role_id) = existing_rows
        .first()
        .and_then(|row| row.get("role_id"))
        .and_then(Value::as_str)
    {
        if existing_role_id == alias.role_id {
            relational
                .exec_serialized(&format!(
                    "UPDATE architecture_role_aliases SET \
                        alias_key = {alias_key}, source_kind = {source_kind}, metadata_json = {metadata}, updated_at = {now} \
                     WHERE repo_id = {repo_id} AND alias_normalized = {alias_normalized};",
                    alias_key = sql_text(&alias.alias_key),
                    source_kind = sql_text(&alias.source_kind),
                    metadata = sql_json_value(relational, &alias.metadata),
                    now = sql_now(relational),
                    repo_id = sql_text(&alias.repo_id),
                    alias_normalized = sql_text(&alias_normalized),
                ))
                .await
                .context("updating existing architecture alias")?;
            return Ok(Ok(()));
        }

        return Ok(Err(AliasConflict::AlreadyAssignedToDifferentRole {
            alias: alias_normalized,
            existing_role_id: existing_role_id.to_string(),
        }));
    }

    relational
        .exec_serialized(&format!(
            "INSERT INTO architecture_role_aliases (
                repo_id, alias_id, role_id, alias_key, alias_normalized, source_kind, metadata_json, created_at, updated_at
            ) VALUES (
                {repo_id}, {alias_id}, {role_id}, {alias_key}, {alias_normalized}, {source_kind}, {metadata}, {now}, {now}
            );",
            repo_id = sql_text(&alias.repo_id),
            alias_id = sql_text(&alias.alias_id),
            role_id = sql_text(&alias.role_id),
            alias_key = sql_text(&alias.alias_key),
            alias_normalized = sql_text(&alias_normalized),
            source_kind = sql_text(&alias.source_kind),
            metadata = sql_json_value(relational, &alias.metadata),
            now = sql_now(relational),
        ))
        .await
        .context("creating architecture role alias")?;
    Ok(Ok(()))
}

pub async fn insert_role_rule(
    relational: &RelationalStorage,
    rule: &ArchitectureRoleRuleRecord,
) -> Result<()> {
    relational
        .exec_serialized(&format!(
            "INSERT INTO architecture_role_detection_rules (
                repo_id, rule_id, role_id, version, lifecycle_status, canonical_hash,
                candidate_selector_json, positive_conditions_json, negative_conditions_json, score_json,
                provenance_json, evidence_json, metadata_json, supersedes_rule_id, created_at, updated_at
            ) VALUES (
                {repo_id}, {rule_id}, {role_id}, {version}, {lifecycle_status}, {canonical_hash},
                {candidate_selector}, {positive_conditions}, {negative_conditions}, {score},
                {provenance}, {evidence}, {metadata}, {supersedes_rule_id}, {now}, {now}
            )
            ON CONFLICT(repo_id, rule_id) DO UPDATE SET
                role_id = excluded.role_id,
                version = excluded.version,
                lifecycle_status = excluded.lifecycle_status,
                canonical_hash = excluded.canonical_hash,
                candidate_selector_json = excluded.candidate_selector_json,
                positive_conditions_json = excluded.positive_conditions_json,
                negative_conditions_json = excluded.negative_conditions_json,
                score_json = excluded.score_json,
                provenance_json = excluded.provenance_json,
                evidence_json = excluded.evidence_json,
                metadata_json = excluded.metadata_json,
                supersedes_rule_id = excluded.supersedes_rule_id,
                updated_at = excluded.updated_at;",
            repo_id = sql_text(&rule.repo_id),
            rule_id = sql_text(&rule.rule_id),
            role_id = sql_text(&rule.role_id),
            version = rule.version,
            lifecycle_status = sql_text(&rule.lifecycle_status),
            canonical_hash = sql_text(&rule.canonical_hash),
            candidate_selector = sql_json_value(relational, &rule.candidate_selector),
            positive_conditions = sql_json_value(relational, &rule.positive_conditions),
            negative_conditions = sql_json_value(relational, &rule.negative_conditions),
            score = sql_json_value(relational, &rule.score),
            provenance = sql_json_value(relational, &rule.provenance),
            evidence = sql_json_value(relational, &rule.evidence),
            metadata = sql_json_value(relational, &rule.metadata),
            supersedes_rule_id = sql_opt_text(rule.supersedes_rule_id.as_deref()),
            now = sql_now(relational),
        ))
        .await
        .context("inserting architecture role rule")
}

pub async fn load_role_rule_by_id(
    relational: &RelationalStorage,
    repo_id: &str,
    rule_id: &str,
) -> Result<Option<ArchitectureRoleRuleRecord>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT repo_id, rule_id, role_id, version, lifecycle_status, canonical_hash,
                    candidate_selector_json, positive_conditions_json, negative_conditions_json, score_json,
                    provenance_json, evidence_json, metadata_json, supersedes_rule_id \
             FROM architecture_role_detection_rules \
             WHERE repo_id = {repo_id} AND rule_id = {rule_id} \
             LIMIT 1;",
            repo_id = sql_text(repo_id),
            rule_id = sql_text(rule_id),
        ))
        .await
        .context("loading architecture role rule by id")?;
    rows.first().map(parse_rule_row).transpose()
}

pub async fn load_role_rules(
    relational: &RelationalStorage,
    repo_id: &str,
    role_id: &str,
) -> Result<Vec<ArchitectureRoleRuleRecord>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT repo_id, rule_id, role_id, version, lifecycle_status, canonical_hash,
                    candidate_selector_json, positive_conditions_json, negative_conditions_json, score_json,
                    provenance_json, evidence_json, metadata_json, supersedes_rule_id \
             FROM architecture_role_detection_rules \
             WHERE repo_id = {repo_id} AND role_id = {role_id} \
             ORDER BY version ASC, rule_id ASC;",
            repo_id = sql_text(repo_id),
            role_id = sql_text(role_id),
        ))
        .await
        .context("loading architecture role rules")?;
    rows.iter().map(parse_rule_row).collect()
}

pub async fn next_role_rule_version(
    relational: &RelationalStorage,
    repo_id: &str,
    role_id: &str,
) -> Result<u64> {
    let rows = relational
        .query_rows(&format!(
            "SELECT COALESCE(MAX(version), 0) AS max_version \
             FROM architecture_role_detection_rules \
             WHERE repo_id = {repo_id} AND role_id = {role_id};",
            repo_id = sql_text(repo_id),
            role_id = sql_text(role_id),
        ))
        .await
        .context("loading next rule version")?;
    let max = rows
        .first()
        .and_then(|row| row.get("max_version"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Ok(max + 1)
}

pub async fn update_role_rule_lifecycle(
    relational: &RelationalStorage,
    repo_id: &str,
    rule_id: &str,
    lifecycle_status: &str,
) -> Result<bool> {
    let before = load_role_rule_by_id(relational, repo_id, rule_id).await?;
    if before.is_none() {
        return Ok(false);
    }
    relational
        .exec_serialized(&format!(
            "UPDATE architecture_role_detection_rules SET \
                lifecycle_status = {lifecycle_status}, updated_at = {now} \
             WHERE repo_id = {repo_id} AND rule_id = {rule_id};",
            lifecycle_status = sql_text(lifecycle_status),
            now = sql_now(relational),
            repo_id = sql_text(repo_id),
            rule_id = sql_text(rule_id),
        ))
        .await
        .context("updating architecture role rule lifecycle")?;
    Ok(true)
}

pub async fn insert_role_assignment(
    relational: &RelationalStorage,
    assignment: &ArchitectureRoleAssignmentRecord,
) -> Result<()> {
    relational
        .exec_serialized(&format!(
            "INSERT INTO architecture_role_assignments (
                repo_id, assignment_id, artefact_id, role_id, source_kind, confidence, status,
                status_reason, rule_id, migration_id, migrated_to_assignment_id,
                provenance_json, evidence_json, metadata_json, created_at, updated_at
            ) VALUES (
                {repo_id}, {assignment_id}, {artefact_id}, {role_id}, {source_kind}, {confidence}, {status},
                {status_reason}, {rule_id}, {migration_id}, {migrated_to_assignment_id},
                {provenance}, {evidence}, {metadata}, {now}, {now}
            )
            ON CONFLICT(repo_id, assignment_id) DO UPDATE SET
                artefact_id = excluded.artefact_id,
                role_id = excluded.role_id,
                source_kind = excluded.source_kind,
                confidence = excluded.confidence,
                status = excluded.status,
                status_reason = excluded.status_reason,
                rule_id = excluded.rule_id,
                migration_id = excluded.migration_id,
                migrated_to_assignment_id = excluded.migrated_to_assignment_id,
                provenance_json = excluded.provenance_json,
                evidence_json = excluded.evidence_json,
                metadata_json = excluded.metadata_json,
                updated_at = excluded.updated_at;",
            repo_id = sql_text(&assignment.repo_id),
            assignment_id = sql_text(&assignment.assignment_id),
            artefact_id = sql_text(&assignment.artefact_id),
            role_id = sql_text(&assignment.role_id),
            source_kind = sql_text(&assignment.source_kind),
            confidence = assignment.confidence,
            status = sql_text(&assignment.status),
            status_reason = sql_text(&assignment.status_reason),
            rule_id = sql_opt_text(assignment.rule_id.as_deref()),
            migration_id = sql_opt_text(assignment.migration_id.as_deref()),
            migrated_to_assignment_id = sql_opt_text(assignment.migrated_to_assignment_id.as_deref()),
            provenance = sql_json_value(relational, &assignment.provenance),
            evidence = sql_json_value(relational, &assignment.evidence),
            metadata = sql_json_value(relational, &assignment.metadata),
            now = sql_now(relational),
        ))
        .await
        .context("inserting architecture role assignment")
}

pub async fn load_assignment_by_id(
    relational: &RelationalStorage,
    repo_id: &str,
    assignment_id: &str,
) -> Result<Option<ArchitectureRoleAssignmentRecord>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT repo_id, assignment_id, artefact_id, role_id, source_kind, confidence, status, \
                    status_reason, rule_id, migration_id, migrated_to_assignment_id, provenance_json, evidence_json, metadata_json \
             FROM architecture_role_assignments \
             WHERE repo_id = {repo_id} AND assignment_id = {assignment_id} \
             LIMIT 1;",
            repo_id = sql_text(repo_id),
            assignment_id = sql_text(assignment_id),
        ))
        .await
        .context("loading architecture role assignment by id")?;
    rows.first().map(parse_assignment_row).transpose()
}

pub async fn list_assignments_for_role(
    relational: &RelationalStorage,
    repo_id: &str,
    role_id: &str,
) -> Result<Vec<ArchitectureRoleAssignmentRecord>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT repo_id, assignment_id, artefact_id, role_id, source_kind, confidence, status, \
                    status_reason, rule_id, migration_id, migrated_to_assignment_id, provenance_json, evidence_json, metadata_json \
             FROM architecture_role_assignments \
             WHERE repo_id = {repo_id} AND role_id = {role_id} \
             ORDER BY assignment_id ASC;",
            repo_id = sql_text(repo_id),
            role_id = sql_text(role_id),
        ))
        .await
        .context("listing assignments for role")?;
    rows.iter().map(parse_assignment_row).collect()
}

pub async fn update_assignment_status(
    relational: &RelationalStorage,
    repo_id: &str,
    assignment_id: &str,
    status: &str,
    reason: &str,
    migration_id: Option<&str>,
) -> Result<bool> {
    if load_assignment_by_id(relational, repo_id, assignment_id)
        .await?
        .is_none()
    {
        return Ok(false);
    }
    relational
        .exec_serialized(&format!(
            "UPDATE architecture_role_assignments SET \
                status = {status}, status_reason = {reason}, migration_id = {migration_id}, updated_at = {now} \
             WHERE repo_id = {repo_id} AND assignment_id = {assignment_id};",
            status = sql_text(status),
            reason = sql_text(reason),
            migration_id = sql_opt_text(migration_id),
            now = sql_now(relational),
            repo_id = sql_text(repo_id),
            assignment_id = sql_text(assignment_id),
        ))
        .await
        .context("updating assignment status")?;
    Ok(true)
}

pub async fn mark_assignment_invalidated(
    relational: &RelationalStorage,
    repo_id: &str,
    assignment_id: &str,
    reason: &str,
) -> Result<bool> {
    update_assignment_status(
        relational,
        repo_id,
        assignment_id,
        "needs_review",
        reason,
        None,
    )
    .await
}

pub async fn mark_assignment_migrated(
    relational: &RelationalStorage,
    repo_id: &str,
    assignment_id: &str,
    migrated_to_assignment_id: &str,
    migration_id: Option<&str>,
) -> Result<bool> {
    if load_assignment_by_id(relational, repo_id, assignment_id)
        .await?
        .is_none()
    {
        return Ok(false);
    }
    relational
        .exec_serialized(&format!(
            "UPDATE architecture_role_assignments SET \
                status = 'migrated', status_reason = 'migrated by proposal', \
                migration_id = {migration_id}, migrated_to_assignment_id = {migrated_to_assignment_id}, updated_at = {now} \
             WHERE repo_id = {repo_id} AND assignment_id = {assignment_id};",
            migration_id = sql_opt_text(migration_id),
            migrated_to_assignment_id = sql_text(migrated_to_assignment_id),
            now = sql_now(relational),
            repo_id = sql_text(repo_id),
            assignment_id = sql_text(assignment_id),
        ))
        .await
        .context("marking assignment migrated")?;
    Ok(true)
}

pub async fn insert_role_proposal(
    relational: &RelationalStorage,
    proposal: &ArchitectureRoleProposalRecord,
) -> Result<()> {
    relational
        .exec_serialized(&format!(
            "INSERT INTO architecture_role_change_proposals (
                repo_id, proposal_id, proposal_type, status, request_payload_json, preview_payload_json,
                result_payload_json, provenance_json, created_at, updated_at, applied_at
            ) VALUES (
                {repo_id}, {proposal_id}, {proposal_type}, {status}, {request_payload}, {preview_payload},
                {result_payload}, {provenance}, {now}, {now}, {applied_at}
            )
            ON CONFLICT(repo_id, proposal_id) DO UPDATE SET
                proposal_type = excluded.proposal_type,
                status = excluded.status,
                request_payload_json = excluded.request_payload_json,
                preview_payload_json = excluded.preview_payload_json,
                result_payload_json = excluded.result_payload_json,
                provenance_json = excluded.provenance_json,
                updated_at = excluded.updated_at,
                applied_at = excluded.applied_at;",
            repo_id = sql_text(&proposal.repo_id),
            proposal_id = sql_text(&proposal.proposal_id),
            proposal_type = sql_text(&proposal.proposal_type),
            status = sql_text(&proposal.status),
            request_payload = sql_json_value(relational, &proposal.request_payload),
            preview_payload = sql_json_value(relational, &proposal.preview_payload),
            result_payload = sql_json_value(relational, &proposal.result_payload),
            provenance = sql_json_value(relational, &proposal.provenance),
            now = sql_now(relational),
            applied_at = sql_opt_text(proposal.applied_at.as_deref()),
        ))
        .await
        .context("inserting architecture role proposal")
}

pub async fn load_role_proposal_by_id(
    relational: &RelationalStorage,
    repo_id: &str,
    proposal_id: &str,
) -> Result<Option<ArchitectureRoleProposalRecord>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT repo_id, proposal_id, proposal_type, status, request_payload_json, preview_payload_json,
                    result_payload_json, provenance_json, applied_at \
             FROM architecture_role_change_proposals \
             WHERE repo_id = {repo_id} AND proposal_id = {proposal_id} \
             LIMIT 1;",
            repo_id = sql_text(repo_id),
            proposal_id = sql_text(proposal_id),
        ))
        .await
        .context("loading architecture role proposal")?;
    rows.first().map(parse_proposal_row).transpose()
}

pub async fn update_role_proposal_preview(
    relational: &RelationalStorage,
    repo_id: &str,
    proposal_id: &str,
    preview_payload: &Value,
) -> Result<bool> {
    if load_role_proposal_by_id(relational, repo_id, proposal_id)
        .await?
        .is_none()
    {
        return Ok(false);
    }
    relational
        .exec_serialized(&format!(
            "UPDATE architecture_role_change_proposals SET \
                preview_payload_json = {preview_payload}, updated_at = {now} \
             WHERE repo_id = {repo_id} AND proposal_id = {proposal_id};",
            preview_payload = sql_json_value(relational, preview_payload),
            now = sql_now(relational),
            repo_id = sql_text(repo_id),
            proposal_id = sql_text(proposal_id),
        ))
        .await
        .context("updating proposal preview")?;
    Ok(true)
}

pub async fn mark_role_proposal_applied(
    relational: &RelationalStorage,
    repo_id: &str,
    proposal_id: &str,
    result_payload: &Value,
) -> Result<bool> {
    if load_role_proposal_by_id(relational, repo_id, proposal_id)
        .await?
        .is_none()
    {
        return Ok(false);
    }
    relational
        .exec_serialized(&format!(
            "UPDATE architecture_role_change_proposals SET \
                status = 'applied', result_payload_json = {result_payload}, applied_at = {now}, updated_at = {now} \
             WHERE repo_id = {repo_id} AND proposal_id = {proposal_id};",
            result_payload = sql_json_value(relational, result_payload),
            now = sql_now(relational),
            repo_id = sql_text(repo_id),
            proposal_id = sql_text(proposal_id),
        ))
        .await
        .context("marking proposal applied")?;
    Ok(true)
}

pub async fn insert_assignment_migration_record(
    relational: &RelationalStorage,
    migration: &ArchitectureRoleAssignmentMigrationRecord,
) -> Result<()> {
    relational
        .exec_serialized(&format!(
            "INSERT INTO architecture_role_assignment_migrations (
                repo_id, migration_id, proposal_id, migration_type, status, source_role_id,
                target_role_id, summary_json, created_at, updated_at
            ) VALUES (
                {repo_id}, {migration_id}, {proposal_id}, {migration_type}, {status}, {source_role_id},
                {target_role_id}, {summary}, {now}, {now}
            )
            ON CONFLICT(repo_id, migration_id) DO UPDATE SET
                proposal_id = excluded.proposal_id,
                migration_type = excluded.migration_type,
                status = excluded.status,
                source_role_id = excluded.source_role_id,
                target_role_id = excluded.target_role_id,
                summary_json = excluded.summary_json,
                updated_at = excluded.updated_at;",
            repo_id = sql_text(&migration.repo_id),
            migration_id = sql_text(&migration.migration_id),
            proposal_id = sql_text(&migration.proposal_id),
            migration_type = sql_text(&migration.migration_type),
            status = sql_text(&migration.status),
            source_role_id = sql_opt_text(migration.source_role_id.as_deref()),
            target_role_id = sql_opt_text(migration.target_role_id.as_deref()),
            summary = sql_json_value(relational, &migration.summary),
            now = sql_now(relational),
        ))
        .await
        .context("inserting assignment migration record")
}

pub async fn list_assignment_migrations_for_proposal(
    relational: &RelationalStorage,
    repo_id: &str,
    proposal_id: &str,
) -> Result<Vec<ArchitectureRoleAssignmentMigrationRecord>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT repo_id, migration_id, proposal_id, migration_type, status, source_role_id, target_role_id, summary_json \
             FROM architecture_role_assignment_migrations \
             WHERE repo_id = {repo_id} AND proposal_id = {proposal_id} \
             ORDER BY migration_id ASC;",
            repo_id = sql_text(repo_id),
            proposal_id = sql_text(proposal_id),
        ))
        .await
        .context("listing assignment migrations for proposal")?;
    rows.iter().map(parse_migration_row).collect()
}

fn parse_role_row(row: &Value) -> Result<ArchitectureRoleRecord> {
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

fn parse_alias_row(row: &Value) -> Result<ArchitectureRoleAliasRecord> {
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

fn parse_rule_row(row: &Value) -> Result<ArchitectureRoleRuleRecord> {
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

fn parse_assignment_row(row: &Value) -> Result<ArchitectureRoleAssignmentRecord> {
    Ok(ArchitectureRoleAssignmentRecord {
        assignment_id: string_field(row, "assignment_id")?,
        repo_id: string_field(row, "repo_id")?,
        artefact_id: string_field(row, "artefact_id")?,
        role_id: string_field(row, "role_id")?,
        source_kind: string_field(row, "source_kind")?,
        confidence: f64_field(row, "confidence")?,
        status: string_field(row, "status")?,
        status_reason: string_field(row, "status_reason")?,
        rule_id: optional_string_field(row, "rule_id"),
        migration_id: optional_string_field(row, "migration_id"),
        migrated_to_assignment_id: optional_string_field(row, "migrated_to_assignment_id"),
        provenance: json_text_field(row, "provenance_json")?,
        evidence: json_text_field(row, "evidence_json")?,
        metadata: json_text_field(row, "metadata_json")?,
    })
}

fn parse_proposal_row(row: &Value) -> Result<ArchitectureRoleProposalRecord> {
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

fn parse_migration_row(row: &Value) -> Result<ArchitectureRoleAssignmentMigrationRecord> {
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

fn f64_field(row: &Value, key: &str) -> Result<f64> {
    row.get(key)
        .and_then(Value::as_f64)
        .or_else(|| {
            row.get(key)
                .and_then(Value::as_i64)
                .map(|value| value as f64)
        })
        .ok_or_else(|| anyhow!("row missing float field `{key}`"))
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

fn sql_text(value: &str) -> String {
    format!("'{}'", esc_pg(value))
}

fn sql_opt_text(value: Option<&str>) -> String {
    value.map(sql_text).unwrap_or_else(|| "NULL".to_string())
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use serde_json::json;
    use tempfile::tempdir;

    use super::*;
    use crate::capability_packs::architecture_graph::schema::architecture_graph_sqlite_schema_sql;

    fn test_role() -> ArchitectureRoleRecord {
        ArchitectureRoleRecord {
            role_id: deterministic_role_id("repo-1", "domain_owner"),
            repo_id: "repo-1".to_string(),
            canonical_key: "domain_owner".to_string(),
            display_name: "Domain Owner".to_string(),
            description: "Owns the domain".to_string(),
            family: Some("domain".to_string()),
            lifecycle_status: "active".to_string(),
            provenance: json!({"source": "test"}),
            evidence: json!([{ "path": "src/payments" }]),
            metadata: json!({"scope": "payments"}),
        }
    }

    fn test_alias(role_id: &str, alias: &str) -> ArchitectureRoleAliasRecord {
        ArchitectureRoleAliasRecord {
            alias_id: deterministic_alias_id("repo-1", alias),
            repo_id: "repo-1".to_string(),
            role_id: role_id.to_string(),
            alias_key: alias.to_string(),
            alias_normalized: normalize_role_alias(alias),
            source_kind: "manual".to_string(),
            metadata: json!({"source": "test"}),
        }
    }

    fn test_rule(role_id: &str) -> ArchitectureRoleRuleRecord {
        let version = 1;
        let canonical_hash = "rule-hash-1";
        ArchitectureRoleRuleRecord {
            rule_id: deterministic_rule_id("repo-1", role_id, version, canonical_hash),
            repo_id: "repo-1".to_string(),
            role_id: role_id.to_string(),
            version,
            lifecycle_status: "draft".to_string(),
            canonical_hash: canonical_hash.to_string(),
            candidate_selector: json!({"path_prefixes": ["src/payments"]}),
            positive_conditions: json!([{ "kind": "path_contains", "value": "payments" }]),
            negative_conditions: json!([]),
            score: json!({ "base_confidence": 0.82 }),
            provenance: json!({"source": "seed"}),
            evidence: json!(["src/payments/service.rs"]),
            metadata: json!({"reviewable": true}),
            supersedes_rule_id: None,
        }
    }

    fn test_assignment(role_id: &str) -> ArchitectureRoleAssignmentRecord {
        ArchitectureRoleAssignmentRecord {
            assignment_id: deterministic_assignment_id("repo-1", "artefact-1", role_id),
            repo_id: "repo-1".to_string(),
            artefact_id: "artefact-1".to_string(),
            role_id: role_id.to_string(),
            source_kind: "deterministic_rule".to_string(),
            confidence: 0.91,
            status: "active".to_string(),
            status_reason: String::new(),
            rule_id: Some("rule-1".to_string()),
            migration_id: None,
            migrated_to_assignment_id: None,
            provenance: json!({"source": "test"}),
            evidence: json!(["src/payments/service.rs"]),
            metadata: json!({"ticket": "ARCH-1"}),
        }
    }

    fn test_proposal() -> ArchitectureRoleProposalRecord {
        ArchitectureRoleProposalRecord {
            proposal_id: "proposal-1".to_string(),
            repo_id: "repo-1".to_string(),
            proposal_type: "rename_role".to_string(),
            status: "draft".to_string(),
            request_payload: json!({"role_id": "role-1", "display_name": "Payments Domain Owner"}),
            preview_payload: json!({"affected_assignments": 1}),
            result_payload: json!({}),
            provenance: json!({"source": "cli"}),
            applied_at: None,
        }
    }

    fn test_migration(proposal_id: &str) -> ArchitectureRoleAssignmentMigrationRecord {
        ArchitectureRoleAssignmentMigrationRecord {
            migration_id: deterministic_migration_id("repo-1", proposal_id, "merge_roles"),
            repo_id: "repo-1".to_string(),
            proposal_id: proposal_id.to_string(),
            migration_type: "merge_roles".to_string(),
            status: "applied".to_string(),
            source_role_id: Some("role-1".to_string()),
            target_role_id: Some("role-2".to_string()),
            summary: json!({"migrated_assignments": 1}),
        }
    }

    async fn sqlite_relational_with_schema() -> Result<RelationalStorage> {
        let temp = tempdir()?;
        let sqlite_path = temp.path().join("roles.sqlite");
        rusqlite::Connection::open(&sqlite_path)?;
        let relational = RelationalStorage::local_only(sqlite_path);
        relational
            .exec(&architecture_graph_sqlite_schema_sql())
            .await?;
        std::mem::forget(temp);
        Ok(relational)
    }

    #[tokio::test]
    async fn role_and_alias_round_trip_and_conflict_detection() -> Result<()> {
        let relational = sqlite_relational_with_schema().await?;
        let role = test_role();
        let persisted = upsert_role(&relational, &role).await?;
        assert_eq!(persisted, role);

        let loaded = load_role_by_canonical_key(&relational, "repo-1", "domain_owner")
            .await?
            .expect("role");
        assert_eq!(loaded, role);

        let alias = test_alias(&role.role_id, "Domain Owner");
        assert_eq!(create_role_alias(&relational, &alias).await?, Ok(()));

        let by_alias = load_role_by_alias(&relational, "repo-1", "domain owner")
            .await?
            .expect("role by alias");
        assert_eq!(by_alias.role_id, role.role_id);

        let conflicting = ArchitectureRoleAliasRecord {
            alias_id: "alias-conflict".to_string(),
            role_id: "role-2".to_string(),
            ..alias.clone()
        };
        assert_eq!(
            create_role_alias(&relational, &conflicting).await?,
            Err(AliasConflict::AlreadyAssignedToDifferentRole {
                alias: alias.alias_normalized,
                existing_role_id: role.role_id,
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn role_rule_assignment_proposal_and_migration_round_trip() -> Result<()> {
        let relational = sqlite_relational_with_schema().await?;
        let role = test_role();
        upsert_role(&relational, &role).await?;

        let rule = test_rule(&role.role_id);
        insert_role_rule(&relational, &rule).await?;
        let rules = load_role_rules(&relational, "repo-1", &role.role_id).await?;
        assert_eq!(rules, vec![rule.clone()]);

        let assignment = test_assignment(&role.role_id);
        insert_role_assignment(&relational, &assignment).await?;
        assert!(
            mark_assignment_invalidated(
                &relational,
                "repo-1",
                &assignment.assignment_id,
                "needs review after role change",
            )
            .await?
        );
        assert!(
            mark_assignment_migrated(
                &relational,
                "repo-1",
                &assignment.assignment_id,
                "assignment-2",
                Some("migration-1"),
            )
            .await?
        );

        let loaded_assignment =
            load_assignment_by_id(&relational, "repo-1", &assignment.assignment_id)
                .await?
                .expect("assignment");
        assert_eq!(loaded_assignment.status, "migrated");
        assert_eq!(
            loaded_assignment.migrated_to_assignment_id.as_deref(),
            Some("assignment-2")
        );

        let proposal = test_proposal();
        insert_role_proposal(&relational, &proposal).await?;
        let loaded_proposal =
            load_role_proposal_by_id(&relational, "repo-1", &proposal.proposal_id)
                .await?
                .expect("proposal");
        assert_eq!(loaded_proposal.preview_payload, proposal.preview_payload);

        assert!(
            mark_role_proposal_applied(
                &relational,
                "repo-1",
                &proposal.proposal_id,
                &json!({"applied": true}),
            )
            .await?
        );

        let migration = test_migration(&proposal.proposal_id);
        insert_assignment_migration_record(&relational, &migration).await?;
        let migrations =
            list_assignment_migrations_for_proposal(&relational, "repo-1", &proposal.proposal_id)
                .await?;
        assert_eq!(migrations, vec![migration]);
        Ok(())
    }
}
