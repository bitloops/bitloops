use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;
#[cfg(test)]
use serde_json::json;
use std::collections::BTreeSet;
use std::path::Path;

use crate::capability_packs::architecture_graph::roles::classifier::{
    ArchitectureRoleClassificationInput, ArchitectureRoleClassificationScope,
    classify_architecture_roles_for_current_state,
};
use crate::capability_packs::architecture_graph::roles::fact_extraction::RelationalArchitectureRoleCurrentStateSource;
use crate::capability_packs::architecture_graph::roles::migrations::{
    apply_proposal, create_alias_proposal, create_deprecate_role_proposal,
    create_merge_role_proposal, create_remove_role_proposal, create_rename_role_proposal,
    create_rule_activate_proposal, create_rule_disable_proposal, create_rule_draft_proposal,
    create_rule_edit_proposal, create_split_role_proposal, show_proposal,
};
use crate::capability_packs::architecture_graph::roles::taxonomy::{
    RoleSplitSpecFile, RuleSpecFile,
};
use crate::capability_packs::architecture_graph::roles::{
    ArchitectureRoleReconcileMetrics, RoleAdjudicationEnqueueMetrics, default_queue_store,
    enqueue_adjudication_requests,
};
#[cfg(test)]
use crate::capability_packs::architecture_graph::types::{
    ARCHITECTURE_GRAPH_CAPABILITY_ID, ARCHITECTURE_GRAPH_ROLE_ADJUDICATION_MAILBOX,
};
use crate::capability_packs::semantic_clones::{
    runtime_config::resolve_semantic_clones_config,
    types::SEMANTIC_CLONES_CAPABILITY_ID,
    workplane::{
        architecture_embedding_jobs_for_artefacts, architecture_embedding_path_cleanup_jobs,
        load_effective_mailbox_intent_for_repo,
    },
};
use crate::host::capability_host::{CapabilityConfigView, DevqlCapabilityHost};
use crate::host::devql::{RelationalStorage, esc_pg, sql_string_list_pg};

use super::*;

mod roles_seed;
mod roles_status;
mod support;

use roles_seed::{
    BootstrapCommandSummary, SeedCommandSummary, activate_seeded_draft_rules,
    configured_seed_profile_name, ensure_seed_owned_draft_rules_exist,
    format_bootstrap_command_output, format_seed_command_output, seed_architecture_roles,
};
#[cfg(test)]
use roles_seed::{
    SeedRecoverySummary, SeedRuleActivationSummary, SeedSummary,
    architecture_seed_request_diagnostics, ensure_seed_alias, persist_seeded_taxonomy,
};
#[cfg(test)]
use support::sql_text;
use support::{cli_provenance, load_json_spec, print_apply_summary, print_proposal_summary};

use roles_status::run_architecture_roles_status;

#[cfg(test)]
use roles_status::{
    load_role_adjudication_attempt_items, load_role_review_items,
    role_adjudication_queue_item_from_job,
};

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
                let _ = enqueue_architecture_embedding_refresh(
                    &scope.repo.repo_id,
                    &scope.repo_root,
                    true,
                    &[],
                    &[],
                    context,
                )
                .await?;
                print_apply_summary(&summary);
                Ok(())
            }
        },
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct RolesClassifyOutput {
    roles: ArchitectureRoleReconcileMetrics,
    architecture_embedding_selected: u64,
    architecture_embedding_enqueued: u64,
    architecture_embedding_deduped: u64,
    role_adjudication_selected: usize,
    role_adjudication_enqueued: usize,
    role_adjudication_deduped: usize,
    warnings: Vec<String>,
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
    let role_current_state = RelationalArchitectureRoleCurrentStateSource::new(
        &scope.repo.repo_id,
        context.relational.as_ref(),
    );

    let outcome = classify_architecture_roles_for_current_state(
        context.storage.as_ref(),
        &role_current_state,
        ArchitectureRoleClassificationInput {
            repo_id: &scope.repo.repo_id,
            generation_seq,
            scope: role_scope,
            files: &files,
        },
    )
    .await?;
    let mut warnings = outcome.warnings;
    let architecture_embedding_metrics = match enqueue_architecture_embedding_refresh(
        &scope.repo.repo_id,
        &scope.repo_root,
        outcome.metrics.full_reconcile,
        &outcome.architecture_embedding_refresh_paths,
        &outcome.architecture_embedding_cleanup_paths,
        context,
    )
    .await
    {
        Ok(metrics) => metrics,
        Err(err) => {
            warnings.push(format!(
                "Architecture embedding refresh enqueue failed: {err:#}"
            ));
            ArchitectureEmbeddingEnqueueMetrics::default()
        }
    };
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
        architecture_embedding_selected: architecture_embedding_metrics.selected,
        architecture_embedding_enqueued: architecture_embedding_metrics.enqueued,
        architecture_embedding_deduped: architecture_embedding_metrics.deduped,
        role_adjudication_selected: adjudication_metrics.selected,
        role_adjudication_enqueued: adjudication_metrics.enqueued,
        role_adjudication_deduped: adjudication_metrics.deduped,
        warnings,
    })
}

#[derive(Debug, Clone, Copy, Default)]
struct ArchitectureEmbeddingEnqueueMetrics {
    selected: u64,
    enqueued: u64,
    deduped: u64,
}

