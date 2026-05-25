use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::host::capability_host::gateways::RelationalGateway;
use crate::host::devql::RelationalStorage;

use super::storage::{
    ArchitectureRoleAssignmentMigrationRecord, load_role_by_id, load_role_proposal_by_id,
};
use super::taxonomy::{
    RoleCandidateSelector, RoleFactCondition, RoleRuleCandidateSelector, RoleRuleCondition,
    RoleSplitSpecFile, RuleSpecFile, parse_rule_conditions, parse_rule_score, parse_rule_selector,
    validate_role_split_spec, validate_rule_spec_file,
};

mod application;

pub use application::apply_proposal;

#[cfg(test)]
use super::storage::{
    ArchitectureRoleRecord, ArchitectureRoleRuleRecord, deterministic_role_id,
    deterministic_rule_id, insert_role_rule, load_current_assignment_by_id, load_role_by_alias,
    load_role_rules, next_role_rule_version, upsert_assignment, upsert_role,
};
#[cfg(test)]
use super::taxonomy;
#[cfg(test)]
use application::canonical_rule_hash;
use application::{
    persist_proposal, preview_alias_change, preview_role_change, preview_rule_lifecycle_change,
    preview_rule_spec, preview_split_role_change, resolve_role_ref, resolve_rule_ref,
};
#[cfg(test)]
use serde_json::json;

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

pub async fn preview_rule_activation_safety(
    relational: &RelationalStorage,
    repo_id: &str,
    rule_ref: &str,
) -> Result<Value> {
    let rule = resolve_rule_ref(relational, repo_id, rule_ref).await?;
    preview_rule_lifecycle_change(relational, &rule, "activate").await
}

pub async fn preview_existing_rule_match_safety(
    relational: &RelationalStorage,
    gateway: &dyn RelationalGateway,
    repo_id: &str,
    rule_ref: &str,
) -> Result<Value> {
    let rule = resolve_rule_ref(relational, repo_id, rule_ref).await?;
    let role = load_role_by_id(relational, repo_id, &rule.role_id)
        .await?
        .ok_or_else(|| anyhow!("role `{}` was not found", rule.role_id))?;
    let spec = RuleSpecFile {
        role_ref: role.canonical_key,
        candidate_selector: stored_rule_candidate_selector(&rule.candidate_selector)?,
        positive_conditions: stored_rule_conditions(&rule.positive_conditions)?,
        negative_conditions: stored_rule_conditions(&rule.negative_conditions)?,
        score: parse_rule_score(&rule.score)?,
        evidence: rule.evidence.clone(),
        metadata: rule.metadata.clone(),
    };
    preview_rule_spec(gateway, repo_id, &rule.role_id, &spec, Some(&rule)).await
}

fn stored_rule_candidate_selector(value: &Value) -> Result<RoleRuleCandidateSelector> {
    parse_rule_selector(value).or_else(|_| {
        let selector = serde_json::from_value::<RoleCandidateSelector>(value.clone())?;
        Ok(RoleRuleCandidateSelector {
            target_kinds: selector.target_kinds,
            path_prefixes: selector.path_prefixes,
            path_suffixes: selector.path_suffixes,
            required_facts: selector
                .required_facts
                .into_iter()
                .map(role_rule_condition_from_contract)
                .collect(),
            required_fact_any_groups: selector
                .required_fact_any_groups
                .into_iter()
                .map(|group| {
                    group
                        .into_iter()
                        .map(role_rule_condition_from_contract)
                        .collect()
                })
                .collect(),
            ..Default::default()
        })
    })
}

fn stored_rule_conditions(value: &Value) -> Result<Vec<RoleRuleCondition>> {
    parse_rule_conditions(value).or_else(|_| {
        let conditions = serde_json::from_value::<Vec<RoleFactCondition>>(value.clone())?;
        Ok(conditions
            .into_iter()
            .map(role_rule_condition_from_contract)
            .collect())
    })
}

fn role_rule_condition_from_contract(condition: RoleFactCondition) -> RoleRuleCondition {
    RoleRuleCondition {
        kind: condition.kind,
        key: Some(condition.key),
        op: Some(condition.op),
        value: Value::String(condition.value),
        score: Some(condition.score),
    }
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

#[cfg(test)]
mod tests;
