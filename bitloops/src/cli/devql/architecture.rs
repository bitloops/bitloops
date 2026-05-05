use anyhow::{Context, Result, anyhow, bail};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;

use crate::capability_packs::architecture_graph::roles::llm_adjudication::{
    collect_seed_evidence, run_seed_generation,
};
use crate::capability_packs::architecture_graph::roles::migrations::{
    ProposalApplySummary, ProposalSummary, apply_proposal, create_alias_proposal,
    create_deprecate_role_proposal, create_merge_role_proposal, create_remove_role_proposal,
    create_rename_role_proposal, create_rule_activate_proposal, create_rule_disable_proposal,
    create_rule_draft_proposal, create_rule_edit_proposal, create_split_role_proposal,
    show_proposal,
};
use crate::capability_packs::architecture_graph::roles::storage::{
    AliasConflict, ArchitectureRoleAliasRecord, ArchitectureRoleRecord, ArchitectureRoleRuleRecord,
    create_role_alias, deterministic_alias_id, deterministic_role_id, deterministic_rule_id,
    insert_role_rule, load_role_by_alias, load_role_by_canonical_key, load_role_rules,
    next_role_rule_version, normalize_role_alias, normalize_role_key, upsert_role,
};
use crate::capability_packs::architecture_graph::roles::taxonomy::{
    RoleSplitSpecFile, RuleSpecFile, SeededArchitectureTaxonomy,
};
use crate::config::InferenceTask;
use crate::host::capability_host::DevqlCapabilityHost;
use crate::host::inference::InferenceGateway;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
struct SeedSummary {
    profile_name: String,
    roles_total: usize,
    roles_created: usize,
    roles_reused: usize,
    rules_total: usize,
    rules_created: usize,
    rules_reused: usize,
}

pub(super) async fn run_architecture_command(
    scope: &SlimCliRepoScope,
    args: DevqlArchitectureArgs,
) -> Result<()> {
    let host = DevqlCapabilityHost::builtin(scope.repo_root.clone(), scope.repo.clone())?;
    host.ensure_migrations_applied_sync()?;
    let context = host.build_current_state_consumer_context("architecture_graph")?;

    match args.command {
        DevqlArchitectureCommand::Roles(args) => {
            run_architecture_roles_command(scope, &host, &context, args).await
        }
    }
}

async fn run_architecture_roles_command(
    scope: &SlimCliRepoScope,
    host: &DevqlCapabilityHost,
    context: &crate::host::capability_host::CurrentStateConsumerContext,
    args: DevqlArchitectureRolesArgs,
) -> Result<()> {
    match args.command {
        DevqlArchitectureRolesCommand::Seed(_) => {
            run_architecture_roles_seed(scope, host, context).await
        }
        DevqlArchitectureRolesCommand::Rename(args) => {
            let summary = create_rename_role_proposal(
                context.storage.as_ref(),
                &scope.repo.repo_id,
                &args.role_ref,
                &args.display_name,
                cli_provenance("rename_role"),
            )
            .await?;
            print_proposal_summary(&summary);
            Ok(())
        }
        DevqlArchitectureRolesCommand::Deprecate(args) => {
            let summary = create_deprecate_role_proposal(
                context.storage.as_ref(),
                &scope.repo.repo_id,
                &args.role_ref,
                args.replacement.as_deref(),
                cli_provenance("deprecate_role"),
            )
            .await?;
            print_proposal_summary(&summary);
            Ok(())
        }
        DevqlArchitectureRolesCommand::Remove(args) => {
            let summary = create_remove_role_proposal(
                context.storage.as_ref(),
                &scope.repo.repo_id,
                &args.role_ref,
                args.replacement.as_deref(),
                cli_provenance("remove_role"),
            )
            .await?;
            print_proposal_summary(&summary);
            Ok(())
        }
        DevqlArchitectureRolesCommand::Merge(args) => {
            let summary = create_merge_role_proposal(
                context.storage.as_ref(),
                &scope.repo.repo_id,
                &args.source_role_ref,
                &args.target_role_ref,
                cli_provenance("merge_roles"),
            )
            .await?;
            print_proposal_summary(&summary);
            Ok(())
        }
        DevqlArchitectureRolesCommand::Split(args) => {
            let spec: RoleSplitSpecFile = load_json_spec(&args.spec)?;
            let summary = create_split_role_proposal(
                context.storage.as_ref(),
                &scope.repo.repo_id,
                &args.role_ref,
                spec,
                cli_provenance("split_role"),
            )
            .await?;
            print_proposal_summary(&summary);
            Ok(())
        }
        DevqlArchitectureRolesCommand::Alias(args) => match args.command {
            DevqlArchitectureRolesAliasCommand::Create(args) => {
                let summary = create_alias_proposal(
                    context.storage.as_ref(),
                    &scope.repo.repo_id,
                    &args.role_ref,
                    &args.alias_key,
                    cli_provenance("create_role_alias"),
                )
                .await?;
                print_proposal_summary(&summary);
                Ok(())
            }
        },
        DevqlArchitectureRolesCommand::Rules(args) => match args.command {
            DevqlArchitectureRolesRulesCommand::Draft(args) => {
                let spec: RuleSpecFile = load_json_spec(&args.spec)?;
                let summary = create_rule_draft_proposal(
                    context.storage.as_ref(),
                    context.relational.as_ref(),
                    &scope.repo.repo_id,
                    spec,
                    cli_provenance("draft_rule"),
                )
                .await?;
                print_proposal_summary(&summary);
                Ok(())
            }
            DevqlArchitectureRolesRulesCommand::Edit(args) => {
                let spec: RuleSpecFile = load_json_spec(&args.spec)?;
                let summary = create_rule_edit_proposal(
                    context.storage.as_ref(),
                    context.relational.as_ref(),
                    &scope.repo.repo_id,
                    &args.rule_ref,
                    spec,
                    cli_provenance("edit_rule"),
                )
                .await?;
                print_proposal_summary(&summary);
                Ok(())
            }
            DevqlArchitectureRolesRulesCommand::Activate(args) => {
                let summary = create_rule_activate_proposal(
                    context.storage.as_ref(),
                    &scope.repo.repo_id,
                    &args.rule_ref,
                    cli_provenance("activate_rule"),
                )
                .await?;
                print_proposal_summary(&summary);
                Ok(())
            }
            DevqlArchitectureRolesRulesCommand::Disable(args) => {
                let summary = create_rule_disable_proposal(
                    context.storage.as_ref(),
                    &scope.repo.repo_id,
                    &args.rule_ref,
                    cli_provenance("disable_rule"),
                )
                .await?;
                print_proposal_summary(&summary);
                Ok(())
            }
        },
        DevqlArchitectureRolesCommand::Proposal(args) => match args.command {
            DevqlArchitectureRolesProposalCommand::Show(args) => {
                let summary = show_proposal(
                    context.storage.as_ref(),
                    &scope.repo.repo_id,
                    &args.proposal_id,
                )
                .await?;
                print_proposal_summary(&summary);
                Ok(())
            }
            DevqlArchitectureRolesProposalCommand::Apply(args) => {
                let summary = apply_proposal(
                    context.storage.as_ref(),
                    &scope.repo.repo_id,
                    &args.proposal_id,
                )
                .await?;
                print_apply_summary(&summary);
                Ok(())
            }
        },
    }
}

