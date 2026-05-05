use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::io::Write;
use std::time::Instant;

use crate::capability_packs::knowledge::run_knowledge_versions_via_host;
use crate::devql_transport::{
    SlimCliRepoScope, discover_slim_cli_repo_scope, is_repo_root_discovery_error,
};
use crate::host::devql::{
    AnalyticsRepoScope, CheckpointFileSnapshotBackfillOptions, DevqlConfig, GraphqlCompileMode,
    ParsedDevqlQuery, SyncSummary, compile_devql_to_graphql_with_mode, compile_query_document,
    execute_analytics_sql, format_analytics_sql_result_table, format_query_output,
    parse_devql_query, run_capability_packs_report, run_checkpoint_file_snapshot_backfill,
    use_raw_graphql_mode,
};

mod architecture;
mod args;
pub(crate) mod graphql;
mod knowledge;
mod navigation_context;
mod test_harness;

#[cfg(test)]
use navigation_context::{
    format_navigation_context_materialisation, format_navigation_context_status, minify_schema_sdl,
};

#[cfg(test)]
mod tests;

pub use crate::host::devql::run_connection_status;
pub use args::{
    DevqlAnalyticsArgs, DevqlAnalyticsCommand, DevqlAnalyticsSqlArgs, DevqlArchitectureArgs,
    DevqlArchitectureCommand, DevqlArchitectureRolesAliasArgs, DevqlArchitectureRolesAliasCommand,
    DevqlArchitectureRolesAliasCreateArgs, DevqlArchitectureRolesArgs,
    DevqlArchitectureRolesCommand, DevqlArchitectureRolesDeprecateArgs,
    DevqlArchitectureRolesMergeArgs, DevqlArchitectureRolesProposalArgs,
    DevqlArchitectureRolesProposalCommand, DevqlArchitectureRolesProposalRefArgs,
    DevqlArchitectureRolesRemoveArgs, DevqlArchitectureRolesRenameArgs,
    DevqlArchitectureRolesRulesArgs, DevqlArchitectureRolesRulesCommand,
    DevqlArchitectureRolesRulesDraftArgs, DevqlArchitectureRolesRulesEditArgs,
    DevqlArchitectureRolesRulesRefArgs, DevqlArchitectureRolesSeedArgs,
    DevqlArchitectureRolesSplitArgs, DevqlArchitectureRolesStatusArgs, DevqlArgs,
    DevqlCheckpointFileSnapshotsArgs, DevqlCommand, DevqlConnectionStatusArgs, DevqlInitArgs,
    DevqlKnowledgeAddArgs, DevqlKnowledgeArgs, DevqlKnowledgeAssociateArgs, DevqlKnowledgeCommand,
    DevqlKnowledgeRefArgs, DevqlNavigationContextAcceptArgs, DevqlNavigationContextArgs,
    DevqlNavigationContextCommand, DevqlNavigationContextMaterialiseArgs,
    DevqlNavigationContextStatusArg, DevqlNavigationContextStatusArgs, DevqlPacksArgs,
    DevqlProjectionArgs, DevqlProjectionCommand, DevqlQueryArgs, DevqlSchemaArgs,
    DevqlTaskCancelArgs, DevqlTaskEnqueueArgs, DevqlTaskKindArg, DevqlTaskListArgs,
    DevqlTaskPauseArgs, DevqlTaskResumeArgs, DevqlTaskStatusArg, DevqlTaskStatusArgs,
    DevqlTaskWatchArgs, DevqlTasksArgs, DevqlTasksCommand, DevqlTestHarnessArgs,
    DevqlTestHarnessCommand, DevqlTestHarnessIngestCoverageArgs,
    DevqlTestHarnessIngestCoverageBatchArgs, DevqlTestHarnessIngestResultsArgs,
    DevqlTestHarnessIngestTestsArgs,
};

