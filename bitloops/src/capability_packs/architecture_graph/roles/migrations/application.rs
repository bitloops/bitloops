use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

use crate::host::capability_host::gateways::RelationalGateway;
use crate::host::devql::RelationalStorage;

use super::{
    AliasRequest, DraftRuleRequest, EditRuleRequest, LifecycleRoleRequest, MergeRoleRequest,
    PROPOSAL_ACTIVATE_RULE, PROPOSAL_CREATE_ROLE_ALIAS, PROPOSAL_DEPRECATE_ROLE,
    PROPOSAL_DISABLE_RULE, PROPOSAL_DRAFT_RULE, PROPOSAL_EDIT_RULE, PROPOSAL_MERGE_ROLES,
    PROPOSAL_REMOVE_ROLE, PROPOSAL_RENAME_ROLE, PROPOSAL_SPLIT_ROLE, ProposalApplySummary,
    ProposalSummary, RenameRoleRequest, RuleRefRequest, SplitRoleRequest,
};
use crate::capability_packs::architecture_graph::roles::storage::{
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
use crate::capability_packs::architecture_graph::roles::taxonomy::{
    MatchableArtefact, RoleRuleCandidateSelector, RoleRuleCondition, RoleSplitSpecFile,
    RuleSpecFile, parse_rule_conditions, parse_rule_selector, role_rule_matches,
};

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

pub(super) async fn persist_proposal<T: Serialize>(
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

pub(super) async fn preview_role_change(
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

pub(super) async fn preview_split_role_change(
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

pub(super) async fn preview_alias_change(
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

pub(super) async fn preview_rule_spec(
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

pub(super) async fn preview_rule_lifecycle_change(
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

pub(super) async fn resolve_role_ref(
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

pub(super) async fn resolve_rule_ref(
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

pub(super) fn canonical_rule_hash(spec: &RuleSpecFile) -> Result<String> {
    let bytes = serde_json::to_vec(spec).context("serialise rule spec for hashing")?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn sha256_json(value: &Value) -> Result<String> {
    let bytes = serde_json::to_vec(value).context("serialise proposal payload for hashing")?;
    Ok(hex::encode(Sha256::digest(bytes)))
}
