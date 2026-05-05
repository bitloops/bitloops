use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

use crate::host::capability_host::gateways::RelationalGateway;
use crate::host::devql::RelationalStorage;

use super::storage::{
    ArchitectureRoleAliasRecord, ArchitectureRoleAssignmentMigrationRecord,
    ArchitectureRoleAssignmentRecord, ArchitectureRoleProposalRecord, ArchitectureRoleRecord,
    ArchitectureRoleRuleRecord, create_role_alias, deterministic_alias_id,
    deterministic_assignment_id, deterministic_migration_id, deterministic_proposal_id,
    deterministic_role_id, deterministic_rule_id, insert_assignment_migration_record,
    insert_role_assignment, insert_role_proposal, insert_role_rule,
    list_assignment_migrations_for_proposal, list_assignments_for_role, list_role_aliases,
    load_assignment_by_id, load_role_by_alias, load_role_by_canonical_key, load_role_by_id,
    load_role_proposal_by_id, load_role_rule_by_id, load_role_rules, mark_assignment_invalidated,
    mark_assignment_migrated, mark_role_proposal_applied, next_role_rule_version,
    normalize_role_alias, normalize_role_key, update_assignment_status, update_role_rule_lifecycle,
    upsert_role,
};
use super::taxonomy::{
    MatchableArtefact, RoleRuleCandidateSelector, RoleRuleCondition, RoleSplitSpecFile,
    RuleSpecFile, parse_rule_conditions, parse_rule_selector, role_rule_matches,
    validate_role_split_spec, validate_rule_spec_file,
};

const PROPOSAL_RENAME_ROLE: &str = "rename_role";
const PROPOSAL_DEPRECATE_ROLE: &str = "deprecate_role";
const PROPOSAL_REMOVE_ROLE: &str = "remove_role";
const PROPOSAL_MERGE_ROLES: &str = "merge_roles";
const PROPOSAL_SPLIT_ROLE: &str = "split_role";
const PROPOSAL_CREATE_ROLE_ALIAS: &str = "create_role_alias";
const PROPOSAL_DRAFT_RULE: &str = "draft_rule";
const PROPOSAL_EDIT_RULE: &str = "edit_rule";
const PROPOSAL_ACTIVATE_RULE: &str = "activate_rule";
const PROPOSAL_DISABLE_RULE: &str = "disable_rule";