async fn enqueue_architecture_embedding_refresh(
    repo_id: &str,
    repo_root: &Path,
    full_reconcile: bool,
    refresh_paths: &[String],
    cleanup_paths: &[String],
    context: &crate::host::capability_host::CurrentStateConsumerContext,
) -> Result<ArchitectureEmbeddingEnqueueMetrics> {
    let semantic_clones_config = resolve_semantic_clones_config(&CapabilityConfigView::new(
        SEMANTIC_CLONES_CAPABILITY_ID,
        context.config_root.clone(),
    ));
    let intent = load_effective_mailbox_intent_for_repo(repo_root, &semantic_clones_config)
        .context("loading semantic-clones mailbox intent for architecture embedding refresh")?;
    if !intent.architecture_embeddings_active {
        return Ok(ArchitectureEmbeddingEnqueueMetrics::default());
    }

    let artefact_ids = if full_reconcile {
        load_current_embedding_artefact_ids(context.storage.as_ref(), repo_id, None).await?
    } else {
        load_current_embedding_artefact_ids(context.storage.as_ref(), repo_id, Some(refresh_paths))
            .await?
    };
    let mut jobs = architecture_embedding_jobs_for_artefacts(&artefact_ids)?;
    jobs.extend(architecture_embedding_path_cleanup_jobs(cleanup_paths)?);
    let selected = jobs.len() as u64;
    if jobs.is_empty() {
        return Ok(ArchitectureEmbeddingEnqueueMetrics::default());
    }
    let result = context.workplane.enqueue_jobs(jobs)?;
    Ok(ArchitectureEmbeddingEnqueueMetrics {
        selected,
        enqueued: result.inserted_jobs,
        deduped: result.updated_jobs,
    })
}

async fn load_current_embedding_artefact_ids(
    relational: &RelationalStorage,
    repo_id: &str,
    paths: Option<&[String]>,
) -> Result<Vec<String>> {
    let path_filter = paths
        .filter(|paths| !paths.is_empty())
        .map(|paths| format!("AND current.path IN ({})", sql_string_list_pg(paths)))
        .unwrap_or_default();
    let rows = relational
        .query_rows(&format!(
            "SELECT current.artefact_id \
             FROM artefacts_current current \
             JOIN current_file_state state ON state.repo_id = current.repo_id AND state.path = current.path \
             WHERE current.repo_id = '{}' \
               {path_filter} \
               AND state.analysis_mode = 'code' \
               AND LOWER(COALESCE(current.canonical_kind, COALESCE(current.language_kind, 'symbol'))) <> 'import' \
             ORDER BY current.path, current.start_line, current.symbol_id, COALESCE(current.start_byte, 0), current.artefact_id",
            esc_pg(repo_id),
        ))
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            row.get("artefact_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect())
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
        let activation = activate_seeded_draft_rules(
            context.storage.as_ref(),
            context.relational.as_ref(),
            &scope.repo.repo_id,
            &seed.profile_name,
            cli_provenance("seed_activate_rules"),
        )
        .await?;
        if !args.classify {
            let _ = enqueue_architecture_embedding_refresh(
                &scope.repo.repo_id,
                &scope.repo_root,
                true,
                &[],
                &[],
                context,
            )
            .await?;
        }
        Some(activation)
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
            context.relational.as_ref(),
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
            "roles: full_reconcile={} affected_paths={} refreshed_paths={} removed_paths={} skipped_unchanged_paths={}",
            output.roles.full_reconcile,
            output.roles.affected_paths,
            output.roles.refreshed_paths,
            output.roles.removed_paths,
            output.roles.skipped_unchanged_paths,
        ),
        format!(
            "coverage: targets={} active={} review={} conflict={} unknown={} deterministic_ratio={:.3}",
            output.roles.target_count,
            output.roles.deterministic_active_targets,
            output.roles.deterministic_needs_review_targets,
            output.roles.deterministic_conflict_targets,
            output.roles.deterministic_unassigned_targets,
            output.roles.deterministic_coverage_ratio,
        ),
        format!(
            "unknown policy: total={} suppressed_non_role={} rule_mining_eligible={} adjudication_escalated={}",
            output.roles.unknown_targets_total,
            output.roles.unknown_targets_suppressed_non_role,
            output.roles.unknown_targets_rule_mining_eligible,
            output.roles.unknown_targets_adjudication_escalated,
        ),
        format!(
            "rule mining: clusters={} representative_targets={}",
            output.roles.role_mining_clusters, output.roles.role_mining_representative_targets,
        ),
        format!(
            "architecture embeddings: selected={} enqueued={} deduped={}",
            output.architecture_embedding_selected,
            output.architecture_embedding_enqueued,
            output.architecture_embedding_deduped,
        ),
        format!(
            "adjudication: candidates={} selected={} enqueued={} deduped={}",
            output.roles.adjudication_candidates,
            output.role_adjudication_selected,
            output.role_adjudication_enqueued,
            output.role_adjudication_deduped,
        ),
        format!(
            "adjudication reasons: unknown={} high_impact={} low_confidence={} conflict={} repeated_suppressed={}",
            output.roles.unknown_adjudication_candidates,
            output.roles.high_impact_adjudication_candidates,
            output.roles.low_confidence_adjudication_candidates,
            output.roles.conflict_adjudication_candidates,
            output.roles.repeated_adjudication_suppressed,
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

#[cfg(test)]
mod deterministic_tests;