async fn run_architecture_roles_seed(
    scope: &SlimCliRepoScope,
    host: &DevqlCapabilityHost,
    context: &crate::host::capability_host::CurrentStateConsumerContext,
) -> Result<()> {
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
    let summary = persist_seeded_taxonomy(
        context.storage.as_ref(),
        &scope.repo.repo_id,
        &profile_name,
        taxonomy,
    )
    .await?;
    println!("{}", format_seed_summary(&summary));
    Ok(())
}

async fn persist_seeded_taxonomy(
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
            candidate_selector: serde_json::to_value(&candidate.candidate_selector)?,
            positive_conditions: serde_json::to_value(&candidate.positive_conditions)?,
            negative_conditions: serde_json::to_value(&candidate.negative_conditions)?,
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

fn configured_seed_profile_name(scoped_config: Option<&Value>) -> Result<String> {
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

fn print_proposal_summary(summary: &ProposalSummary) {
    println!(
        "proposal={} type={} status={}",
        summary.proposal_id, summary.proposal_type, summary.status
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&summary.preview_payload)
            .unwrap_or_else(|_| summary.preview_payload.to_string())
    );
}

fn print_apply_summary(summary: &ProposalApplySummary) {
    println!(
        "proposal={} type={} applied",
        summary.proposal_id, summary.proposal_type
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&summary.result_payload)
            .unwrap_or_else(|_| summary.result_payload.to_string())
    );
    if !summary.migration_records.is_empty() {
        println!("migrations={}", summary.migration_records.len());
    }
}

