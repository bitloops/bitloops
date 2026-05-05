use anyhow::{Context, Result, anyhow, bail};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;

use crate::capability_packs::architecture_graph::roles::RoleAdjudicationMailboxPayload;
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
    RoleSplitSpecFile, RuleSpecFile, SeededArchitectureRuleCandidate, SeededArchitectureTaxonomy,
    role_rule_candidate_selector_contract, role_rule_conditions_contract,
};
use crate::capability_packs::architecture_graph::types::{
    ARCHITECTURE_GRAPH_CAPABILITY_ID, ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX,
};
use crate::config::InferenceTask;
use crate::host::capability_host::DevqlCapabilityHost;
use crate::host::inference::InferenceGateway;
use crate::host::runtime_store::{RepoSqliteRuntimeStore, WorkplaneJobQuery, WorkplaneJobStatus};

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

const ROLE_REVIEW_STATUSES: &[&str] = &["needs_review", "stale", "rejected", "unknown"];

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
        DevqlArchitectureRolesCommand::Status(args) => {
            run_architecture_roles_status(scope, context, args).await
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

#[derive(Debug, Clone, Serialize)]
struct RolesStatusOutput {
    queue_summary: RoleAdjudicationQueueSummary,
    queue_items: Vec<RoleAdjudicationQueueItem>,
    review_items: Vec<RoleReviewItem>,
}

#[derive(Debug, Clone, Serialize)]
struct RoleAdjudicationQueueSummary {
    total: usize,
    by_status: BTreeMap<String, usize>,
    by_reason: BTreeMap<String, usize>,
    parse_errors: usize,
}

#[derive(Debug, Clone, Serialize)]
struct RoleAdjudicationQueueItem {
    job_id: String,
    status: String,
    attempts: u32,
    updated_at_unix: u64,
    dedupe_key: Option<String>,
    reason: Option<String>,
    generation: Option<u64>,
    artefact_id: Option<String>,
    symbol_id: Option<String>,
    path: Option<String>,
    canonical_kind: Option<String>,
    deterministic_confidence: Option<f64>,
    parse_error: Option<String>,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct RoleReviewItem {
    assignment_id: String,
    artefact_id: String,
    path: Option<String>,
    role_id: String,
    source_kind: String,
    confidence: f64,
    status: String,
    status_reason: String,
    updated_at: Option<String>,
}

async fn run_architecture_roles_status(
    scope: &SlimCliRepoScope,
    context: &crate::host::capability_host::CurrentStateConsumerContext,
    args: DevqlArchitectureRolesStatusArgs,
) -> Result<()> {
    let limit = usize::try_from(args.limit).context("converting --limit to usize")?;
    let queue_items = load_role_adjudication_queue_items(scope, limit)?;
    let review_items =
        load_role_review_items(context.storage.as_ref(), &scope.repo.repo_id, limit).await?;
    let summary = summarise_queue_items(&queue_items);
    let output = RolesStatusOutput {
        queue_summary: summary,
        queue_items,
        review_items,
    };

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&output)
                .context("serialising roles status output as JSON")?
        );
        return Ok(());
    }

    print_roles_status_human(&output);
    Ok(())
}

fn load_role_adjudication_queue_items(
    scope: &SlimCliRepoScope,
    limit: usize,
) -> Result<Vec<RoleAdjudicationQueueItem>> {
    let store = RepoSqliteRuntimeStore::open(&scope.repo_root)
        .context("opening repo runtime store for architecture roles status")?;
    let jobs = store
        .list_capability_workplane_jobs(WorkplaneJobQuery {
            capability_id: Some(ARCHITECTURE_GRAPH_CAPABILITY_ID.to_string()),
            mailbox_name: Some(ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX.to_string()),
            statuses: vec![
                WorkplaneJobStatus::Pending,
                WorkplaneJobStatus::Running,
                WorkplaneJobStatus::Failed,
            ],
            limit: Some(limit as u64),
        })
        .context("loading architecture role adjudication queue jobs")?;

    Ok(jobs
        .into_iter()
        .map(role_adjudication_queue_item_from_job)
        .collect())
}

fn role_adjudication_queue_item_from_job(
    job: crate::host::runtime_store::WorkplaneJobRecord,
) -> RoleAdjudicationQueueItem {
    let mut item = RoleAdjudicationQueueItem {
        job_id: job.job_id,
        status: job.status.as_str().to_string(),
        attempts: job.attempts,
        updated_at_unix: job.updated_at_unix,
        dedupe_key: job.dedupe_key,
        reason: None,
        generation: None,
        artefact_id: None,
        symbol_id: None,
        path: None,
        canonical_kind: None,
        deterministic_confidence: None,
        parse_error: None,
        last_error: job.last_error,
    };

    match serde_json::from_value::<RoleAdjudicationMailboxPayload>(job.payload) {
        Ok(payload) => {
            item.reason = Some(payload.request.reason.as_str().to_string());
            item.generation = Some(payload.request.generation);
            item.artefact_id = payload.request.artefact_id;
            item.symbol_id = payload.request.symbol_id;
            item.path = payload.request.path;
            item.canonical_kind = payload.request.canonical_kind;
            item.deterministic_confidence = payload.request.deterministic_confidence;
        }
        Err(err) => {
            item.parse_error = Some(err.to_string());
        }
    }

    item
}

