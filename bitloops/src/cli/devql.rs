use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::io::Write;
use std::time::Instant;

use crate::capability_packs::knowledge::run_knowledge_versions_via_host;
use crate::devql_transport::{
    SlimCliRepoScope, discover_slim_cli_repo_scope, is_repo_root_discovery_error,
};
use crate::host::devql::{
    CheckpointFileSnapshotBackfillOptions, DevqlConfig, GraphqlCompileMode, ParsedDevqlQuery,
    SyncSummary, compile_devql_to_graphql_with_mode, compile_query_document, format_query_output,
    parse_devql_query, run_capability_packs_report, run_checkpoint_file_snapshot_backfill,
    use_raw_graphql_mode,
};

mod args;
pub(crate) mod graphql;
mod knowledge;
mod test_harness;

#[cfg(test)]
mod tests;

pub use crate::host::devql::run_connection_status;
pub use args::{
    DevqlArgs, DevqlCheckpointFileSnapshotsArgs, DevqlCommand, DevqlConnectionStatusArgs,
    DevqlInitArgs, DevqlKnowledgeAddArgs, DevqlKnowledgeArgs, DevqlKnowledgeAssociateArgs,
    DevqlKnowledgeCommand, DevqlKnowledgeRefArgs, DevqlPacksArgs, DevqlProjectionArgs,
    DevqlProjectionCommand, DevqlQueryArgs, DevqlSchemaArgs, DevqlTaskCancelArgs,
    DevqlTaskEnqueueArgs, DevqlTaskKindArg, DevqlTaskListArgs, DevqlTaskPauseArgs,
    DevqlTaskResumeArgs, DevqlTaskStatusArg, DevqlTaskStatusArgs, DevqlTaskWatchArgs,
    DevqlTasksArgs, DevqlTasksCommand, DevqlTestHarnessArgs, DevqlTestHarnessCommand,
    DevqlTestHarnessIngestCoverageArgs, DevqlTestHarnessIngestCoverageBatchArgs,
    DevqlTestHarnessIngestResultsArgs, DevqlTestHarnessIngestTestsArgs,
};

pub(crate) const MISSING_SUBCOMMAND_MESSAGE: &str = "missing subcommand. Use one of: `bitloops devql init`, `bitloops devql tasks enqueue`, `bitloops devql tasks watch`, `bitloops devql tasks status`, `bitloops devql tasks list`, `bitloops devql tasks pause`, `bitloops devql tasks resume`, `bitloops devql tasks cancel`, `bitloops devql projection checkpoint-file-snapshots`, `bitloops devql schema`, `bitloops devql query`, `bitloops devql connection-status`, `bitloops devql packs`, `bitloops devql knowledge add`, `bitloops devql knowledge associate`, `bitloops devql knowledge refresh`, `bitloops devql knowledge versions`, `bitloops devql test-harness ingest-tests`, `bitloops devql test-harness ingest-coverage`, `bitloops devql test-harness ingest-coverage-batch`, `bitloops devql test-harness ingest-results`";
const SCHEMA_SCOPE_REQUIRED_MESSAGE: &str = "`bitloops devql schema` requires a Git repository scope. Run it from within a repository or use `bitloops devql schema --global`.";

fn format_schema_sdl_output(args: &DevqlSchemaArgs, sdl: &str) -> String {
    if args.human {
        sdl.to_string()
    } else {
        minify_schema_sdl(sdl)
    }
}

async fn write_schema_sdl<F, W>(
    args: &DevqlSchemaArgs,
    writer: &mut W,
    discover_scope: F,
) -> Result<()>
where
    F: FnOnce() -> Result<SlimCliRepoScope>,
    W: Write,
{
    let sdl = if args.global {
        graphql::fetch_global_schema_sdl_via_daemon().await?
    } else {
        let scope = discover_scope().map_err(map_schema_scope_error)?;
        graphql::fetch_slim_schema_sdl_via_daemon(&scope).await?
    };

    writer
        .write_all(format_schema_sdl_output(args, &sdl).as_bytes())
        .context("writing DevQL schema SDL")
}

fn map_schema_scope_error(err: anyhow::Error) -> anyhow::Error {
    if is_repo_root_discovery_error(&err) {
        anyhow!(SCHEMA_SCOPE_REQUIRED_MESSAGE)
    } else {
        err
    }
}