#[derive(Debug, Clone, PartialEq)]
pub struct ProposalSummary {
    pub proposal_id: String,
    pub proposal_type: String,
    pub status: String,
    pub preview_payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProposalApplySummary {
    pub proposal_id: String,
    pub proposal_type: String,
    pub result_payload: Value,
    pub migration_records: Vec<ArchitectureRoleAssignmentMigrationRecord>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct RenameRoleRequest {
    role_id: String,
    new_display_name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct LifecycleRoleRequest {
    role_id: String,
    replacement_role_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MergeRoleRequest {
    source_role_id: String,
    target_role_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SplitRoleRequest {
    source_role_id: String,
    split_spec: RoleSplitSpecFile,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct AliasRequest {
    role_id: String,
    alias_key: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct DraftRuleRequest {
    role_id: String,
    spec: RuleSpecFile,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct EditRuleRequest {
    rule_id: String,
    role_id: String,
    spec: RuleSpecFile,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct RuleRefRequest {
    rule_id: String,
}

pub async fn create_rename_role_proposal(
    relational: &RelationalStorage,
    repo_id: &str,
    role_ref: &str,
    new_display_name: &str,
    provenance: Value,
) -> Result<ProposalSummary> {
    let role = resolve_role_ref(relational, repo_id, role_ref).await?;
    if new_display_name.trim().is_empty() {
        bail!("new display name must not be empty");
    }

    let request = RenameRoleRequest {
        role_id: role.role_id.clone(),
        new_display_name: new_display_name.trim().to_string(),
    };
    let preview = preview_role_change(relational, &role, "rename", None).await?;
    persist_proposal(
        relational,
        repo_id,
        PROPOSAL_RENAME_ROLE,
        &request,
        preview,
        provenance,
    )
    .await
}

pub async fn create_deprecate_role_proposal(
    relational: &RelationalStorage,
    repo_id: &str,
    role_ref: &str,
    replacement_role_ref: Option<&str>,
    provenance: Value,
) -> Result<ProposalSummary> {
    let role = resolve_role_ref(relational, repo_id, role_ref).await?;
    let replacement_role = match replacement_role_ref {
        Some(role_ref) => Some(resolve_role_ref(relational, repo_id, role_ref).await?),
        None => None,
    };
    let request = LifecycleRoleRequest {
        role_id: role.role_id.clone(),
        replacement_role_id: replacement_role.as_ref().map(|role| role.role_id.clone()),
    };
    let preview = preview_role_change(
        relational,
        &role,
        "deprecate",
        replacement_role.as_ref().map(|role| role.role_id.as_str()),
    )
    .await?;
    persist_proposal(
        relational,
        repo_id,
        PROPOSAL_DEPRECATE_ROLE,
        &request,
        preview,
        provenance,
    )
    .await
}

pub async fn create_remove_role_proposal(
    relational: &RelationalStorage,
    repo_id: &str,
    role_ref: &str,
    replacement_role_ref: Option<&str>,
    provenance: Value,
) -> Result<ProposalSummary> {
    let role = resolve_role_ref(relational, repo_id, role_ref).await?;
    let replacement_role = match replacement_role_ref {
        Some(role_ref) => Some(resolve_role_ref(relational, repo_id, role_ref).await?),
        None => None,
    };
    let request = LifecycleRoleRequest {
        role_id: role.role_id.clone(),
        replacement_role_id: replacement_role.as_ref().map(|role| role.role_id.clone()),
    };
    let preview = preview_role_change(
        relational,
        &role,
        "remove",
        replacement_role.as_ref().map(|role| role.role_id.as_str()),
    )
    .await?;
    persist_proposal(
        relational,
        repo_id,
        PROPOSAL_REMOVE_ROLE,
        &request,
        preview,
        provenance,
    )
    .await
}

pub async fn create_merge_role_proposal(
    relational: &RelationalStorage,
    repo_id: &str,
    source_role_ref: &str,
    target_role_ref: &str,
    provenance: Value,
) -> Result<ProposalSummary> {
    let source_role = resolve_role_ref(relational, repo_id, source_role_ref).await?;
    let target_role = resolve_role_ref(relational, repo_id, target_role_ref).await?;
    let request = MergeRoleRequest {
        source_role_id: source_role.role_id.clone(),
        target_role_id: target_role.role_id.clone(),
    };
    let preview = preview_role_change(
        relational,
        &source_role,
        "merge",
        Some(target_role.role_id.as_str()),
    )
    .await?;
    persist_proposal(
        relational,
        repo_id,
        PROPOSAL_MERGE_ROLES,
        &request,
        preview,
        provenance,
    )
    .await
}

pub async fn create_split_role_proposal(
    relational: &RelationalStorage,
    repo_id: &str,
    role_ref: &str,
    split_spec: RoleSplitSpecFile,
    provenance: Value,
) -> Result<ProposalSummary> {
    validate_role_split_spec(&split_spec)?;
    let role = resolve_role_ref(relational, repo_id, role_ref).await?;
    let request = SplitRoleRequest {
        source_role_id: role.role_id.clone(),
        split_spec: split_spec.clone(),
    };
    let preview = preview_split_role_change(relational, &role, &split_spec).await?;
    persist_proposal(
        relational,
        repo_id,
        PROPOSAL_SPLIT_ROLE,
        &request,
        preview,
        provenance,
    )
    .await
}

pub async fn create_alias_proposal(
    relational: &RelationalStorage,
    repo_id: &str,
    role_ref: &str,
    alias_key: &str,
    provenance: Value,
) -> Result<ProposalSummary> {
    let role = resolve_role_ref(relational, repo_id, role_ref).await?;
    let request = AliasRequest {
        role_id: role.role_id.clone(),
        alias_key: alias_key.trim().to_string(),
    };
    let preview = preview_alias_change(relational, &role, alias_key).await?;
    persist_proposal(
        relational,
        repo_id,
        PROPOSAL_CREATE_ROLE_ALIAS,
        &request,
        preview,
        provenance,
    )
    .await
}

pub async fn create_rule_draft_proposal(
    relational: &RelationalStorage,
    gateway: &dyn RelationalGateway,
    repo_id: &str,
    spec: RuleSpecFile,
    provenance: Value,
) -> Result<ProposalSummary> {
    validate_rule_spec_file(&spec)?;
    let role = resolve_role_ref(relational, repo_id, &spec.role_ref).await?;
    let preview = preview_rule_spec(gateway, repo_id, &role.role_id, &spec, None).await?;
    let request = DraftRuleRequest {
        role_id: role.role_id.clone(),
        spec,
    };
    persist_proposal(
        relational,
        repo_id,
        PROPOSAL_DRAFT_RULE,
        &request,
        preview,
        provenance,
    )
    .await
}

pub async fn create_rule_edit_proposal(
    relational: &RelationalStorage,
    gateway: &dyn RelationalGateway,
    repo_id: &str,
    rule_ref: &str,
    spec: RuleSpecFile,
    provenance: Value,
) -> Result<ProposalSummary> {
    validate_rule_spec_file(&spec)?;
    let existing_rule = resolve_rule_ref(relational, repo_id, rule_ref).await?;
    let preview = preview_rule_spec(
        gateway,
        repo_id,
        &existing_rule.role_id,
        &spec,
        Some(&existing_rule),
    )
    .await?;
    let request = EditRuleRequest {
        rule_id: existing_rule.rule_id.clone(),
        role_id: existing_rule.role_id.clone(),
        spec,
    };
    persist_proposal(
        relational,
        repo_id,
        PROPOSAL_EDIT_RULE,
        &request,
        preview,
        provenance,
    )
    .await
}

pub async fn create_rule_activate_proposal(
    relational: &RelationalStorage,
    repo_id: &str,
    rule_ref: &str,
    provenance: Value,
) -> Result<ProposalSummary> {
    let rule = resolve_rule_ref(relational, repo_id, rule_ref).await?;
    let preview = preview_rule_lifecycle_change(relational, &rule, "activate").await?;
    let request = RuleRefRequest {
        rule_id: rule.rule_id.clone(),
    };
    persist_proposal(
        relational,
        repo_id,
        PROPOSAL_ACTIVATE_RULE,
        &request,
        preview,
        provenance,
    )
    .await
}

pub async fn create_rule_disable_proposal(
    relational: &RelationalStorage,
    repo_id: &str,
    rule_ref: &str,
    provenance: Value,
) -> Result<ProposalSummary> {
    let rule = resolve_rule_ref(relational, repo_id, rule_ref).await?;
    let preview = preview_rule_lifecycle_change(relational, &rule, "disable").await?;
    let request = RuleRefRequest {
        rule_id: rule.rule_id.clone(),
    };
    persist_proposal(
        relational,
        repo_id,
        PROPOSAL_DISABLE_RULE,
        &request,
        preview,
        provenance,
    )
    .await
}

pub async fn show_proposal(
    relational: &RelationalStorage,
    repo_id: &str,
    proposal_id: &str,
) -> Result<ProposalSummary> {
    let proposal = load_role_proposal_by_id(relational, repo_id, proposal_id)
        .await?
        .ok_or_else(|| anyhow!("proposal `{proposal_id}` was not found"))?;
    Ok(ProposalSummary {
        proposal_id: proposal.proposal_id,
        proposal_type: proposal.proposal_type,
        status: proposal.status,
        preview_payload: proposal.preview_payload,
    })
}

pub async fn apply_proposal(
    relational: &RelationalStorage,
    repo_id: &str,
    proposal_id: &str,
) -> Result<ProposalApplySummary> {
    let proposal = load_role_proposal_by_id(relational, repo_id, proposal_id)
        .await?
        .ok_or_else(|| anyhow!("proposal `{proposal_id}` was not found"))?;
    if proposal.status == "applied" {
        let migrations =
            list_assignment_migrations_for_proposal(relational, repo_id, proposal_id).await?;
        return Ok(ProposalApplySummary {
            proposal_id: proposal.proposal_id,
            proposal_type: proposal.proposal_type,
            result_payload: proposal.result_payload,
            migration_records: migrations,
        });
    }

    let result_payload = match proposal.proposal_type.as_str() {
        PROPOSAL_RENAME_ROLE => {
            let request: RenameRoleRequest =
                serde_json::from_value(proposal.request_payload.clone())
                    .context("parse rename proposal request")?;
            apply_rename_role_proposal(relational, repo_id, proposal_id, &request).await?
        }
        PROPOSAL_DEPRECATE_ROLE => {
            let request: LifecycleRoleRequest =
                serde_json::from_value(proposal.request_payload.clone())
                    .context("parse deprecate proposal request")?;
            apply_lifecycle_role_proposal(
                relational,
                repo_id,
                proposal_id,
                &request,
                "deprecated",
                "needs_review",
            )
            .await?
        }
        PROPOSAL_REMOVE_ROLE => {
            let request: LifecycleRoleRequest =
                serde_json::from_value(proposal.request_payload.clone())
                    .context("parse remove proposal request")?;
            apply_lifecycle_role_proposal(
                relational,
                repo_id,
                proposal_id,
                &request,
                "removed",
                "stale",
            )
            .await?
        }
        PROPOSAL_MERGE_ROLES => {
            let request: MergeRoleRequest =
                serde_json::from_value(proposal.request_payload.clone())
                    .context("parse merge proposal request")?;
            apply_merge_role_proposal(relational, repo_id, proposal_id, &request).await?
        }
        PROPOSAL_SPLIT_ROLE => {
            let request: SplitRoleRequest =
                serde_json::from_value(proposal.request_payload.clone())
                    .context("parse split proposal request")?;
            apply_split_role_proposal(relational, repo_id, proposal_id, &request).await?
        }
        PROPOSAL_CREATE_ROLE_ALIAS => {
            let request: AliasRequest = serde_json::from_value(proposal.request_payload.clone())
                .context("parse alias proposal request")?;
            apply_alias_proposal(relational, repo_id, proposal_id, &request).await?
        }
        PROPOSAL_DRAFT_RULE => {
            let request: DraftRuleRequest =
                serde_json::from_value(proposal.request_payload.clone())
                    .context("parse draft-rule proposal request")?;
            apply_rule_draft_proposal(relational, repo_id, proposal_id, &request).await?
        }
        PROPOSAL_EDIT_RULE => {
            let request: EditRuleRequest = serde_json::from_value(proposal.request_payload.clone())
                .context("parse edit-rule proposal request")?;
            apply_rule_edit_proposal(relational, repo_id, proposal_id, &request).await?
        }
        PROPOSAL_ACTIVATE_RULE => {
            let request: RuleRefRequest = serde_json::from_value(proposal.request_payload.clone())
                .context("parse activate-rule proposal request")?;
            apply_rule_lifecycle_proposal(relational, repo_id, &request.rule_id, "active").await?
        }
        PROPOSAL_DISABLE_RULE => {
            let request: RuleRefRequest = serde_json::from_value(proposal.request_payload.clone())
                .context("parse disable-rule proposal request")?;
            apply_rule_lifecycle_proposal(relational, repo_id, &request.rule_id, "disabled").await?
        }
        other => bail!("unsupported proposal type `{other}`"),
    };

    mark_role_proposal_applied(relational, repo_id, proposal_id, &result_payload).await?;
    let migrations =
        list_assignment_migrations_for_proposal(relational, repo_id, proposal_id).await?;
    Ok(ProposalApplySummary {
        proposal_id: proposal.proposal_id,
        proposal_type: proposal.proposal_type,
        result_payload,
        migration_records: migrations,
    })
}

async fn persist_proposal<T: Serialize>(
    relational: &RelationalStorage,
    repo_id: &str,
    proposal_type: &str,
    request: &T,
    preview: Value,
    provenance: Value,
) -> Result<ProposalSummary> {
    let request_payload =
        serde_json::to_value(request).context("serialise architecture role proposal request")?;
    let request_hash = sha256_json(&request_payload)?;
    let proposal = ArchitectureRoleProposalRecord {
        proposal_id: deterministic_proposal_id(repo_id, proposal_type, &request_hash),
        repo_id: repo_id.to_string(),
        proposal_type: proposal_type.to_string(),
        status: "draft".to_string(),
        request_payload,
        preview_payload: preview.clone(),
        result_payload: json!({}),
        provenance,
        applied_at: None,
    };
    insert_role_proposal(relational, &proposal).await?;
    Ok(ProposalSummary {
        proposal_id: proposal.proposal_id,
        proposal_type: proposal.proposal_type,
        status: proposal.status,
        preview_payload: preview,
    })
}

async fn preview_role_change(
    relational: &RelationalStorage,
    role: &ArchitectureRoleRecord,
    operation: &str,
    replacement_role_id: Option<&str>,
) -> Result<Value> {
    let assignments = list_assignments_for_role(relational, &role.repo_id, &role.role_id).await?;
    let rules = load_role_rules(relational, &role.repo_id, &role.role_id).await?;
    let aliases = list_role_aliases(relational, &role.repo_id, &role.role_id).await?;
    let affected_artefacts = assignments
        .iter()
        .map(|assignment| assignment.artefact_id.clone())
        .collect::<BTreeSet<_>>();
    let affected_assignments = assignments
        .iter()
        .map(|assignment| assignment.assignment_id.clone())
        .collect::<Vec<_>>();
    let affected_rules = rules
        .iter()
        .map(|rule| rule.rule_id.clone())
        .collect::<Vec<_>>();
    let mut affected_roles = vec![role.role_id.clone()];
    if let Some(replacement_role_id) = replacement_role_id {
        affected_roles.push(replacement_role_id.to_string());
    }
    Ok(json!({
        "operation": operation,
        "role": {
            "role_id": role.role_id,
            "canonical_key": role.canonical_key,
            "display_name": role.display_name,
        },
        "replacement_role_id": replacement_role_id,
        "affected_role_ids": affected_roles,
        "affected_rule_ids": affected_rules,
        "affected_assignment_ids": affected_assignments,
        "affected_artefact_ids": affected_artefacts,
        "affected_roles": if replacement_role_id.is_some() { 2 } else { 1 },
        "affected_rules": rules.len(),
        "affected_assignments": assignments.len(),
        "affected_artefacts": assignments
            .iter()
            .map(|assignment| assignment.artefact_id.clone())
            .collect::<BTreeSet<_>>()
            .len(),
        "alias_count": aliases.len(),
        "downstream_review_work": {
            "reclassification_required": operation == "split",
            "safe_migration_available": replacement_role_id.is_some() || operation == "merge",
        }
    }))
}

async fn preview_split_role_change(
    relational: &RelationalStorage,
    role: &ArchitectureRoleRecord,
    split_spec: &RoleSplitSpecFile,
) -> Result<Value> {
    let assignments = list_assignments_for_role(relational, &role.repo_id, &role.role_id).await?;
    let rules = load_role_rules(relational, &role.repo_id, &role.role_id).await?;
    let affected_artefacts = assignments
        .iter()
        .map(|assignment| assignment.artefact_id.clone())
        .collect::<BTreeSet<_>>();
    Ok(json!({
        "operation": "split",
        "role": {
            "role_id": role.role_id,
            "canonical_key": role.canonical_key,
            "display_name": role.display_name,
        },
        "target_roles": split_spec.target_roles,
        "affected_role_ids": std::iter::once(role.role_id.clone())
            .chain(
                split_spec
                    .target_roles
                    .iter()
                    .map(|target| deterministic_role_id(&role.repo_id, &target.canonical_key)),
            )
            .collect::<Vec<_>>(),
        "affected_rule_ids": rules.iter().map(|rule| rule.rule_id.clone()).collect::<Vec<_>>(),
        "affected_assignment_ids": assignments
            .iter()
            .map(|assignment| assignment.assignment_id.clone())
            .collect::<Vec<_>>(),
        "affected_artefact_ids": affected_artefacts.clone(),
        "affected_roles": 1 + split_spec.target_roles.len(),
        "affected_rules": rules.len(),
        "affected_assignments": assignments.len(),
        "affected_artefacts": affected_artefacts.len(),
        "downstream_review_work": {
            "reclassification_required": true,
            "new_role_count": split_spec.target_roles.len(),
        }
    }))
}

async fn preview_alias_change(
    relational: &RelationalStorage,
    role: &ArchitectureRoleRecord,
    alias_key: &str,
) -> Result<Value> {
    let alias_normalized = normalize_role_alias(alias_key);
    let conflict = load_role_by_alias(relational, &role.repo_id, alias_key)
        .await?
        .is_some_and(|existing| existing.role_id != role.role_id);
    Ok(json!({
        "operation": "create_alias",
        "role": {
            "role_id": role.role_id,
            "canonical_key": role.canonical_key,
        },
        "alias_key": alias_key,
        "alias_normalized": alias_normalized,
        "conflict": conflict,
        "affected_role_ids": [role.role_id.clone()],
        "affected_rule_ids": [],
        "affected_assignment_ids": [],
        "affected_artefact_ids": [],
        "affected_roles": 1,
        "affected_rules": 0,
        "affected_assignments": 0,
        "affected_artefacts": 0,
        "downstream_review_work": {
            "reclassification_required": false,
        }
    }))
}

async fn preview_rule_spec(
    gateway: &dyn RelationalGateway,
    repo_id: &str,
    role_id: &str,
    spec: &RuleSpecFile,
    existing_rule: Option<&ArchitectureRoleRuleRecord>,
) -> Result<Value> {
    let artefacts = load_matchable_artefacts(gateway, repo_id)?;
    let new_matches = compute_rule_matches(
        &artefacts,
        &spec.candidate_selector,
        &spec.positive_conditions,
        &spec.negative_conditions,
    );
    let current_matches = if let Some(rule) = existing_rule {
        let selector = parse_rule_selector(&rule.candidate_selector)?;
        let positive = parse_rule_conditions(&rule.positive_conditions)?;
        let negative = parse_rule_conditions(&rule.negative_conditions)?;
        compute_rule_matches(&artefacts, &selector, &positive, &negative)
    } else {
        BTreeSet::new()
    };
    let added_matches = new_matches
        .difference(&current_matches)
        .cloned()
        .collect::<Vec<_>>();
    let removed_matches = current_matches
        .difference(&new_matches)
        .cloned()
        .collect::<Vec<_>>();
    let affected_artefact_ids = current_matches
        .union(&new_matches)
        .cloned()
        .collect::<Vec<_>>();
    Ok(json!({
        "operation": if existing_rule.is_some() { "edit_rule" } else { "draft_rule" },
        "affected_role_ids": [role_id],
        "affected_rule_ids": existing_rule
            .map(|rule| vec![rule.rule_id.clone()])
            .unwrap_or_default(),
        "affected_assignment_ids": current_matches.clone().into_iter().collect::<Vec<_>>(),
        "affected_artefact_ids": affected_artefact_ids.clone(),
        "affected_roles": 1,
        "affected_rules": if existing_rule.is_some() { 1 } else { 0 },
        "current_matches": current_matches,
        "new_matches": new_matches,
        "added_matches": added_matches,
        "removed_matches": removed_matches,
        "affected_assignments": current_matches.len() + added_matches.len(),
        "affected_artefacts": affected_artefact_ids.len(),
        "downstream_review_work": {
            "reclassification_required": !removed_matches.is_empty() || !added_matches.is_empty(),
        }
    }))
}

async fn preview_rule_lifecycle_change(
    relational: &RelationalStorage,
    rule: &ArchitectureRoleRuleRecord,
    operation: &str,
) -> Result<Value> {
    let assignments = list_assignments_for_role(relational, &rule.repo_id, &rule.role_id)
        .await?
        .into_iter()
        .filter(|assignment| assignment.rule_id.as_deref() == Some(rule.rule_id.as_str()))
        .collect::<Vec<_>>();
    Ok(json!({
        "operation": operation,
        "rule_id": rule.rule_id,
        "role_id": rule.role_id,
        "current_lifecycle_status": rule.lifecycle_status,
        "affected_role_ids": [rule.role_id.clone()],
        "affected_rule_ids": [rule.rule_id.clone()],
        "affected_assignment_ids": assignments
            .iter()
            .map(|assignment| assignment.assignment_id.clone())
            .collect::<Vec<_>>(),
        "affected_artefact_ids": assignments
            .iter()
            .map(|assignment| assignment.artefact_id.clone())
            .collect::<BTreeSet<_>>(),
        "affected_roles": 1,
        "affected_rules": 1,
        "affected_assignments": assignments.len(),
        "affected_artefacts": assignments.iter().map(|assignment| assignment.artefact_id.clone()).collect::<BTreeSet<_>>().len(),
        "downstream_review_work": {
            "reclassification_required": true,
        }
    }))
}

async fn apply_rename_role_proposal(
    relational: &RelationalStorage,
    repo_id: &str,
    _proposal_id: &str,
    request: &RenameRoleRequest,
) -> Result<Value> {
    let mut role = load_role_by_id(relational, repo_id, &request.role_id)
        .await?
        .ok_or_else(|| anyhow!("role `{}` was not found", request.role_id))?;
    let previous_display_name = role.display_name.clone();
    role.display_name = request.new_display_name.clone();
    let persisted = upsert_role(relational, &role).await?;
    let alias = ArchitectureRoleAliasRecord {
        alias_id: deterministic_alias_id(repo_id, &previous_display_name),
        repo_id: repo_id.to_string(),
        role_id: persisted.role_id.clone(),
        alias_key: previous_display_name.clone(),
        alias_normalized: normalize_role_alias(&previous_display_name),
        source_kind: "proposal_apply".to_string(),
        metadata: json!({"created_by": PROPOSAL_RENAME_ROLE}),
    };
    let _ = create_role_alias(relational, &alias).await?;
    Ok(json!({
        "role_id": persisted.role_id,
        "previous_display_name": previous_display_name,
        "new_display_name": persisted.display_name,
        "reclassification_required": false
    }))
}

async fn apply_lifecycle_role_proposal(
    relational: &RelationalStorage,
    repo_id: &str,
    proposal_id: &str,
    request: &LifecycleRoleRequest,
    lifecycle_status: &str,
    invalidated_status: &str,
) -> Result<Value> {
    let mut role = load_role_by_id(relational, repo_id, &request.role_id)
        .await?
        .ok_or_else(|| anyhow!("role `{}` was not found", request.role_id))?;
    let replacement_role = match request.replacement_role_id.as_deref() {
        Some(role_id) => Some(
            load_role_by_id(relational, repo_id, role_id)
                .await?
                .ok_or_else(|| anyhow!("replacement role `{role_id}` was not found"))?,
        ),
        None => None,
    };
    let assignments = list_assignments_for_role(relational, repo_id, &role.role_id).await?;
    role.lifecycle_status = lifecycle_status.to_string();
    upsert_role(relational, &role).await?;

    let mut migrated = 0usize;
    let mut invalidated = 0usize;
    if let Some(replacement_role) = replacement_role.as_ref() {
        for assignment in assignments {
            migrate_assignment_to_role(
                relational,
                repo_id,
                proposal_id,
                &assignment,
                replacement_role,
                lifecycle_status,
            )
            .await?;
            migrated += 1;
        }
    } else {
        for assignment in assignments {
            update_assignment_status(
                relational,
                repo_id,
                &assignment.assignment_id,
                invalidated_status,
                &format!("proposal applied: {lifecycle_status}"),
                None,
            )
            .await?;
            invalidated += 1;
        }
    }

    Ok(json!({
        "role_id": role.role_id,
        "lifecycle_status": lifecycle_status,
        "replacement_role_id": replacement_role.as_ref().map(|role| role.role_id.clone()),
        "migrated_assignments": migrated,
        "invalidated_assignments": invalidated,
        "reclassification_required": replacement_role.is_none(),
    }))
}

async fn apply_merge_role_proposal(
    relational: &RelationalStorage,
    repo_id: &str,
    proposal_id: &str,
    request: &MergeRoleRequest,
) -> Result<Value> {
    let source_role = load_role_by_id(relational, repo_id, &request.source_role_id)
        .await?
        .ok_or_else(|| anyhow!("source role `{}` was not found", request.source_role_id))?;
    let target_role = load_role_by_id(relational, repo_id, &request.target_role_id)
        .await?
        .ok_or_else(|| anyhow!("target role `{}` was not found", request.target_role_id))?;
    let assignments = list_assignments_for_role(relational, repo_id, &source_role.role_id).await?;
    for assignment in &assignments {
        migrate_assignment_to_role(
            relational,
            repo_id,
            proposal_id,
            assignment,
            &target_role,
            PROPOSAL_MERGE_ROLES,
        )
        .await?;
    }

    let alias = ArchitectureRoleAliasRecord {
        alias_id: deterministic_alias_id(repo_id, &source_role.canonical_key),
        repo_id: repo_id.to_string(),
        role_id: target_role.role_id.clone(),
        alias_key: source_role.canonical_key.clone(),
        alias_normalized: normalize_role_alias(&source_role.canonical_key),
        source_kind: "proposal_apply".to_string(),
        metadata: json!({"created_by": PROPOSAL_MERGE_ROLES}),
    };
    let _ = create_role_alias(relational, &alias).await?;

    let mut source_role_mut = source_role.clone();
    source_role_mut.lifecycle_status = "deprecated".to_string();
    upsert_role(relational, &source_role_mut).await?;

    Ok(json!({
        "source_role_id": source_role.role_id,
        "target_role_id": target_role.role_id,
        "migrated_assignments": assignments.len(),
        "reclassification_required": false,
    }))
}

async fn apply_split_role_proposal(
    relational: &RelationalStorage,
    repo_id: &str,
    proposal_id: &str,
    request: &SplitRoleRequest,
) -> Result<Value> {
    let source_role = load_role_by_id(relational, repo_id, &request.source_role_id)
        .await?
        .ok_or_else(|| anyhow!("source role `{}` was not found", request.source_role_id))?;
    let assignments = list_assignments_for_role(relational, repo_id, &source_role.role_id).await?;

    let mut created_roles = Vec::new();
    for target_role in &request.split_spec.target_roles {
        let role = ArchitectureRoleRecord {
            role_id: deterministic_role_id(repo_id, &target_role.canonical_key),
            repo_id: repo_id.to_string(),
            canonical_key: normalize_role_key(&target_role.canonical_key),
            display_name: target_role.display_name.clone(),
            description: target_role.description.clone(),
            family: target_role.family.clone(),
            lifecycle_status: "active".to_string(),
            provenance: json!({"source": PROPOSAL_SPLIT_ROLE}),
            evidence: json!([]),
            metadata: json!({"created_by": PROPOSAL_SPLIT_ROLE}),
        };
        let persisted = upsert_role(relational, &role).await?;
        created_roles.push(persisted.role_id.clone());
        for alias_key in &target_role.alias_keys {
            let alias = ArchitectureRoleAliasRecord {
                alias_id: deterministic_alias_id(repo_id, alias_key),
                repo_id: repo_id.to_string(),
                role_id: persisted.role_id.clone(),
                alias_key: alias_key.clone(),
                alias_normalized: normalize_role_alias(alias_key),
                source_kind: "proposal_apply".to_string(),
                metadata: json!({"created_by": PROPOSAL_SPLIT_ROLE}),
            };
            let _ = create_role_alias(relational, &alias).await?;
        }
    }

    let migration_id = deterministic_migration_id(repo_id, proposal_id, "split_role");
    for assignment in assignments.iter() {
        update_assignment_status(
            relational,
            repo_id,
            &assignment.assignment_id,
            "needs_review",
            "role split requires reclassification",
            Some(&migration_id),
        )
        .await?;
    }

    insert_assignment_migration_record(
        relational,
        &ArchitectureRoleAssignmentMigrationRecord {
            migration_id,
            repo_id: repo_id.to_string(),
            proposal_id: proposal_id.to_string(),
            migration_type: "split_role".to_string(),
            status: "applied".to_string(),
            source_role_id: Some(source_role.role_id.clone()),
            target_role_id: None,
            summary: json!({
                "invalidated_assignments": assignments.len(),
                "created_roles": created_roles,
                "note": request.split_spec.note,
            }),
        },
    )
    .await?;

    Ok(json!({
        "source_role_id": source_role.role_id,
        "created_roles": created_roles,
        "invalidated_assignments": assignments.len(),
        "reclassification_required": true,
    }))
}

async fn apply_alias_proposal(
    relational: &RelationalStorage,
    repo_id: &str,
    _proposal_id: &str,
    request: &AliasRequest,
) -> Result<Value> {
    let role = load_role_by_id(relational, repo_id, &request.role_id)
        .await?
        .ok_or_else(|| anyhow!("role `{}` was not found", request.role_id))?;
    let alias = ArchitectureRoleAliasRecord {
        alias_id: deterministic_alias_id(repo_id, &request.alias_key),
        repo_id: repo_id.to_string(),
        role_id: role.role_id.clone(),
        alias_key: request.alias_key.clone(),
        alias_normalized: normalize_role_alias(&request.alias_key),
        source_kind: "proposal_apply".to_string(),
        metadata: json!({"created_by": PROPOSAL_CREATE_ROLE_ALIAS}),
    };
    create_role_alias(relational, &alias)
        .await?
        .map_err(|err| anyhow!("alias conflict: {err:?}"))?;
    Ok(json!({
        "role_id": role.role_id,
        "alias_key": request.alias_key,
        "reclassification_required": false,
    }))
}

async fn apply_rule_draft_proposal(
    relational: &RelationalStorage,
    repo_id: &str,
    _proposal_id: &str,
    request: &DraftRuleRequest,
) -> Result<Value> {
    let version = next_role_rule_version(relational, repo_id, &request.role_id).await?;
    let canonical_hash = canonical_rule_hash(&request.spec)?;
    let rule = ArchitectureRoleRuleRecord {
        rule_id: deterministic_rule_id(repo_id, &request.role_id, version, &canonical_hash),
        repo_id: repo_id.to_string(),
        role_id: request.role_id.clone(),
        version,
        lifecycle_status: "draft".to_string(),
        canonical_hash,
        candidate_selector: serde_json::to_value(&request.spec.candidate_selector)?,
        positive_conditions: serde_json::to_value(&request.spec.positive_conditions)?,
        negative_conditions: serde_json::to_value(&request.spec.negative_conditions)?,
        score: serde_json::to_value(&request.spec.score)?,
        provenance: json!({"source": PROPOSAL_DRAFT_RULE}),
        evidence: request.spec.evidence.clone(),
        metadata: request.spec.metadata.clone(),
        supersedes_rule_id: None,
    };
    insert_role_rule(relational, &rule).await?;
    Ok(json!({
        "rule_id": rule.rule_id,
        "role_id": rule.role_id,
        "lifecycle_status": rule.lifecycle_status,
    }))
}

async fn apply_rule_edit_proposal(
    relational: &RelationalStorage,
    repo_id: &str,
    _proposal_id: &str,
    request: &EditRuleRequest,
) -> Result<Value> {
    let existing_rule = load_role_rule_by_id(relational, repo_id, &request.rule_id)
        .await?
        .ok_or_else(|| anyhow!("rule `{}` was not found", request.rule_id))?;
    let version = next_role_rule_version(relational, repo_id, &request.role_id).await?;
    let canonical_hash = canonical_rule_hash(&request.spec)?;
    let rule = ArchitectureRoleRuleRecord {
        rule_id: deterministic_rule_id(repo_id, &request.role_id, version, &canonical_hash),
        repo_id: repo_id.to_string(),
        role_id: request.role_id.clone(),
        version,
        lifecycle_status: "draft".to_string(),
        canonical_hash,
        candidate_selector: serde_json::to_value(&request.spec.candidate_selector)?,
        positive_conditions: serde_json::to_value(&request.spec.positive_conditions)?,
        negative_conditions: serde_json::to_value(&request.spec.negative_conditions)?,
        score: serde_json::to_value(&request.spec.score)?,
        provenance: json!({"source": PROPOSAL_EDIT_RULE}),
        evidence: request.spec.evidence.clone(),
        metadata: request.spec.metadata.clone(),
        supersedes_rule_id: Some(existing_rule.rule_id.clone()),
    };
    insert_role_rule(relational, &rule).await?;
    Ok(json!({
        "rule_id": rule.rule_id,
        "supersedes_rule_id": existing_rule.rule_id,
        "lifecycle_status": rule.lifecycle_status,
    }))
}

async fn apply_rule_lifecycle_proposal(
    relational: &RelationalStorage,
    repo_id: &str,
    rule_id: &str,
    lifecycle_status: &str,
) -> Result<Value> {
    let rule = load_role_rule_by_id(relational, repo_id, rule_id)
        .await?
        .ok_or_else(|| anyhow!("rule `{rule_id}` was not found"))?;
    update_role_rule_lifecycle(relational, repo_id, rule_id, lifecycle_status).await?;

    let assignments = list_assignments_for_role(relational, repo_id, &rule.role_id)
        .await?
        .into_iter()
        .filter(|assignment| assignment.rule_id.as_deref() == Some(rule_id))
        .collect::<Vec<_>>();
    let mut invalidated = 0usize;
    for assignment in &assignments {
        mark_assignment_invalidated(
            relational,
            repo_id,
            &assignment.assignment_id,
            &format!("rule lifecycle changed to {lifecycle_status}"),
        )
        .await?;
        invalidated += 1;
    }
    Ok(json!({
        "rule_id": rule.rule_id,
        "role_id": rule.role_id,
        "lifecycle_status": lifecycle_status,
        "invalidated_assignments": invalidated,
        "reclassification_required": true,
    }))
}

async fn migrate_assignment_to_role(
    relational: &RelationalStorage,
    repo_id: &str,
    proposal_id: &str,
    assignment: &ArchitectureRoleAssignmentRecord,
    target_role: &ArchitectureRoleRecord,
    migration_kind: &str,
) -> Result<()> {
    let new_assignment_id =
        deterministic_assignment_id(repo_id, &assignment.artefact_id, &target_role.role_id);
    let migration_token = format!("{proposal_id}|{}", assignment.assignment_id);
    let migration_id = deterministic_migration_id(repo_id, &migration_token, migration_kind);
    if load_assignment_by_id(relational, repo_id, &new_assignment_id)
        .await?
        .is_none()
    {
        insert_role_assignment(
            relational,
            &ArchitectureRoleAssignmentRecord {
                assignment_id: new_assignment_id.clone(),
                repo_id: repo_id.to_string(),
                artefact_id: assignment.artefact_id.clone(),
                role_id: target_role.role_id.clone(),
                source_kind: "proposal_migration".to_string(),
                confidence: assignment.confidence,
                status: "active".to_string(),
                status_reason: String::new(),
                rule_id: None,
                migration_id: Some(migration_id.clone()),
                migrated_to_assignment_id: None,
                provenance: json!({"source": migration_kind}),
                evidence: assignment.evidence.clone(),
                metadata: json!({
                    "migrated_from_assignment_id": assignment.assignment_id,
                    "source_rule_id": assignment.rule_id,
                }),
            },
        )
        .await?;
    }

    mark_assignment_migrated(
        relational,
        repo_id,
        &assignment.assignment_id,
        &new_assignment_id,
        Some(&migration_id),
    )
    .await?;
    insert_assignment_migration_record(
        relational,
        &ArchitectureRoleAssignmentMigrationRecord {
            migration_id,
            repo_id: repo_id.to_string(),
            proposal_id: proposal_id.to_string(),
            migration_type: migration_kind.to_string(),
            status: "applied".to_string(),
            source_role_id: Some(assignment.role_id.clone()),
            target_role_id: Some(target_role.role_id.clone()),
            summary: json!({
                "from_assignment_id": assignment.assignment_id,
                "to_assignment_id": new_assignment_id,
                "artefact_id": assignment.artefact_id,
            }),
        },
    )
    .await?;
    Ok(())
}

async fn resolve_role_ref(
    relational: &RelationalStorage,
    repo_id: &str,
    role_ref: &str,
) -> Result<ArchitectureRoleRecord> {
    let stripped = role_ref
        .trim()
        .strip_prefix("role:")
        .unwrap_or(role_ref.trim());
    if let Some(role) = load_role_by_id(relational, repo_id, stripped).await? {
        return Ok(role);
    }
    if let Some(role) = load_role_by_canonical_key(relational, repo_id, stripped).await? {
        return Ok(role);
    }
    if let Some(role) = load_role_by_alias(relational, repo_id, stripped).await? {
        return Ok(role);
    }
    bail!("role reference `{role_ref}` was not found")
}

async fn resolve_rule_ref(
    relational: &RelationalStorage,
    repo_id: &str,
    rule_ref: &str,
) -> Result<ArchitectureRoleRuleRecord> {
    let stripped = rule_ref
        .trim()
        .strip_prefix("rule:")
        .unwrap_or(rule_ref.trim());
    load_role_rule_by_id(relational, repo_id, stripped)
        .await?
        .ok_or_else(|| anyhow!("rule reference `{rule_ref}` was not found"))
}

fn load_matchable_artefacts(
    gateway: &dyn RelationalGateway,
    repo_id: &str,
) -> Result<Vec<MatchableArtefact>> {
    gateway
        .load_current_canonical_artefacts(repo_id)?
        .into_iter()
        .map(|artefact| {
            Ok(MatchableArtefact {
                artefact_id: artefact.artefact_id,
                path: artefact.path,
                language: Some(artefact.language),
                canonical_kind: artefact.canonical_kind,
                symbol_fqn: artefact.symbol_fqn,
            })
        })
        .collect()
}

fn compute_rule_matches(
    artefacts: &[MatchableArtefact],
    selector: &RoleRuleCandidateSelector,
    positive_conditions: &[RoleRuleCondition],
    negative_conditions: &[RoleRuleCondition],
) -> BTreeSet<String> {
    artefacts
        .iter()
        .filter(|artefact| {
            role_rule_matches(selector, positive_conditions, negative_conditions, artefact)
        })
        .map(|artefact| artefact.artefact_id.clone())
        .collect()
}

fn canonical_rule_hash(spec: &RuleSpecFile) -> Result<String> {
    let bytes = serde_json::to_vec(spec).context("serialise rule spec for hashing")?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn sha256_json(value: &Value) -> Result<String> {
    let bytes = serde_json::to_vec(value).context("serialise proposal payload for hashing")?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability_packs::architecture_graph::schema::architecture_graph_sqlite_schema_sql;
    use crate::models::{CurrentCanonicalArtefactRecord, ProductionArtefact};

    struct FakeRelationalGateway {
        artefacts: Vec<CurrentCanonicalArtefactRecord>,
    }

    impl RelationalGateway for FakeRelationalGateway {
        fn resolve_checkpoint_id(&self, _repo_id: &str, _checkpoint_ref: &str) -> Result<String> {
            bail!("not used")
        }

        fn artefact_exists(&self, _repo_id: &str, _artefact_id: &str) -> Result<bool> {
            bail!("not used")
        }

        fn load_repo_id_for_commit(&self, _commit_sha: &str) -> Result<String> {
            bail!("not used")
        }

        fn load_current_canonical_artefacts(
            &self,
            _repo_id: &str,
        ) -> Result<Vec<CurrentCanonicalArtefactRecord>> {
            Ok(self.artefacts.clone())
        }

        fn load_current_production_artefacts(
            &self,
            _repo_id: &str,
        ) -> Result<Vec<ProductionArtefact>> {
            bail!("not used")
        }

        fn load_production_artefacts(&self, _commit_sha: &str) -> Result<Vec<ProductionArtefact>> {
            bail!("not used")
        }

        fn load_artefacts_for_file_lines(
            &self,
            _commit_sha: &str,
            _file_path: &str,
        ) -> Result<Vec<(String, i64, i64)>> {
            bail!("not used")
        }
    }

    fn gateway() -> FakeRelationalGateway {
        FakeRelationalGateway {
            artefacts: vec![
                CurrentCanonicalArtefactRecord {
                    repo_id: "repo-1".to_string(),
                    path: "src/cli/commands/run.rs".to_string(),
                    content_id: "content-1".to_string(),
                    symbol_id: "symbol-1".to_string(),
                    artefact_id: "artefact-1".to_string(),
                    language: "rust".to_string(),
                    extraction_fingerprint: "fingerprint".to_string(),
                    canonical_kind: Some("function".to_string()),
                    language_kind: Some("function".to_string()),
                    symbol_fqn: Some("crate::cli::commands::run".to_string()),
                    parent_symbol_id: None,
                    parent_artefact_id: None,
                    start_line: 1,
                    end_line: 10,
                    start_byte: 0,
                    end_byte: 50,
                    signature: Some("fn run()".to_string()),
                    modifiers: "[]".to_string(),
                    docstring: None,
                },
                CurrentCanonicalArtefactRecord {
                    repo_id: "repo-1".to_string(),
                    path: "src/domain/payments.rs".to_string(),
                    content_id: "content-2".to_string(),
                    symbol_id: "symbol-2".to_string(),
                    artefact_id: "artefact-2".to_string(),
                    language: "rust".to_string(),
                    extraction_fingerprint: "fingerprint".to_string(),
                    canonical_kind: Some("struct".to_string()),
                    language_kind: Some("struct".to_string()),
                    symbol_fqn: Some("crate::domain::payments".to_string()),
                    parent_symbol_id: None,
                    parent_artefact_id: None,
                    start_line: 1,
                    end_line: 10,
                    start_byte: 0,
                    end_byte: 50,
                    signature: Some("struct Payments".to_string()),
                    modifiers: "[]".to_string(),
                    docstring: None,
                },
            ],
        }
    }

    async fn relational() -> Result<RelationalStorage> {
        let temp = tempfile::tempdir()?;
        let sqlite_path = temp.path().join("roles.sqlite");
        rusqlite::Connection::open(&sqlite_path)?;
        let relational = RelationalStorage::local_only(sqlite_path);
        relational
            .exec(&architecture_graph_sqlite_schema_sql())
            .await?;
        std::mem::forget(temp);
        Ok(relational)
    }

    async fn seed_role(relational: &RelationalStorage) -> Result<ArchitectureRoleRecord> {
        upsert_role(
            relational,
            &ArchitectureRoleRecord {
                role_id: deterministic_role_id("repo-1", "command_dispatcher"),
                repo_id: "repo-1".to_string(),
                canonical_key: "command_dispatcher".to_string(),
                display_name: "Command Dispatcher".to_string(),
                description: "Routes commands".to_string(),
                family: Some("entrypoint".to_string()),
                lifecycle_status: "active".to_string(),
                provenance: json!({"source": "test"}),
                evidence: json!([]),
                metadata: json!({}),
            },
        )
        .await
    }

    async fn seed_role_with_key(
        relational: &RelationalStorage,
        canonical_key: &str,
        display_name: &str,
    ) -> Result<ArchitectureRoleRecord> {
        upsert_role(
            relational,
            &ArchitectureRoleRecord {
                role_id: deterministic_role_id("repo-1", canonical_key),
                repo_id: "repo-1".to_string(),
                canonical_key: canonical_key.to_string(),
                display_name: display_name.to_string(),
                description: format!("role {display_name}"),
                family: Some("entrypoint".to_string()),
                lifecycle_status: "active".to_string(),
                provenance: json!({"source": "test"}),
                evidence: json!([]),
                metadata: json!({}),
            },
        )
        .await
    }

    async fn seed_assignment(
        relational: &RelationalStorage,
        artefact_id: &str,
        role_id: &str,
    ) -> Result<String> {
        seed_assignment_with_rule(relational, artefact_id, role_id, None).await
    }

    async fn seed_assignment_with_rule(
        relational: &RelationalStorage,
        artefact_id: &str,
        role_id: &str,
        rule_id: Option<&str>,
    ) -> Result<String> {
        let assignment_id = deterministic_assignment_id("repo-1", artefact_id, role_id);
        insert_role_assignment(
            relational,
            &ArchitectureRoleAssignmentRecord {
                assignment_id: assignment_id.clone(),
                repo_id: "repo-1".to_string(),
                artefact_id: artefact_id.to_string(),
                role_id: role_id.to_string(),
                source_kind: "seed".to_string(),
                confidence: 0.9,
                status: "active".to_string(),
                status_reason: String::new(),
                rule_id: rule_id.map(ToOwned::to_owned),
                migration_id: None,
                migrated_to_assignment_id: None,
                provenance: json!({"source": "test"}),
                evidence: json!([]),
                metadata: json!({}),
            },
        )
        .await?;
        Ok(assignment_id)
    }

    async fn seed_rule(
        relational: &RelationalStorage,
        role_id: &str,
        selector: RoleRuleCandidateSelector,
    ) -> Result<ArchitectureRoleRuleRecord> {
        let version = next_role_rule_version(relational, "repo-1", role_id).await?;
        let spec = RuleSpecFile {
            role_ref: role_id.to_string(),
            candidate_selector: selector,
            positive_conditions: vec![],
            negative_conditions: vec![],
            score: super::super::taxonomy::RoleRuleScore {
                base_confidence: Some(0.8),
                weight: None,
            },
            evidence: json!([]),
            metadata: json!({}),
        };
        let canonical_hash = canonical_rule_hash(&spec)?;
        let record = ArchitectureRoleRuleRecord {
            rule_id: deterministic_rule_id("repo-1", role_id, version, &canonical_hash),
            repo_id: "repo-1".to_string(),
            role_id: role_id.to_string(),
            version,
            lifecycle_status: "active".to_string(),
            canonical_hash,
            candidate_selector: serde_json::to_value(&spec.candidate_selector)?,
            positive_conditions: serde_json::to_value(&spec.positive_conditions)?,
            negative_conditions: serde_json::to_value(&spec.negative_conditions)?,
            score: serde_json::to_value(&spec.score)?,
            provenance: json!({"source": "test"}),
            evidence: json!([]),
            metadata: json!({}),
            supersedes_rule_id: None,
        };
        insert_role_rule(relational, &record).await?;
        Ok(record)
    }

    #[tokio::test]
    async fn rename_proposal_preview_and_apply_keeps_role_identity() -> Result<()> {
        let relational = relational().await?;
        let role = seed_role(&relational).await?;

        let proposal = create_rename_role_proposal(
            &relational,
            "repo-1",
            &role.canonical_key,
            "CLI Command Dispatcher",
            json!({"source": "test"}),
        )
        .await?;
        assert_eq!(proposal.proposal_type, PROPOSAL_RENAME_ROLE);
        assert_eq!(proposal.preview_payload["affected_roles"], json!(1));

        let applied = apply_proposal(&relational, "repo-1", &proposal.proposal_id).await?;
        assert_eq!(
            applied.result_payload["new_display_name"],
            json!("CLI Command Dispatcher")
        );

        let loaded = load_role_by_id(&relational, "repo-1", &role.role_id)
            .await?
            .expect("role");
        assert_eq!(loaded.role_id, role.role_id);
        assert_eq!(loaded.display_name, "CLI Command Dispatcher");
        Ok(())
    }

    #[tokio::test]
    async fn deprecate_and_remove_proposals_invalidate_or_migrate_assignments() -> Result<()> {
        let deprecate_relational = relational().await?;
        let role = seed_role(&deprecate_relational).await?;
        let assignment_id =
            seed_assignment(&deprecate_relational, "artefact-1", &role.role_id).await?;

        let deprecate = create_deprecate_role_proposal(
            &deprecate_relational,
            "repo-1",
            &role.canonical_key,
            None,
            json!({"source": "test"}),
        )
        .await?;
        assert_eq!(deprecate.preview_payload["affected_assignments"], json!(1));
        let deprecated =
            apply_proposal(&deprecate_relational, "repo-1", &deprecate.proposal_id).await?;
        assert_eq!(
            deprecated.result_payload["invalidated_assignments"],
            json!(1)
        );
        let invalidated = load_assignment_by_id(&deprecate_relational, "repo-1", &assignment_id)
            .await?
            .expect("assignment");
        assert_eq!(invalidated.status, "needs_review");

        let remove_relational = relational().await?;
        let source = seed_role(&remove_relational).await?;
        let target = seed_role_with_key(&remove_relational, "web_ui", "Web UI").await?;
        let assignment_id =
            seed_assignment(&remove_relational, "artefact-1", &source.role_id).await?;
        let remove = create_remove_role_proposal(
            &remove_relational,
            "repo-1",
            &source.canonical_key,
            Some(&target.canonical_key),
            json!({"source": "test"}),
        )
        .await?;
        let removed = apply_proposal(&remove_relational, "repo-1", &remove.proposal_id).await?;
        assert_eq!(removed.result_payload["migrated_assignments"], json!(1));
        let migrated = load_assignment_by_id(&remove_relational, "repo-1", &assignment_id)
            .await?
            .expect("migrated assignment");
        assert_eq!(migrated.status, "migrated");
        assert_eq!(removed.migration_records.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn split_preview_reports_reclassification_and_rule_edit_preview_reports_diff()
    -> Result<()> {
        let relational = relational().await?;
        let role = seed_role(&relational).await?;
        seed_assignment(&relational, "artefact-1", &role.role_id).await?;

        let split = create_split_role_proposal(
            &relational,
            "repo-1",
            &role.canonical_key,
            RoleSplitSpecFile {
                target_roles: vec![crate::capability_packs::architecture_graph::roles::taxonomy::RoleSplitTargetRole {
                    canonical_key: "cli_command".to_string(),
                    display_name: "CLI Command".to_string(),
                    description: String::new(),
                    family: Some("entrypoint".to_string()),
                    alias_keys: vec![],
                }],
                note: Some("split command surface".to_string()),
            },
            json!({"source": "test"}),
        )
        .await?;
        assert_eq!(
            split.preview_payload["downstream_review_work"]["reclassification_required"],
            json!(true)
        );
        assert_eq!(split.preview_payload["affected_rules"], json!(0));

        let existing_rule = seed_rule(
            &relational,
            &role.role_id,
            RoleRuleCandidateSelector {
                path_prefixes: vec!["src/cli".to_string()],
                ..Default::default()
            },
        )
        .await?;

        let rule_preview = create_rule_edit_proposal(
            &relational,
            &gateway(),
            "repo-1",
            &existing_rule.rule_id,
            RuleSpecFile {
                role_ref: role.canonical_key.clone(),
                candidate_selector: RoleRuleCandidateSelector {
                    path_prefixes: vec!["src/domain".to_string()],
                    ..Default::default()
                },
                positive_conditions: vec![],
                negative_conditions: vec![],
                score: super::super::taxonomy::RoleRuleScore {
                    base_confidence: Some(0.8),
                    weight: None,
                },
                evidence: json!([]),
                metadata: json!({}),
            },
            json!({"source": "test"}),
        )
        .await?;
        assert_eq!(
            rule_preview.preview_payload["added_matches"],
            json!(["artefact-2"])
        );
        assert_eq!(
            rule_preview.preview_payload["removed_matches"],
            json!(["artefact-1"])
        );
        Ok(())
    }

    #[tokio::test]
    async fn merge_preview_counts_impacted_work_and_apply_creates_auditable_migration() -> Result<()>
    {
        let relational = relational().await?;
        let source = seed_role(&relational).await?;
        let target = seed_role_with_key(&relational, "web_ui", "Web UI").await?;
        let assignment_id = seed_assignment(&relational, "artefact-1", &source.role_id).await?;

        let proposal = create_merge_role_proposal(
            &relational,
            "repo-1",
            &source.canonical_key,
            &target.canonical_key,
            json!({"source": "test"}),
        )
        .await?;
        assert_eq!(proposal.preview_payload["affected_roles"], json!(2));
        assert_eq!(proposal.preview_payload["affected_assignments"], json!(1));
        assert_eq!(
            proposal.preview_payload["downstream_review_work"]["safe_migration_available"],
            json!(true)
        );

        let applied = apply_proposal(&relational, "repo-1", &proposal.proposal_id).await?;
        assert_eq!(applied.result_payload["migrated_assignments"], json!(1));
        assert_eq!(applied.migration_records.len(), 1);
        let migrated = load_assignment_by_id(&relational, "repo-1", &assignment_id)
            .await?
            .expect("source assignment");
        assert_eq!(migrated.status, "migrated");
        Ok(())
    }

    #[tokio::test]
    async fn alias_create_and_split_apply_workflows_persist_reviewable_changes() -> Result<()> {
        let alias_relational = relational().await?;
        let role = seed_role(&alias_relational).await?;

        let alias = create_alias_proposal(
            &alias_relational,
            "repo-1",
            &role.canonical_key,
            "cli_surface",
            json!({"source": "test"}),
        )
        .await?;
        let applied_alias = apply_proposal(&alias_relational, "repo-1", &alias.proposal_id).await?;
        assert_eq!(
            applied_alias.result_payload["alias_key"],
            json!("cli_surface")
        );
        let resolved = load_role_by_alias(&alias_relational, "repo-1", "cli_surface")
            .await?
            .expect("alias role");
        assert_eq!(resolved.role_id, role.role_id);

        let split_relational = relational().await?;
        let split_role = seed_role(&split_relational).await?;
        let assignment_id =
            seed_assignment(&split_relational, "artefact-1", &split_role.role_id).await?;
        let split = create_split_role_proposal(
            &split_relational,
            "repo-1",
            &split_role.canonical_key,
            RoleSplitSpecFile {
                target_roles: vec![crate::capability_packs::architecture_graph::roles::taxonomy::RoleSplitTargetRole {
                    canonical_key: "cli_command".to_string(),
                    display_name: "CLI Command".to_string(),
                    description: String::new(),
                    family: Some("entrypoint".to_string()),
                    alias_keys: vec!["command_surface".to_string()],
                }],
                note: Some("split command surface".to_string()),
            },
            json!({"source": "test"}),
        )
        .await?;
        let applied_split = apply_proposal(&split_relational, "repo-1", &split.proposal_id).await?;
        assert_eq!(applied_split.migration_records.len(), 1);
        let invalidated = load_assignment_by_id(&split_relational, "repo-1", &assignment_id)
            .await?
            .expect("split assignment");
        assert_eq!(invalidated.status, "needs_review");
        Ok(())
    }

    #[tokio::test]
    async fn rule_disable_only_invalidates_assignments_linked_to_that_rule() -> Result<()> {
        let relational = relational().await?;
        let role = seed_role(&relational).await?;
        let rule = seed_rule(
            &relational,
            &role.role_id,
            RoleRuleCandidateSelector {
                path_prefixes: vec!["src/cli".to_string()],
                ..Default::default()
            },
        )
        .await?;
        let rule_assignment_id = seed_assignment_with_rule(
            &relational,
            "artefact-1",
            &role.role_id,
            Some(&rule.rule_id),
        )
        .await?;
        let manual_assignment_id =
            seed_assignment_with_rule(&relational, "artefact-2", &role.role_id, None).await?;

        let disable = create_rule_disable_proposal(
            &relational,
            "repo-1",
            &rule.rule_id,
            json!({"source": "test"}),
        )
        .await?;
        assert_eq!(disable.preview_payload["affected_assignments"], json!(1));

        let applied = apply_proposal(&relational, "repo-1", &disable.proposal_id).await?;
        assert_eq!(applied.result_payload["invalidated_assignments"], json!(1));
        let invalidated = load_assignment_by_id(&relational, "repo-1", &rule_assignment_id)
            .await?
            .expect("rule assignment");
        assert_eq!(invalidated.status, "needs_review");
        let untouched = load_assignment_by_id(&relational, "repo-1", &manual_assignment_id)
            .await?
            .expect("manual assignment");
        assert_eq!(untouched.status, "active");
        Ok(())
    }
}
