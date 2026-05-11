use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::capability_packs::architecture_graph::roles::llm_adjudication::{
    collect_seed_evidence, run_seed_generation,
};
use crate::capability_packs::architecture_graph::roles::migrations::{
    apply_proposal, create_rule_activate_proposal,
};
use crate::capability_packs::architecture_graph::roles::storage::{
    AliasConflict, ArchitectureRoleAliasRecord, ArchitectureRoleRecord, ArchitectureRoleRuleRecord,
    create_role_alias, deterministic_alias_id, deterministic_role_id, deterministic_rule_id,
    insert_role_rule, load_role_by_alias, load_role_by_canonical_key, load_role_rules,
    next_role_rule_version, normalize_role_alias, normalize_role_key, upsert_role,
};
use crate::capability_packs::architecture_graph::roles::taxonomy::{
    SeededArchitectureRuleCandidate, SeededArchitectureTaxonomy,
    role_rule_candidate_selector_contract, role_rule_conditions_contract,
};
use crate::config::InferenceTask;
use crate::host::capability_host::{CurrentStateConsumerContext, DevqlCapabilityHost};
use crate::host::inference::InferenceGateway;

use super::super::SlimCliRepoScope;
use super::{RolesClassifyOutput, format_roles_classify_output};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct SeedSummary {
    pub(super) profile_name: String,
    pub(super) roles_total: usize,
    pub(super) roles_created: usize,
    pub(super) roles_reused: usize,
    pub(super) rules_total: usize,
    pub(super) rules_created: usize,
    pub(super) rules_reused: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct SeedRuleActivationSummary {
    pub(super) seed_owned_draft_rules: usize,
    pub(super) proposals_created: usize,
    pub(super) proposals_applied: usize,
    pub(super) activated_rule_ids: Vec<String>,
    pub(super) proposal_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct SeedCommandSummary {
    pub(super) seed: SeedSummary,
    pub(super) rule_activation: Option<SeedRuleActivationSummary>,
    pub(super) classification: Option<RolesClassifyOutput>,
}

pub(super) async fn seed_architecture_roles(
    scope: &SlimCliRepoScope,
    host: &DevqlCapabilityHost,
    context: &CurrentStateConsumerContext,
) -> Result<SeedSummary> {
    let profile_name =
        configured_seed_profile_name(host.config_view("architecture_graph").scoped())?;

    let resolved = host
        .inference_for_capability("architecture_graph")
        .describe("fact_synthesis")
        .ok_or_else(|| {
            anyhow!(
                "The configured architecture fact_synthesis slot is unresolved. Check `architecture.inference.fact_synthesis = \"{}\"`.",
                profile_name
            )
        })?;
    if resolved.task != Some(InferenceTask::StructuredGeneration) {
        bail!(
            "Architecture seed requires a `structured_generation` profile, but `{}` is configured with task `{}`.",
            profile_name,
            resolved
                .task
                .map(|task| task.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
    }

    let service = host
        .inference_for_capability("architecture_graph")
        .structured_generation("fact_synthesis")
        .with_context(|| {
            format!("resolving architecture fact_synthesis inference slot `{profile_name}`")
        })?;
    let evidence = collect_seed_evidence(scope, context).await?;
    let taxonomy = run_seed_generation(service.as_ref(), scope, &evidence)?;
    persist_seeded_taxonomy(
        context.storage.as_ref(),
        &scope.repo.repo_id,
        &profile_name,
        taxonomy,
    )
    .await
}

pub(super) async fn persist_seeded_taxonomy(
    relational: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    profile_name: &str,
    taxonomy: SeededArchitectureTaxonomy,
) -> Result<SeedSummary> {
    let roles_total = taxonomy.roles.len();
    let rules_total = taxonomy.rule_candidates.len();
    let mut roles_created = 0usize;
    let mut roles_reused = 0usize;
    let mut rules_created = 0usize;
    let mut rules_reused = 0usize;
    let mut persisted_role_ids = std::collections::BTreeMap::new();

    for seeded_role in taxonomy.roles {
        let canonical_key = normalize_role_key(&seeded_role.canonical_key);
        let existing = if let Some(role) =
            load_role_by_canonical_key(relational, repo_id, &canonical_key).await?
        {
            Some(role)
        } else if let Some(role) = load_role_by_alias(relational, repo_id, &canonical_key).await? {
            Some(role)
        } else {
            load_role_by_alias(relational, repo_id, &seeded_role.display_name).await?
        };
        let role = ArchitectureRoleRecord {
            role_id: existing
                .as_ref()
                .map(|role| role.role_id.clone())
                .unwrap_or_else(|| deterministic_role_id(repo_id, &canonical_key)),
            repo_id: repo_id.to_string(),
            canonical_key: existing
                .as_ref()
                .map(|role| role.canonical_key.clone())
                .unwrap_or_else(|| canonical_key.clone()),
            display_name: seeded_role.display_name.clone(),
            description: seeded_role.description.clone(),
            family: seeded_role.family.clone(),
            lifecycle_status: seeded_role
                .lifecycle_status
                .clone()
                .unwrap_or_else(|| "active".to_string()),
            provenance: merge_provenance(
                seeded_role.provenance,
                json!({
                    "seeded_by_profile": profile_name,
                    "source": "architecture_roles_seed",
                }),
            ),
            evidence: seeded_role.evidence,
            metadata: json!({}),
        };
        let persisted = upsert_role(relational, &role).await?;
        if existing.is_some() {
            roles_reused += 1;
        } else {
            roles_created += 1;
        }
        if persisted.canonical_key != canonical_key {
            ensure_seed_alias(
                relational,
                &ArchitectureRoleAliasRecord {
                    alias_id: deterministic_alias_id(repo_id, &canonical_key),
                    repo_id: repo_id.to_string(),
                    role_id: persisted.role_id.clone(),
                    alias_key: canonical_key.clone(),
                    alias_normalized: normalize_role_alias(&canonical_key),
                    source_kind: "seed".to_string(),
                    metadata: json!({"seed_profile": profile_name}),
                },
            )
            .await?;
        }
        let display_alias = persisted.display_name.clone();
        ensure_seed_alias(
            relational,
            &ArchitectureRoleAliasRecord {
                alias_id: deterministic_alias_id(repo_id, &display_alias),
                repo_id: repo_id.to_string(),
                role_id: persisted.role_id.clone(),
                alias_key: display_alias.clone(),
                alias_normalized: normalize_role_alias(&display_alias),
                source_kind: "seed".to_string(),
                metadata: json!({"seed_profile": profile_name}),
            },
        )
        .await?;
        persisted_role_ids.insert(canonical_key, persisted.role_id);
    }

    for candidate in taxonomy.rule_candidates {
        let role_key = normalize_role_key(&candidate.target_role_key);
        let role_id = persisted_role_ids
            .get(&role_key)
            .cloned()
            .ok_or_else(|| anyhow!("seeded rule candidate referenced unknown role `{role_key}`"))?;
        let canonical_hash = seed_rule_hash(&role_id, &candidate)?;
        let existing_rules = load_role_rules(relational, repo_id, &role_id).await?;
        if existing_rules
            .iter()
            .any(|rule| rule.canonical_hash == canonical_hash)
        {
            rules_reused += 1;
            continue;
        }
        let version = next_role_rule_version(relational, repo_id, &role_id).await?;
        let rule = ArchitectureRoleRuleRecord {
            rule_id: deterministic_rule_id(repo_id, &role_id, version, &canonical_hash),
            repo_id: repo_id.to_string(),
            role_id,
            version,
            lifecycle_status: "draft".to_string(),
            canonical_hash,
            candidate_selector: serde_json::to_value(role_rule_candidate_selector_contract(
                &candidate.candidate_selector,
            ))?,
            positive_conditions: serde_json::to_value(role_rule_conditions_contract(
                &candidate.positive_conditions,
            )?)?,
            negative_conditions: serde_json::to_value(role_rule_conditions_contract(
                &candidate.negative_conditions,
            )?)?,
            score: serde_json::to_value(&candidate.score)?,
            provenance: json!({
                "source": "architecture_roles_seed",
                "seed_profile": profile_name,
            }),
            evidence: candidate.evidence,
            metadata: candidate.metadata,
            supersedes_rule_id: None,
        };
        insert_role_rule(relational, &rule).await?;
        rules_created += 1;
    }

    Ok(SeedSummary {
        profile_name: profile_name.to_string(),
        roles_total,
        roles_created,
        roles_reused,
        rules_total,
        rules_created,
        rules_reused,
    })
}

pub(super) async fn load_seed_owned_draft_rule_ids(
    relational: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    profile_name: &str,
) -> Result<Vec<String>> {
    let rows = relational
        .query_rows(&format!(
            "SELECT rule.rule_id, rule.provenance_json \
             FROM architecture_role_detection_rules rule \
             JOIN architecture_roles role \
               ON role.repo_id = rule.repo_id AND role.role_id = rule.role_id \
             WHERE rule.repo_id = {repo_id} \
               AND rule.lifecycle_status = 'draft' \
             ORDER BY role.canonical_key ASC, rule.version ASC, rule.rule_id ASC;",
            repo_id = sql_text(repo_id),
        ))
        .await
        .context("loading seed-owned draft architecture role rules")?;

    let mut rule_ids = Vec::new();
    for row in rows {
        let rule_id = row
            .get("rule_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("seed-owned draft rule row missing `rule_id`"))?;
        let provenance = match row.get("provenance_json") {
            Some(Value::String(text)) if !text.trim().is_empty() => {
                serde_json::from_str::<Value>(text)
                    .context("parsing architecture role rule provenance")?
            }
            Some(value) => value.clone(),
            None => json!({}),
        };
        let is_seed_owned = provenance.get("source").and_then(Value::as_str)
            == Some("architecture_roles_seed")
            && provenance.get("seed_profile").and_then(Value::as_str) == Some(profile_name);
        if is_seed_owned {
            rule_ids.push(rule_id.to_string());
        }
    }

    Ok(rule_ids)
}

pub(super) async fn activate_seeded_draft_rules(
    relational: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    profile_name: &str,
    provenance: Value,
) -> Result<SeedRuleActivationSummary> {
    let rule_ids = load_seed_owned_draft_rule_ids(relational, repo_id, profile_name).await?;
    let mut activated_rule_ids = Vec::new();
    let mut proposal_ids = Vec::new();

    for rule_id in &rule_ids {
        let rule_ref = format!("rule:{rule_id}");
        let proposal = create_rule_activate_proposal(
            relational,
            repo_id,
            &rule_ref,
            merge_provenance(
                provenance.clone(),
                json!({
                    "source": "architecture_roles_seed_automation",
                    "operation": "activate_seed_rule",
                    "seed_profile": profile_name,
                }),
            ),
        )
        .await?;
        apply_proposal(relational, repo_id, &proposal.proposal_id).await?;
        activated_rule_ids.push(rule_id.clone());
        proposal_ids.push(proposal.proposal_id);
    }

    Ok(SeedRuleActivationSummary {
        seed_owned_draft_rules: rule_ids.len(),
        proposals_created: proposal_ids.len(),
        proposals_applied: proposal_ids.len(),
        activated_rule_ids,
        proposal_ids,
    })
}

pub(super) fn configured_seed_profile_name(scoped_config: Option<&Value>) -> Result<String> {
    scoped_config
        .and_then(|value| value.pointer("/inference/fact_synthesis"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            anyhow!(
                "No architecture inference profile is configured. Set `[architecture.inference].fact_synthesis` to a structured-generation profile such as `local_agent`."
            )
        })
}

pub(super) fn format_seed_command_output(
    summary: &SeedCommandSummary,
    json_output: bool,
) -> Result<String> {
    if json_output {
        return serde_json::to_string_pretty(summary)
            .context("serialising architecture roles seed output as JSON");
    }

    let mut sections = vec![format_seed_summary(&summary.seed)];
    if let Some(activation) = summary.rule_activation.as_ref() {
        sections.push(format!(
            "seeded rule activation: seed_owned_draft_rules={} proposals_created={} proposals_applied={}",
            activation.seed_owned_draft_rules,
            activation.proposals_created,
            activation.proposals_applied,
        ));
        for (rule_id, proposal_id) in activation
            .activated_rule_ids
            .iter()
            .zip(activation.proposal_ids.iter())
        {
            sections.push(format!("activated_rule={rule_id} proposal={proposal_id}"));
        }
    }
    if let Some(classification) = summary.classification.as_ref() {
        sections.push(format_roles_classify_output(classification, false)?);
    }

    Ok(sections.join("\n"))
}

fn format_seed_summary(summary: &SeedSummary) -> String {
    format!(
        "architecture roles seeded with profile `{}`\nroles: total={} created={} reused={}\nrules: total={} created={} reused={}",
        summary.profile_name,
        summary.roles_total,
        summary.roles_created,
        summary.roles_reused,
        summary.rules_total,
        summary.rules_created,
        summary.rules_reused,
    )
}

fn sql_text(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn merge_provenance(mut left: Value, right: Value) -> Value {
    match (&mut left, right) {
        (Value::Object(left), Value::Object(right)) => {
            for (key, value) in right {
                left.insert(key, value);
            }
            Value::Object(left.clone())
        }
        (_, right) => right,
    }
}

pub(super) async fn ensure_seed_alias(
    relational: &crate::host::devql::RelationalStorage,
    alias: &ArchitectureRoleAliasRecord,
) -> Result<()> {
    match create_role_alias(relational, alias).await? {
        Ok(()) => Ok(()),
        Err(AliasConflict::AlreadyAssignedToDifferentRole {
            alias,
            existing_role_id,
        }) => {
            bail!("seeded role alias `{alias}` conflicts with existing role `{existing_role_id}`")
        }
    }
}

fn seed_rule_hash(role_id: &str, candidate: &SeededArchitectureRuleCandidate) -> Result<String> {
    let bytes = serde_json::to_vec(&json!({
        "role_id": role_id,
        "candidate_selector": role_rule_candidate_selector_contract(&candidate.candidate_selector),
        "positive_conditions": role_rule_conditions_contract(&candidate.positive_conditions)?,
        "negative_conditions": role_rule_conditions_contract(&candidate.negative_conditions)?,
        "score": candidate.score,
    }))
    .context("serialising seeded rule candidate for hashing")?;
    Ok(hex::encode(Sha256::digest(bytes)))
}
