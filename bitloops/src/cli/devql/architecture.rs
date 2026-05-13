use anyhow::{Context, Result, anyhow};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use crate::capability_packs::architecture_graph::roles::classifier::{
    ArchitectureRoleClassificationInput, ArchitectureRoleClassificationScope,
    classify_architecture_roles_for_current_state,
};
use crate::capability_packs::architecture_graph::roles::migrations::{
    ProposalApplySummary, ProposalSummary, apply_proposal, create_alias_proposal,
    create_deprecate_role_proposal, create_merge_role_proposal, create_remove_role_proposal,
    create_rename_role_proposal, create_rule_activate_proposal, create_rule_disable_proposal,
    create_rule_draft_proposal, create_rule_edit_proposal, create_split_role_proposal,
    show_proposal,
};
use crate::capability_packs::architecture_graph::roles::storage::list_recent_role_adjudication_attempts;
use crate::capability_packs::architecture_graph::roles::taxonomy::{
    RoleSplitSpecFile, RuleSpecFile,
};
use crate::capability_packs::architecture_graph::roles::{
    ArchitectureRoleReconcileMetrics, RoleAdjudicationEnqueueMetrics,
    RoleAdjudicationMailboxPayload, default_queue_store, enqueue_adjudication_requests,
};
use crate::capability_packs::architecture_graph::types::{
    ARCHITECTURE_GRAPH_CAPABILITY_ID, ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX,
};
use crate::host::capability_host::DevqlCapabilityHost;
use crate::host::runtime_store::{
    RepoCapabilityWorkplaneStatusReader, WorkplaneJobQuery, WorkplaneJobStatus,
};

use super::*;

mod roles_seed;

use roles_seed::{
    BootstrapCommandSummary, SeedCommandSummary, activate_seeded_draft_rules,
    configured_seed_profile_name, ensure_seed_owned_draft_rules_exist,
    format_bootstrap_command_output, format_seed_command_output, seed_architecture_roles,
};
#[cfg(test)]
use roles_seed::{
    SeedRuleActivationSummary, SeedSummary, architecture_seed_request_diagnostics,
    ensure_seed_alias, persist_seeded_taxonomy,
};

const ROLE_REVIEW_STATUSES: &[&str] = &["needs_review", "stale", "rejected", "unknown"];

pub(super) async fn run_architecture_command(
    scope: &SlimCliRepoScope,
    args: DevqlArchitectureArgs,
) -> Result<()> {
    let host = DevqlCapabilityHost::builtin(scope.repo_root.clone(), scope.repo.clone())?;
    host.ensure_migrations_applied_sync()?;

    match args.command {
        DevqlArchitectureCommand::Roles(args) => {
            if !architecture_roles_command_requires_current_state_context(&args) {
                return run_architecture_roles_command_without_current_state(scope, &host, args)
                    .await;
            }
            let context = host.build_current_state_consumer_context("architecture_graph")?;
            run_architecture_roles_command(scope, &host, &context, args).await
        }
    }
}

fn architecture_roles_command_requires_current_state_context(
    args: &DevqlArchitectureRolesArgs,
) -> bool {
    !matches!(&args.command, DevqlArchitectureRolesCommand::Status(_))
}

