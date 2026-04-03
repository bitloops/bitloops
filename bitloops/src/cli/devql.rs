use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::time::Instant;

use crate::capability_packs::knowledge::run_knowledge_versions_via_host;
use crate::devql_transport::{SlimCliRepoScope, discover_slim_cli_repo_scope};
use crate::host::devql::{
    CheckpointFileSnapshotBackfillOptions, DevqlConfig, GraphqlCompileMode, ParsedDevqlQuery,
    SyncSummary, compile_devql_to_graphql_with_mode, compile_query_document, format_query_output,
    parse_devql_query, run_capability_packs_report, run_checkpoint_file_snapshot_backfill,
    use_raw_graphql_mode,
};

mod args;
pub(crate) mod graphql;
mod knowledge;

#[cfg(test)]
mod tests;

pub use crate::host::devql::run_connection_status;
pub use args::{
    DevqlArgs, DevqlCheckpointFileSnapshotsArgs, DevqlCommand, DevqlConnectionStatusArgs,
    DevqlIngestArgs, DevqlInitArgs, DevqlKnowledgeAddArgs, DevqlKnowledgeArgs,
    DevqlKnowledgeAssociateArgs, DevqlKnowledgeCommand, DevqlKnowledgeRefArgs, DevqlPacksArgs,
    DevqlProjectionArgs, DevqlProjectionCommand, DevqlQueryArgs, DevqlSyncArgs,
};

pub(crate) const MISSING_SUBCOMMAND_MESSAGE: &str = "missing subcommand. Use one of: `bitloops devql init`, `bitloops devql ingest`, `bitloops devql sync`, `bitloops devql projection checkpoint-file-snapshots`, `bitloops devql query`, `bitloops devql connection-status`, `bitloops devql packs`, `bitloops devql knowledge add`, `bitloops devql knowledge associate`, `bitloops devql knowledge refresh`, `bitloops devql knowledge versions`";