fn minify_schema_sdl(sdl: &str) -> String {
    #[derive(Copy, Clone, Eq, PartialEq)]
    enum State {
        Normal,
        String,
        BlockString,
    }

    fn starts_with_triple_quotes(chars: &[char], index: usize) -> bool {
        chars.get(index) == Some(&'"')
            && chars.get(index + 1) == Some(&'"')
            && chars.get(index + 2) == Some(&'"')
    }

    fn push_pending_space(
        output: &mut String,
        next: char,
        pending_space: &mut bool,
        last_emitted: &mut Option<char>,
    ) {
        if !*pending_space || last_emitted.is_none() {
            *pending_space = false;
            return;
        }

        let previous = *last_emitted;
        if previous == Some('{')
            || next == '}'
            || matches!(previous, Some(' ' | '\n' | '\r' | '\t'))
        {
            *pending_space = false;
            return;
        }

        output.push(' ');
        *last_emitted = Some(' ');
        *pending_space = false;
    }

    let chars = sdl.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(sdl.len());
    let mut state = State::Normal;
    let mut pending_space = false;
    let mut last_emitted = None;
    let mut index = 0usize;

    while index < chars.len() {
        match state {
            State::Normal => {
                if starts_with_triple_quotes(&chars, index) {
                    push_pending_space(&mut output, '"', &mut pending_space, &mut last_emitted);
                    output.push_str("\"\"\"");
                    last_emitted = Some('"');
                    index += 3;
                    state = State::BlockString;
                } else if chars[index] == '"' {
                    push_pending_space(&mut output, '"', &mut pending_space, &mut last_emitted);
                    output.push('"');
                    last_emitted = Some('"');
                    index += 1;
                    state = State::String;
                } else if chars[index].is_whitespace() {
                    pending_space = true;
                    index += 1;
                } else {
                    push_pending_space(
                        &mut output,
                        chars[index],
                        &mut pending_space,
                        &mut last_emitted,
                    );
                    output.push(chars[index]);
                    last_emitted = Some(chars[index]);
                    index += 1;
                }
            }
            State::String => {
                let ch = chars[index];
                output.push(ch);
                last_emitted = Some(ch);
                index += 1;
                if ch == '\\' {
                    if let Some(next) = chars.get(index) {
                        output.push(*next);
                        last_emitted = Some(*next);
                        index += 1;
                    }
                } else if ch == '"' {
                    state = State::Normal;
                }
            }
            State::BlockString => {
                if starts_with_triple_quotes(&chars, index) {
                    output.push_str("\"\"\"");
                    last_emitted = Some('"');
                    index += 3;
                    state = State::Normal;
                } else {
                    output.push(chars[index]);
                    last_emitted = Some(chars[index]);
                    index += 1;
                }
            }
        }
    }

    if !output.ends_with('\n') {
        output.push('\n');
    }

    output
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
    let Some(command) = args.command else {
        bail!(MISSING_SUBCOMMAND_MESSAGE);
    };

    let command = match command {
        DevqlCommand::Schema(args) => {
            return write_schema_sdl(&args, schema_writer, discover_scope).await;
        }
        DevqlCommand::ConnectionStatus(_) => return run_connection_status().await,
        command => command,
    };

    let scope = discover_scope()?;
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

    if let DevqlCommand::TestHarness(args) = command {
        return test_harness::run(args, &repo_root).await;
    }

    let cfg = DevqlConfig::from_env(repo_root, repo)?;

    match command {
        DevqlCommand::Init(_) => graphql::run_init_via_graphql(&scope).await,
        DevqlCommand::Tasks(args) => run_tasks_command(&scope, args).await,
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
            let parsed_query = (!use_raw_graphql)
                .then(|| parse_devql_query(&args.query))
                .transpose()?;
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
            let output = match format_query_output(
                &data,
                args.compact,
                use_raw_graphql,
                parsed_query.as_ref(),
            ) {
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
        DevqlCommand::Schema(_) => unreachable!("handled before repo setup"),
        DevqlCommand::ConnectionStatus(_) => unreachable!("handled before repo setup"),
        DevqlCommand::Knowledge(_) => unreachable!("handled before cfg setup"),
        DevqlCommand::TestHarness(_) => unreachable!("handled before cfg setup"),
    }
}

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
            let result = graphql::pause_task_queue_via_graphql(scope, args.reason.as_deref()).await?;
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
                bail!("`--backfill` is only supported for `bitloops devql tasks enqueue --kind ingest`");
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

pub(crate) fn format_task_queue_submission(task: &graphql::TaskGraphqlRecord, merged: bool) -> String {
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
    format!(
        "task complete: {} {}",
        task.kind.to_ascii_lowercase(),
        task.task_id
    )
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