pub(crate) const MISSING_SUBCOMMAND_MESSAGE: &str = "missing subcommand. Use one of: `bitloops devql init`, `bitloops devql analytics sql`, `bitloops devql tasks enqueue`, `bitloops devql tasks watch`, `bitloops devql tasks status`, `bitloops devql tasks list`, `bitloops devql tasks pause`, `bitloops devql tasks resume`, `bitloops devql tasks cancel`, `bitloops devql projection checkpoint-file-snapshots`, `bitloops devql schema`, `bitloops devql query`, `bitloops devql connection-status`, `bitloops devql packs`, `bitloops devql architecture roles seed`, `bitloops devql architecture roles status`, `bitloops devql architecture roles rename`, `bitloops devql architecture roles rules draft`, `bitloops devql architecture roles proposal show`, `bitloops devql knowledge add`, `bitloops devql knowledge associate`, `bitloops devql knowledge refresh`, `bitloops devql knowledge versions`, `bitloops devql navigation-context status`, `bitloops devql navigation-context materialise`, `bitloops devql navigation-context accept`, `bitloops devql test-harness ingest-tests`, `bitloops devql test-harness ingest-coverage`, `bitloops devql test-harness ingest-coverage-batch`, `bitloops devql test-harness ingest-results`";
const SCHEMA_SCOPE_REQUIRED_MESSAGE: &str = "`bitloops devql schema` requires a Git repository scope. Run it from within a repository or use `bitloops devql schema --global`.";

async fn run_tasks_command(scope: &SlimCliRepoScope, args: DevqlTasksArgs) -> Result<()> {
    match args.command {
        DevqlTasksCommand::Enqueue(args) => run_task_enqueue(scope, args).await,
        DevqlTasksCommand::Watch(args) => run_task_watch(scope, args).await,
        DevqlTasksCommand::Status(_) => {
            let status = graphql::task_queue_status_via_graphql(scope).await?;
            print_task_queue_status(&status);
            Ok(())
        }
        DevqlTasksCommand::List(args) => {
            let tasks = graphql::list_tasks_via_graphql(
                scope,
                args.kind.as_ref().map(task_kind_arg_name),
                args.status.as_ref().map(task_status_arg_name),
                args.limit,
            )
            .await?;
            print_task_list(&tasks);
            Ok(())
        }
        DevqlTasksCommand::Pause(args) => {
            let result =
                graphql::pause_task_queue_via_graphql(scope, args.reason.as_deref()).await?;
            println!("{}", format_task_queue_control_result(&result));
            Ok(())
        }
        DevqlTasksCommand::Resume(_) => {
            let result = graphql::resume_task_queue_via_graphql(scope).await?;
            println!("{}", format_task_queue_control_result(&result));
            Ok(())
        }
        DevqlTasksCommand::Cancel(args) => {
            let task = graphql::cancel_task_via_graphql(scope, args.task_id.as_str()).await?;
            println!("{}", format_task_brief(&task));
            Ok(())
        }
    }
}

async fn run_task_enqueue(scope: &SlimCliRepoScope, args: DevqlTaskEnqueueArgs) -> Result<()> {
    validate_task_enqueue_args(&args)?;
    let (task, merged) = match args.kind {
        DevqlTaskKindArg::Sync => {
            graphql::enqueue_sync_task_via_graphql(
                scope,
                args.full,
                args.paths,
                args.repair,
                args.validate,
                "manual_cli",
                args.require_daemon,
            )
            .await?
        }
        DevqlTaskKindArg::Ingest => {
            graphql::enqueue_ingest_task_via_graphql(scope, args.backfill, args.require_daemon)
                .await?
        }
    };

    if args.status {
        if let Some(task) = graphql::watch_task_via_graphql(scope, task.clone()).await? {
            println!("{}", format_task_completion_summary(&task));
        }
    } else {
        println!("{}", format_task_queue_submission(&task, merged));
    }
    Ok(())
}

async fn run_task_watch(scope: &SlimCliRepoScope, args: DevqlTaskWatchArgs) -> Result<()> {
    if let Some(task) =
        graphql::watch_task_id_via_graphql(scope, args.task_id.as_str(), args.require_daemon)
            .await?
    {
        println!("{}", format_task_completion_summary(&task));
    }
    Ok(())
}