fn load_json_spec<T: DeserializeOwned>(path: &std::path::Path) -> Result<T> {
    let bytes = fs::read(path)
        .with_context(|| format!("reading architecture roles spec from `{}`", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing JSON spec from `{}`", path.display()))
}

fn cli_provenance(operation: &str) -> Value {
    json!({
        "source": "devql_cli",
        "operation": operation,
    })
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

async fn ensure_seed_alias(
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

fn seed_rule_hash(
    role_id: &str,
    candidate: &crate::capability_packs::architecture_graph::roles::taxonomy::SeededArchitectureRuleCandidate,
) -> Result<String> {
    let bytes = serde_json::to_vec(&json!({
        "role_id": role_id,
        "candidate_selector": candidate.candidate_selector,
        "positive_conditions": candidate.positive_conditions,
        "negative_conditions": candidate.negative_conditions,
        "score": candidate.score,
    }))
    .context("serialising seeded rule candidate for hashing")?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability_packs::architecture_graph::roles::storage::{
        list_roles, load_role_by_id, load_role_rules,
    };
    use crate::capability_packs::architecture_graph::roles::taxonomy::{
        RoleRuleCandidateSelector, RoleRuleScore, SeededArchitectureRole,
        SeededArchitectureRuleCandidate,
    };
    use crate::capability_packs::architecture_graph::schema::architecture_graph_sqlite_schema_sql;
    use crate::host::devql::RelationalStorage;

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

    fn seeded_taxonomy(role_key: &str) -> SeededArchitectureTaxonomy {
        SeededArchitectureTaxonomy {
            roles: vec![SeededArchitectureRole {
                canonical_key: role_key.to_string(),
                display_name: "Command Dispatcher".to_string(),
                description: "Routes CLI commands".to_string(),
                family: Some("entrypoint".to_string()),
                lifecycle_status: Some("active".to_string()),
                provenance: json!({"source": "test"}),
                evidence: json!(["cli surface"]),
            }],
            rule_candidates: vec![SeededArchitectureRuleCandidate {
                target_role_key: role_key.to_string(),
                candidate_selector: RoleRuleCandidateSelector {
                    path_prefixes: vec!["src/cli".to_string()],
                    ..Default::default()
                },
                positive_conditions: vec![],
                negative_conditions: vec![],
                score: RoleRuleScore {
                    base_confidence: Some(0.9),
                    weight: Some(1.0),
                },
                evidence: json!(["path prefix"]),
                metadata: json!({"source": "test"}),
            }],
        }
    }

    #[test]
    fn configured_seed_profile_name_requires_fact_synthesis_config() {
        let err = configured_seed_profile_name(Some(&json!({}))).expect_err("missing config");
        assert!(
            err.to_string()
                .contains("[architecture.inference].fact_synthesis")
        );

        let profile = configured_seed_profile_name(Some(
            &json!({"inference": {"fact_synthesis": "local_agent"}}),
        ))
        .expect("configured profile");
        assert_eq!(profile, "local_agent");
    }

    #[tokio::test]
    async fn persist_seeded_taxonomy_is_idempotent_for_repeated_runs() -> Result<()> {
        let relational = relational().await?;

        let first = persist_seeded_taxonomy(
            &relational,
            "repo-1",
            "local_agent",
            seeded_taxonomy("command_dispatcher"),
        )
        .await?;
        assert_eq!(first.roles_created, 1);
        assert_eq!(first.rules_created, 1);

        let second = persist_seeded_taxonomy(
            &relational,
            "repo-1",
            "local_agent",
            seeded_taxonomy("command_dispatcher"),
        )
        .await?;
        assert_eq!(second.roles_reused, 1);
        assert_eq!(second.rules_reused, 1);

        let roles = list_roles(&relational, "repo-1").await?;
        assert_eq!(roles.len(), 1);
        let rules = load_role_rules(&relational, "repo-1", &roles[0].role_id).await?;
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].lifecycle_status, "draft");
        Ok(())
    }

    #[tokio::test]
    async fn persist_seeded_taxonomy_reuses_alias_equivalent_roles() -> Result<()> {
        let relational = relational().await?;
        let existing = ArchitectureRoleRecord {
            role_id: deterministic_role_id("repo-1", "command_dispatcher"),
            repo_id: "repo-1".to_string(),
            canonical_key: "command_dispatcher".to_string(),
            display_name: "Command Dispatcher".to_string(),
            description: "Routes CLI commands".to_string(),
            family: Some("entrypoint".to_string()),
            lifecycle_status: "active".to_string(),
            provenance: json!({"source": "test"}),
            evidence: json!([]),
            metadata: json!({}),
        };
        let existing = upsert_role(&relational, &existing).await?;
        ensure_seed_alias(
            &relational,
            &ArchitectureRoleAliasRecord {
                alias_id: deterministic_alias_id("repo-1", "cli_command_dispatcher"),
                repo_id: "repo-1".to_string(),
                role_id: existing.role_id.clone(),
                alias_key: "cli_command_dispatcher".to_string(),
                alias_normalized: normalize_role_alias("cli_command_dispatcher"),
                source_kind: "manual".to_string(),
                metadata: json!({}),
            },
        )
        .await?;

        let summary = persist_seeded_taxonomy(
            &relational,
            "repo-1",
            "local_agent",
            seeded_taxonomy("cli_command_dispatcher"),
        )
        .await?;
        assert_eq!(summary.roles_created, 0);
        assert_eq!(summary.roles_reused, 1);

        let roles = list_roles(&relational, "repo-1").await?;
        assert_eq!(roles.len(), 1);
        let loaded = load_role_by_id(&relational, "repo-1", &existing.role_id)
            .await?
            .expect("existing role");
        assert_eq!(loaded.canonical_key, "command_dispatcher");
        let rules = load_role_rules(&relational, "repo-1", &existing.role_id).await?;
        assert_eq!(rules.len(), 1);
        Ok(())
    }
}