pub async fn run(args: DevqlArgs) -> Result<()> {
    let Some(command) = args.command else {
        bail!(MISSING_SUBCOMMAND_MESSAGE);
    };

    if matches!(&command, DevqlCommand::ConnectionStatus(_)) {
        return run_connection_status().await;
    }

    let scope = discover_slim_cli_repo_scope(None)?;
    let repo_root = scope.repo_root.clone();
    let repo = scope.repo.clone();

    if let DevqlCommand::Knowledge(args) = command {
        return match args.command {
            DevqlKnowledgeCommand::Add(add) => {
                knowledge::run_knowledge_add_via_graphql(&scope, &add.url, add.commit.as_deref())
                    .await
            }
            DevqlKnowledgeCommand::Associate(associate) => {
                knowledge::run_knowledge_associate_via_graphql(
                    &scope,
                    &associate.source_ref,
                    &associate.target_ref,
                )
                .await
            }
            DevqlKnowledgeCommand::Refresh(refresh) => {
                knowledge::run_knowledge_refresh_via_graphql(&scope, &refresh.knowledge_ref).await
            }
            DevqlKnowledgeCommand::Versions(versions) => {
                run_knowledge_versions_via_host(&repo_root, &repo, &versions.knowledge_ref).await
            }
        };
    }

    let cfg = DevqlConfig::from_env(repo_root, repo)?;
    match command {
        DevqlCommand::Init(_) => graphql::run_init_via_graphql(&scope).await,
        DevqlCommand::Ingest(args) => {
            graphql::run_ingest_via_graphql(&scope, args.max_checkpoints).await
        }
        DevqlCommand::Sync(args) => {
            let (task, merged) = graphql::enqueue_sync_via_graphql(
                &scope,
                args.full,
                args.paths,
                args.repair,
                args.validate,
                "manual_cli",
            )
            .await?;
            if args.status {
                if let Some(summary) =
                    graphql::watch_sync_task_via_graphql(&scope, task.clone()).await?
                {
                    println!("{}", format_sync_completion_summary(&summary));
                }
            } else {
                println!("{}", format_sync_queue_submission(&task, merged));
            }
            Ok(())
        }
        DevqlCommand::Projection(args) => match args.command {
            DevqlProjectionCommand::CheckpointFileSnapshots(backfill) => {
                run_checkpoint_file_snapshot_backfill(
                    &cfg,
                    CheckpointFileSnapshotBackfillOptions {
                        batch_size: backfill.batch_size,
                        max_checkpoints: backfill.max_checkpoints,
                        resume_after: backfill.resume_after,
                        dry_run: backfill.dry_run,
                        emit_progress: true,
                    },
                )
                .await
            }
        },
        DevqlCommand::Query(args) => {
            let use_raw_graphql = use_raw_graphql_mode(&args.query, args.graphql);
            let trace = crate::devql_timing::timings_enabled_from_env()
                .then(crate::devql_timing::TimingTrace::new);

            let compile_started = Instant::now();
            let document = compile_slim_query_document(&args.query, args.graphql, &scope)?;
            if let Some(trace) = trace.as_ref() {
                trace.record(
                    "cli.devql.compile_query_document",
                    compile_started.elapsed(),
                    json!({
                        "inputBytes": args.query.len(),
                        "rawGraphql": use_raw_graphql,
                    }),
                );
            }

            let execute_started = Instant::now();
            let data: serde_json::Value = match crate::daemon::execute_slim_graphql(
                &cfg.repo_root,
                &scope,
                &document,
                serde_json::json!({}),
            )
            .await
            {
                Ok(data) => {
                    if let Some(trace) = trace.as_ref() {
                        trace.record(
                            "cli.devql.execute_graphql",
                            execute_started.elapsed(),
                            Value::Null,
                        );
                    }
                    data
                }
                Err(err) => {
                    if let Some(trace) = trace.as_ref() {
                        trace.record(
                            "cli.devql.execute_graphql",
                            execute_started.elapsed(),
                            json!({
                                "error": format!("{err:#}"),
                            }),
                        );
                        crate::devql_timing::print_summary("cli", &trace.summary_value());
                    }
                    return Err(err);
                }
            };

            let format_started = Instant::now();
            let output = match format_query_output(&data, args.compact, use_raw_graphql) {
                Ok(output) => {
                    if let Some(trace) = trace.as_ref() {
                        trace.record(
                            "cli.devql.format_query_output",
                            format_started.elapsed(),
                            json!({
                                "compact": args.compact,
                                "outputBytes": output.len(),
                            }),
                        );
                    }
                    output
                }
                Err(err) => {
                    if let Some(trace) = trace.as_ref() {
                        trace.record(
                            "cli.devql.format_query_output",
                            format_started.elapsed(),
                            json!({
                                "compact": args.compact,
                                "error": format!("{err:#}"),
                            }),
                        );
                        crate::devql_timing::print_summary("cli", &trace.summary_value());
                    }
                    return Err(err);
                }
            };
            println!("{output}");
            if let Some(trace) = trace.as_ref() {
                crate::devql_timing::print_summary("cli", &trace.summary_value());
            }
            Ok(())
        }
        DevqlCommand::Packs(args) => run_capability_packs_report(
            &cfg,
            args.json,
            args.apply_migrations,
            args.with_health,
            args.with_extensions,
        ),
        DevqlCommand::ConnectionStatus(_) => unreachable!("handled before repo setup"),
        DevqlCommand::Knowledge(_) => unreachable!("handled before cfg setup"),
    }
}

pub(crate) fn format_sync_queue_submission(
    task: &graphql::SyncTaskGraphqlRecord,
    merged: bool,
) -> String {
    let mut line = format!(
        "sync queued: task={} repo={} mode={}",
        task.task_id, task.repo_name, task.mode
    );
    if merged {
        line.push_str(" (merged into existing task)");
    }
    line
}