fn validate_task_enqueue_args(args: &DevqlTaskEnqueueArgs) -> Result<()> {
    match args.kind {
        DevqlTaskKindArg::Sync => {
            if args.backfill.is_some() {
                bail!(
                    "`--backfill` is only supported for `bitloops devql tasks enqueue --kind ingest`"
                );
            }
        }
        DevqlTaskKindArg::Ingest => {
            if args.full || args.paths.is_some() || args.repair || args.validate {
                bail!(
                    "sync mode flags are only supported for `bitloops devql tasks enqueue --kind sync`"
                );
            }
        }
    }
    Ok(())
}

fn task_kind_arg_name(kind: &DevqlTaskKindArg) -> &'static str {
    match kind {
        DevqlTaskKindArg::Sync => "sync",
        DevqlTaskKindArg::Ingest => "ingest",
    }
}

fn task_status_arg_name(status: &DevqlTaskStatusArg) -> &'static str {
    match status {
        DevqlTaskStatusArg::Queued => "queued",
        DevqlTaskStatusArg::Running => "running",
        DevqlTaskStatusArg::Completed => "completed",
        DevqlTaskStatusArg::Failed => "failed",
        DevqlTaskStatusArg::Cancelled => "cancelled",
    }
}

pub async fn run(args: DevqlArgs) -> Result<()> {
    if matches!(args.command.as_ref(), Some(DevqlCommand::Schema(_))) {
        let stdout = std::io::stdout();
        let mut writer = stdout.lock();
        return run_with_scope_discovery(args, &mut writer, || discover_slim_cli_repo_scope(None))
            .await;
    }

    let mut writer = std::io::stdout();
    run_with_scope_discovery(args, &mut writer, || discover_slim_cli_repo_scope(None)).await
}

async fn run_with_scope_discovery<F, W>(
    args: DevqlArgs,
    schema_writer: &mut W,
    discover_scope: F,
) -> Result<()>
where
    F: FnOnce() -> Result<SlimCliRepoScope>,
    W: Write,
{
    if matches!(args.command.as_ref(), Some(DevqlCommand::Architecture(_))) {
        return run_architecture_with_scope_discovery(args, discover_scope).await;
    }

    navigation_context::run_with_scope_discovery(args, schema_writer, discover_scope).await
}

async fn run_architecture_with_scope_discovery<F>(args: DevqlArgs, discover_scope: F) -> Result<()>
where
    F: FnOnce() -> Result<SlimCliRepoScope>,
{
    let Some(DevqlCommand::Architecture(args)) = args.command else {
        unreachable!("run_architecture_with_scope_discovery only handles architecture commands");
    };

    let scope = discover_scope()?;
    architecture::run_architecture_command(&scope, args).await
}

pub(crate) fn format_task_queue_submission(
    task: &graphql::TaskGraphqlRecord,
    merged: bool,
) -> String {
    let mut line = format!(
        "task queued: task={} repo={} kind={}",
        task.task_id,
        task.repo_name,
        task.kind.to_ascii_lowercase()
    );
    if let Some(sync_spec) = task.sync_spec.as_ref() {
        line.push_str(&format!(" mode={}", sync_spec.mode));
    }
    if let Some(ingest_spec) = task.ingest_spec.as_ref()
        && let Some(backfill) = ingest_spec.backfill
    {
        line.push_str(&format!(" backfill={backfill}"));
    }
    if let Some(summary_spec) = task.summary_bootstrap_spec.as_ref() {
        line.push_str(&format!(" action={}", summary_spec.action));
    }
    if merged {
        line.push_str(" (merged into existing task)");
    }
    line
}