fn summarise_queue_items(items: &[RoleAdjudicationQueueItem]) -> RoleAdjudicationQueueSummary {
    let mut by_status = BTreeMap::<String, usize>::new();
    let mut by_reason = BTreeMap::<String, usize>::new();
    let mut parse_errors = 0usize;
    for item in items {
        *by_status.entry(item.status.clone()).or_default() += 1;
        if let Some(reason) = item.reason.as_ref() {
            *by_reason.entry(reason.clone()).or_default() += 1;
        }
        if item.parse_error.is_some() {
            parse_errors += 1;
        }
    }
    RoleAdjudicationQueueSummary {
        total: items.len(),
        by_status,
        by_reason,
        parse_errors,
    }
}

async fn load_role_review_items(
    relational: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    limit: usize,
) -> Result<Vec<RoleReviewItem>> {
    let status_filters = ROLE_REVIEW_STATUSES
        .iter()
        .map(|status| format!("'{}'", status.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");
    let rows = relational
        .query_rows(&format!(
            "SELECT a.assignment_id, COALESCE(a.artefact_id, a.symbol_id, a.path) AS artefact_id, \
                    a.role_id, a.source AS source_kind, a.confidence, a.status, \
                    a.provenance_json, a.updated_at, a.path \
             FROM architecture_role_assignments_current a \
             WHERE a.repo_id = {repo_id} \
               AND a.status IN ({status_filters}) \
             ORDER BY a.updated_at DESC \
             LIMIT {limit};",
            repo_id = sql_text(repo_id),
        ))
        .await
        .context("loading architecture role review items")?;

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let assignment_id = value_str(&row, "assignment_id")
            .ok_or_else(|| anyhow!("missing `assignment_id` in architecture role review row"))?
            .to_string();
        let artefact_id = value_str(&row, "artefact_id")
            .ok_or_else(|| anyhow!("missing `artefact_id` in architecture role review row"))?
            .to_string();
        let role_id = value_str(&row, "role_id")
            .ok_or_else(|| anyhow!("missing `role_id` in architecture role review row"))?
            .to_string();
        let source_kind = value_str(&row, "source_kind")
            .ok_or_else(|| anyhow!("missing `source_kind` in architecture role review row"))?
            .to_string();
        let confidence = row.get("confidence").and_then(Value::as_f64).unwrap_or(0.0);
        let status = value_str(&row, "status")
            .ok_or_else(|| anyhow!("missing `status` in architecture role review row"))?
            .to_string();
        let status_reason = value_json(&row, "provenance_json")
            .and_then(|value| {
                value
                    .get("statusReason")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .unwrap_or_default();
        let updated_at = value_str(&row, "updated_at").map(ToOwned::to_owned);
        let path = value_str(&row, "path").map(ToOwned::to_owned);

        items.push(RoleReviewItem {
            assignment_id,
            artefact_id,
            path,
            role_id,
            source_kind,
            confidence,
            status,
            status_reason,
            updated_at,
        });
    }

    Ok(items)
}

fn print_roles_status_human(output: &RolesStatusOutput) {
    if output.queue_items.is_empty() && output.review_items.is_empty() {
        println!("no ambiguous architecture roles found");
        return;
    }

    println!("queue summary:");
    println!("  total={}", output.queue_summary.total);
    if !output.queue_summary.by_status.is_empty() {
        let entries = output
            .queue_summary
            .by_status
            .iter()
            .map(|(status, count)| format!("{status}={count}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("  by_status: {entries}");
    }
    if !output.queue_summary.by_reason.is_empty() {
        let entries = output
            .queue_summary
            .by_reason
            .iter()
            .map(|(reason, count)| format!("{reason}={count}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!("  by_reason: {entries}");
    }
    if output.queue_summary.parse_errors > 0 {
        println!("  parse_errors={}", output.queue_summary.parse_errors);
    }

    if !output.queue_items.is_empty() {
        println!("queue items:");
        for item in &output.queue_items {
            println!(
                "  job={} status={} reason={} path={} artefact={} symbol={} generation={} confidence={} attempts={} updated_at_unix={}",
                item.job_id,
                item.status,
                item.reason.as_deref().unwrap_or("<unknown>"),
                item.path.as_deref().unwrap_or("<unknown>"),
                item.artefact_id.as_deref().unwrap_or("<unknown>"),
                item.symbol_id.as_deref().unwrap_or("<unknown>"),
                item.generation
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "<unknown>".to_string()),
                item.deterministic_confidence
                    .map(|value| format!("{value:.3}"))
                    .unwrap_or_else(|| "<unknown>".to_string()),
                item.attempts,
                item.updated_at_unix
            );
            if let Some(parse_error) = item.parse_error.as_deref() {
                println!("    parse_error={parse_error}");
            }
            if let Some(last_error) = item.last_error.as_deref() {
                println!("    last_error={last_error}");
            }
        }
    }

    if !output.review_items.is_empty() {
        println!("review items:");
        for item in &output.review_items {
            println!(
                "  assignment={} status={} role={} source={} confidence={:.3} artefact={} path={} updated_at={}",
                item.assignment_id,
                item.status,
                item.role_id,
                item.source_kind,
                item.confidence,
                item.artefact_id,
                item.path.as_deref().unwrap_or("<unknown>"),
                item.updated_at.as_deref().unwrap_or("<unknown>"),
            );
            if !item.status_reason.trim().is_empty() {
                println!("    reason={}", item.status_reason);
            }
        }
    }
}

fn sql_text(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn value_str<'a>(row: &'a Value, key: &str) -> Option<&'a str> {
    row.get(key).and_then(Value::as_str)
}

fn value_json(row: &Value, key: &str) -> Option<Value> {
    match row.get(key) {
        Some(Value::String(text)) => serde_json::from_str(text).ok(),
        Some(value) => Some(value.clone()),
        None => None,
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

#[cfg(test)]
mod deterministic_tests;