pub(crate) fn format_sync_completion_summary(summary: &SyncSummary) -> String {
    if summary.mode == "validate" {
        return format_sync_validation_summary(summary);
    }

    let mut message = format!(
        "sync complete: {} added, {} changed, {} removed, {} unchanged, {} cache hits",
        summary.paths_added,
        summary.paths_changed,
        summary.paths_removed,
        summary.paths_unchanged,
        summary.cache_hits,
    );

    let mut diagnostics = Vec::new();
    if summary.mode != "full" {
        diagnostics.push(format!("mode={}", summary.mode));
    }
    if summary.cache_misses > 0 {
        diagnostics.push(format!("{} cache misses", summary.cache_misses));
    }
    if summary.parse_errors > 0 {
        diagnostics.push(format!("{} parse errors", summary.parse_errors));
    }

    if !diagnostics.is_empty() {
        message.push_str(" (");
        message.push_str(&diagnostics.join(", "));
        message.push(')');
    }

    message
}

fn format_sync_validation_summary(summary: &SyncSummary) -> String {
    let Some(validation) = summary.validation.as_ref() else {
        return "sync validation: no report available".to_string();
    };

    if validation.valid {
        return format!(
            "sync validation: clean (artefacts: expected={} actual={}, edges: expected={} actual={})",
            validation.expected_artefacts,
            validation.actual_artefacts,
            validation.expected_edges,
            validation.actual_edges,
        );
    }

    let mut lines = vec![
        "sync validation: drift detected".to_string(),
        format!(
            "artefacts: expected={} actual={} missing={} stale={} mismatched={}",
            validation.expected_artefacts,
            validation.actual_artefacts,
            validation.missing_artefacts,
            validation.stale_artefacts,
            validation.mismatched_artefacts,
        ),
        format!(
            "edges: expected={} actual={} missing={} stale={} mismatched={}",
            validation.expected_edges,
            validation.actual_edges,
            validation.missing_edges,
            validation.stale_edges,
            validation.mismatched_edges,
        ),
    ];

    for file in &validation.files_with_drift {
        lines.push(format!(
            "{}: artefacts missing={} stale={} mismatched={}; edges missing={} stale={} mismatched={}",
            file.path,
            file.missing_artefacts,
            file.stale_artefacts,
            file.mismatched_artefacts,
            file.missing_edges,
            file.stale_edges,
            file.mismatched_edges,
        ));
    }

    lines.join("\n")
}

fn compile_slim_query_document(
    query: &str,
    raw_graphql: bool,
    scope: &SlimCliRepoScope,
) -> Result<String> {
    if use_raw_graphql_mode(query, raw_graphql) {
        return compile_query_document(query, raw_graphql);
    }

    let mut parsed = parse_devql_query(query)?;
    validate_slim_repo_scope(&parsed, scope)?;
    validate_slim_project_scope(&parsed, scope)?;
    if parsed.project_path.is_none() {
        parsed.project_path = scope.project_path.clone();
    }
    compile_devql_to_graphql_with_mode(&parsed, GraphqlCompileMode::Slim)
}

fn validate_slim_repo_scope(parsed: &ParsedDevqlQuery, scope: &SlimCliRepoScope) -> Result<()> {
    let Some(requested_repo) = parsed.repo.as_deref() else {
        return Ok(());
    };
    let requested_repo = requested_repo.trim();
    if requested_repo.is_empty() {
        return Ok(());
    }
    if requested_repo == scope.repo.name
        || requested_repo == scope.repo.identity
        || requested_repo == scope.repo.repo_id
    {
        return Ok(());
    }
    anyhow::bail!(
        "repo(\"{requested_repo}\") does not match the detected CLI repository scope `{}`",
        scope.repo.identity
    )
}

fn validate_slim_project_scope(parsed: &ParsedDevqlQuery, scope: &SlimCliRepoScope) -> Result<()> {
    let Some(requested_project) = parsed.project_path.as_deref() else {
        return Ok(());
    };
    let requested_project = requested_project.trim().trim_matches('/');
    match scope.project_path.as_deref() {
        Some(project_path) if requested_project == project_path => Ok(()),
        Some(project_path) => anyhow::bail!(
            "project(\"{requested_project}\") does not match the detected CLI project scope `{project_path}`"
        ),
        None => anyhow::bail!(
            "project(\"{requested_project}\") does not match the detected CLI scope; run the command from that project directory or use `/devql/global`"
        ),
    }
}