pub(crate) fn format_task_completion_summary(task: &graphql::TaskGraphqlRecord) -> String {
    if let Some(summary) = task.sync_summary() {
        return format_sync_completion_summary(&summary);
    }
    if let Some(summary) = task.ingest_result.as_ref() {
        return crate::host::devql::format_ingestion_summary(summary);
    }
    if let Some(result) = task.embeddings_bootstrap_result.as_ref() {
        return format_embeddings_bootstrap_completion_summary(result);
    }
    if let Some(result) = task.summary_bootstrap_result.as_ref() {
        return format_summary_bootstrap_completion_summary(result);
    }
    format!(
        "task complete: {} {}",
        task.kind.to_ascii_lowercase(),
        task.task_id
    )
}

fn format_embeddings_bootstrap_completion_summary(
    result: &graphql::EmbeddingsBootstrapResultGraphqlRecord,
) -> String {
    let mut lines = Vec::new();

    if let (Some(version), Some(binary_path)) =
        (result.version.as_deref(), result.binary_path.as_deref())
    {
        let status_line = if result.freshly_installed {
            format!("Installed managed standalone `bitloops-local-embeddings` runtime {version}.")
        } else {
            format!(
                "Managed standalone `bitloops-local-embeddings` runtime {version} already installed."
            )
        };
        lines.push(status_line);
        lines.push(format!("Binary path: {binary_path}"));
    }

    lines.push(result.message.clone());

    if let Some(cache_dir) = result.cache_dir.as_deref() {
        lines.push(format!("Cache directory: {cache_dir}"));
    }
    if let (Some(runtime_name), Some(model_name)) =
        (result.runtime_name.as_deref(), result.model_name.as_deref())
    {
        lines.push(format!("Runtime: {runtime_name} {model_name}"));
    }

    lines.join("\n")
}

fn format_summary_bootstrap_completion_summary(
    result: &graphql::SummaryBootstrapResultGraphqlRecord,
) -> String {
    let mut lines = vec![result.message.clone()];
    if let Some(model_name) = result.model_name.as_deref() {
        lines.push(format!("Model: {model_name}"));
    }
    lines.push(format!("Outcome: {}", result.outcome_kind));
    lines.join("\n")
}

fn format_task_queue_control_result(result: &graphql::TaskQueueControlGraphqlRecord) -> String {
    let mut line = result.message.clone();
    line.push_str(&format!(
        " (repo={}, paused={})",
        result.repo_id,
        if result.paused { "yes" } else { "no" }
    ));
    if let Some(reason) = result.paused_reason.as_ref() {
        line.push_str(&format!(", reason={reason}"));
    }
    line
}

fn format_task_brief(task: &graphql::TaskGraphqlRecord) -> String {
    format!(
        "task {}: kind={} status={} repo={}",
        task.task_id,
        task.kind.to_ascii_lowercase(),
        task.status.to_ascii_lowercase(),
        task.repo_name
    )
}

fn print_task_queue_status(status: &graphql::TaskQueueGraphqlRecord) {
    println!("DevQL task queue");
    println!(
        "state: {}",
        if status.paused { "paused" } else { "running" }
    );
    println!("queued: {}", status.queued_tasks);
    println!("running: {}", status.running_tasks);
    println!("failed: {}", status.failed_tasks);
    println!("completed_recent: {}", status.completed_recent_tasks);
    if let Some(reason) = status.paused_reason.as_ref() {
        println!("pause_reason: {reason}");
    }
    if let Some(action) = status.last_action.as_ref() {
        println!("last_action: {action}");
    }
    for counts in &status.by_kind {
        println!(
            "kind={} queued={} running={} failed={} completed_recent={}",
            counts.kind.to_ascii_lowercase(),
            counts.queued_tasks,
            counts.running_tasks,
            counts.failed_tasks,
            counts.completed_recent_tasks
        );
    }
    if !status.current_repo_tasks.is_empty() {
        println!("current_repo_tasks:");
        for task in &status.current_repo_tasks {
            println!("{}", format_task_brief(task));
        }
    }
}

fn print_task_list(tasks: &[graphql::TaskGraphqlRecord]) {
    if tasks.is_empty() {
        println!("no tasks");
        return;
    }
    for task in tasks {
        println!("{}", format_task_brief(task));
    }
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