async fn run_architecture_roles_command_without_current_state(
    scope: &SlimCliRepoScope,
    host: &DevqlCapabilityHost,
    args: DevqlArchitectureRolesArgs,
) -> Result<()> {
    match args.command {
        DevqlArchitectureRolesCommand::Status(args) => {
            let relational = host.build_relational_storage()?;
            run_architecture_roles_status(scope, &relational, args).await
        }
        _ => {
            let context = host.build_current_state_consumer_context("architecture_graph")?;
            run_architecture_roles_command(scope, host, &context, args).await
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
        DevqlArchitectureRolesCommand::Seed(args) => {
            run_architecture_roles_seed_command(scope, host, context, args).await
        }
        DevqlArchitectureRolesCommand::Bootstrap(args) => {
            run_architecture_roles_bootstrap_command(scope, host, context, args).await
        }
        DevqlArchitectureRolesCommand::Classify(args) => {
            run_architecture_roles_classify(scope, context, args).await
        }
        DevqlArchitectureRolesCommand::Status(args) => {
            run_architecture_roles_status(scope, context.storage.as_ref(), args).await
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
    adjudication_attempt_summary: RoleAdjudicationAttemptSummary,
    adjudication_attempts: Vec<RoleAdjudicationAttemptItem>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct RolesClassifyOutput {
    roles: ArchitectureRoleReconcileMetrics,
    role_adjudication_selected: usize,
    role_adjudication_enqueued: usize,
    role_adjudication_deduped: usize,
    warnings: Vec<String>,
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
struct RoleAdjudicationAttemptSummary {
    total: usize,
    by_outcome: BTreeMap<String, usize>,
    persisted_assignments: usize,
}

#[derive(Debug, Clone, Serialize)]
struct RoleAdjudicationAttemptItem {
    attempt_id: String,
    scope_key: String,
    generation: u64,
    target_kind: Option<String>,
    artefact_id: Option<String>,
    symbol_id: Option<String>,
    path: Option<String>,
    reason: String,
    outcome: String,
    model_descriptor: String,
    assignment_write_persisted: bool,
    assignment_write_source: Option<String>,
    failure_message: Option<String>,
    reasoning_summary: Option<String>,
    observed_at_unix: u64,
    updated_at: Option<String>,
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

async fn classify_architecture_roles_with_output(
    scope: &SlimCliRepoScope,
    context: &crate::host::capability_host::CurrentStateConsumerContext,
    args: DevqlArchitectureRolesClassifyArgs,
) -> Result<RolesClassifyOutput> {
    let files = context
        .relational
        .load_current_canonical_files(&scope.repo.repo_id)
        .context("loading current files for architecture role classification")?;
    let artefacts = context
        .relational
        .load_current_canonical_artefacts(&scope.repo.repo_id)
        .context("loading current artefacts for architecture role classification")?;
    let dependency_edges = context
        .relational
        .load_current_canonical_edges(&scope.repo.repo_id)
        .context("loading current dependency edges for architecture role classification")?;
    let generation_seq = crate::daemon::capability_event_latest_generation(&scope.repo.repo_id)
        .ok()
        .flatten()
        .unwrap_or(0);
    let role_scope = if args.full || args.repair_stale {
        ArchitectureRoleClassificationScope {
            full_reconcile: true,
            affected_paths: BTreeSet::new(),
            removed_paths: BTreeSet::new(),
        }
    } else {
        ArchitectureRoleClassificationScope {
            full_reconcile: false,
            affected_paths: args
                .paths
                .unwrap_or_default()
                .into_iter()
                .collect::<BTreeSet<_>>(),
            removed_paths: BTreeSet::new(),
        }
    };

    let outcome = classify_architecture_roles_for_current_state(
        context.storage.as_ref(),
        ArchitectureRoleClassificationInput {
            repo_id: &scope.repo.repo_id,
            generation_seq,
            scope: role_scope,
            files: &files,
            artefacts: &artefacts,
            dependency_edges: &dependency_edges,
        },
    )
    .await?;
    let mut warnings = outcome.warnings;
    let adjudication_metrics = if args.enqueue_adjudication {
        match enqueue_adjudication_requests(
            &outcome.adjudication_requests,
            context.workplane.as_ref(),
            default_queue_store().as_ref(),
        ) {
            Ok(metrics) => metrics,
            Err(err) => {
                warnings.push(format!(
                    "Architecture role adjudication enqueue failed: {err:#}"
                ));
                RoleAdjudicationEnqueueMetrics {
                    selected: outcome.adjudication_requests.len(),
                    enqueued: 0,
                    deduped: 0,
                }
            }
        }
    } else {
        RoleAdjudicationEnqueueMetrics {
            selected: outcome.adjudication_requests.len(),
            enqueued: 0,
            deduped: 0,
        }
    };
    Ok(RolesClassifyOutput {
        roles: outcome.metrics,
        role_adjudication_selected: adjudication_metrics.selected,
        role_adjudication_enqueued: adjudication_metrics.enqueued,
        role_adjudication_deduped: adjudication_metrics.deduped,
        warnings,
    })
}

async fn run_architecture_roles_classify(
    scope: &SlimCliRepoScope,
    context: &crate::host::capability_host::CurrentStateConsumerContext,
    args: DevqlArchitectureRolesClassifyArgs,
) -> Result<()> {
    let json_output = args.json;
    let output = classify_architecture_roles_with_output(scope, context, args).await?;
    println!("{}", format_roles_classify_output(&output, json_output)?);
    Ok(())
}

fn validate_seed_automation_args(args: &DevqlArchitectureRolesSeedArgs) -> Result<()> {
    if args.classify && !args.activate_rules {
        bail!("`roles seed --classify` requires `--activate-rules`");
    }
    Ok(())
}

async fn run_architecture_roles_seed_command(
    scope: &SlimCliRepoScope,
    host: &DevqlCapabilityHost,
    context: &crate::host::capability_host::CurrentStateConsumerContext,
    args: DevqlArchitectureRolesSeedArgs,
) -> Result<()> {
    validate_seed_automation_args(&args)?;

    let seed = seed_architecture_roles(scope, host, context).await?;
    let rule_activation = if args.activate_rules {
        Some(
            activate_seeded_draft_rules(
                context.storage.as_ref(),
                &scope.repo.repo_id,
                &seed.profile_name,
                cli_provenance("seed_activate_rules"),
            )
            .await?,
        )
    } else {
        None
    };
    let classification = if args.classify {
        Some(
            classify_architecture_roles_with_output(
                scope,
                context,
                DevqlArchitectureRolesClassifyArgs {
                    full: true,
                    paths: None,
                    repair_stale: false,
                    enqueue_adjudication: args.enqueue_adjudication,
                    json: args.json,
                },
            )
            .await?,
        )
    } else {
        None
    };
    let summary = SeedCommandSummary {
        seed,
        rule_activation,
        classification,
    };

    println!("{}", format_seed_command_output(&summary, args.json)?);
    Ok(())
}

async fn run_architecture_roles_bootstrap_command(
    scope: &SlimCliRepoScope,
    host: &DevqlCapabilityHost,
    context: &crate::host::capability_host::CurrentStateConsumerContext,
    args: DevqlArchitectureRolesBootstrapArgs,
) -> Result<()> {
    if args.skip_seed {
        let profile_name =
            configured_seed_profile_name(host.config_view("architecture_graph").scoped())?;
        ensure_seed_owned_draft_rules_exist(
            context.storage.as_ref(),
            &scope.repo.repo_id,
            &profile_name,
        )
        .await?;
        let rule_activation = activate_seeded_draft_rules(
            context.storage.as_ref(),
            &scope.repo.repo_id,
            &profile_name,
            cli_provenance("bootstrap_skip_seed_activate_rules"),
        )
        .await?;
        let classification = classify_architecture_roles_with_output(
            scope,
            context,
            DevqlArchitectureRolesClassifyArgs {
                full: true,
                paths: None,
                repair_stale: false,
                enqueue_adjudication: args.enqueue_adjudication,
                json: args.json,
            },
        )
        .await?;

        println!(
            "{}",
            format_bootstrap_command_output(
                &BootstrapCommandSummary {
                    seed: None,
                    rule_activation,
                    classification,
                    skipped_seed: true,
                },
                args.json,
            )?
        );
        return Ok(());
    }

    run_architecture_roles_seed_command(
        scope,
        host,
        context,
        DevqlArchitectureRolesSeedArgs {
            activate_rules: true,
            classify: true,
            enqueue_adjudication: args.enqueue_adjudication,
            json: args.json,
        },
    )
    .await
}

async fn run_architecture_roles_status(
    scope: &SlimCliRepoScope,
    relational: &crate::host::devql::RelationalStorage,
    args: DevqlArchitectureRolesStatusArgs,
) -> Result<()> {
    let limit = usize::try_from(args.limit).context("converting --limit to usize")?;
    let queue_items = load_role_adjudication_queue_items(scope, limit)?;
    let review_items = load_role_review_items(relational, &scope.repo.repo_id, limit).await?;
    let adjudication_attempts =
        load_role_adjudication_attempt_items(relational, &scope.repo.repo_id, limit).await?;
    let summary = summarise_queue_items(&queue_items);
    let adjudication_attempt_summary = summarise_adjudication_attempts(&adjudication_attempts);
    let output = RolesStatusOutput {
        queue_summary: summary,
        queue_items,
        review_items,
        adjudication_attempt_summary,
        adjudication_attempts,
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
    let Some(reader) =
        RepoCapabilityWorkplaneStatusReader::open(&scope.repo_root, &scope.repo.repo_id).context(
            "opening read-only repo runtime status reader for architecture roles status",
        )?
    else {
        return Ok(Vec::new());
    };

    let jobs = reader
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
        .context("loading architecture role adjudication queue jobs read-only")?;

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

async fn load_role_adjudication_attempt_items(
    relational: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    limit: usize,
) -> Result<Vec<RoleAdjudicationAttemptItem>> {
    let records = list_recent_role_adjudication_attempts(relational, repo_id, limit).await?;
    Ok(records
        .into_iter()
        .map(|record| RoleAdjudicationAttemptItem {
            attempt_id: record.attempt_id,
            scope_key: record.scope_key,
            generation: record.generation,
            target_kind: record.target_kind,
            artefact_id: record.artefact_id,
            symbol_id: record.symbol_id,
            path: record.path,
            reason: record.reason,
            outcome: record.outcome,
            model_descriptor: record.model_descriptor,
            assignment_write_persisted: record.assignment_write_persisted,
            assignment_write_source: record.assignment_write_source,
            failure_message: record.failure_message,
            reasoning_summary: record.reasoning_summary,
            observed_at_unix: record.observed_at_unix,
            updated_at: record.updated_at,
        })
        .collect())
}

fn summarise_adjudication_attempts(
    items: &[RoleAdjudicationAttemptItem],
) -> RoleAdjudicationAttemptSummary {
    let mut by_outcome = BTreeMap::<String, usize>::new();
    let mut persisted_assignments = 0usize;
    for item in items {
        *by_outcome.entry(item.outcome.clone()).or_default() += 1;
        if item.assignment_write_persisted {
            persisted_assignments += 1;
        }
    }
    RoleAdjudicationAttemptSummary {
        total: items.len(),
        by_outcome,
        persisted_assignments,
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
    if output.queue_items.is_empty()
        && output.review_items.is_empty()
        && output.adjudication_attempts.is_empty()
    {
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

    if !output.adjudication_attempts.is_empty() {
        println!("recent adjudication attempts:");
        for item in &output.adjudication_attempts {
            println!(
                "  attempt={} outcome={} persisted={} reason={} path={} artefact={} symbol={} generation={} model={} observed_at_unix={}",
                item.attempt_id,
                item.outcome,
                item.assignment_write_persisted,
                item.reason,
                item.path.as_deref().unwrap_or("<unknown>"),
                item.artefact_id.as_deref().unwrap_or("<unknown>"),
                item.symbol_id.as_deref().unwrap_or("<unknown>"),
                item.generation,
                item.model_descriptor,
                item.observed_at_unix,
            );
            if let Some(summary) = item
                .reasoning_summary
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                println!("    reasoning={summary}");
            }
            if let Some(failure) = item
                .failure_message
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                println!("    failure={failure}");
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

pub(super) fn format_roles_classify_output(
    output: &RolesClassifyOutput,
    json_output: bool,
) -> Result<String> {
    if json_output {
        return serde_json::to_string_pretty(output)
            .context("serialising architecture roles classify output as JSON");
    }

    let mut lines = vec![
        "architecture roles classified".to_string(),
        format!(
            "roles: full_reconcile={} affected_paths={} refreshed_paths={} removed_paths={} skipped_unchanged_paths={}",
            output.roles.full_reconcile,
            output.roles.affected_paths,
            output.roles.refreshed_paths,
            output.roles.removed_paths,
            output.roles.skipped_unchanged_paths,
        ),
        format!(
            "facts: written={} deleted={}",
            output.roles.facts_written, output.roles.facts_deleted
        ),
        format!(
            "signals: written={} deleted={}",
            output.roles.signals_written, output.roles.signals_deleted
        ),
        format!(
            "assignments: written={} marked_stale={} history_rows={}",
            output.roles.assignments_written,
            output.roles.assignments_marked_stale,
            output.roles.assignment_history_rows,
        ),
        format!(
            "adjudication: candidates={} selected={} enqueued={} deduped={}",
            output.roles.adjudication_candidates,
            output.role_adjudication_selected,
            output.role_adjudication_enqueued,
            output.role_adjudication_deduped,
        ),
    ];
    lines.extend(
        output
            .warnings
            .iter()
            .map(|warning| format!("warning: {warning}")),
    );
    Ok(lines.join("\n"))
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

#[cfg(test)]
mod deterministic_tests;
