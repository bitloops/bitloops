use std::path::Path;

use anyhow::{Context, Result, bail};
use reqwest::StatusCode as ReqwestStatusCode;
use serde::de::DeserializeOwned;
use serde_json::json;

#[cfg(test)]
use std::future::Future;

use super::documents::{
    CANCEL_TASK_MUTATION, ENQUEUE_TASK_MUTATION, INIT_SCHEMA_MUTATION, PAUSE_TASK_QUEUE_MUTATION,
    RESUME_TASK_QUEUE_MUTATION, RUNTIME_SNAPSHOT_QUERY, START_INIT_MUTATION, TASK_QUERY,
    TASK_QUEUE_QUERY, TASKS_QUERY,
};
use super::progress::{
    TASK_PROGRESS_POLL_INTERVAL, TASK_RENDER_TICK_INTERVAL, TaskProgressRenderer,
};
use super::subscription::watch_task_via_subscription;
use super::types::{
    CancelTaskMutationData, EnqueueTaskMutationData, InitSchemaMutationData,
    PauseTaskQueueMutationData, ResumeTaskQueueMutationData,
    RuntimeEmbeddingsBootstrapRequestInput, RuntimeSnapshotGraphqlRecord, RuntimeSnapshotQueryData,
    RuntimeStartInitInput, RuntimeSummaryBootstrapRequestInput, StartInitMutationData,
    StartInitResultGraphqlRecord, TaskGraphqlRecord, TaskQueryData, TaskQueueControlGraphqlRecord,
    TaskQueueGraphqlRecord, TaskQueueQueryData, TasksQueryData,
};
use crate::devql_transport::SlimCliRepoScope;
use crate::host::devql::format_init_schema_summary;
use crate::{api::DashboardServerConfig, daemon};

const SYNC_FOLLOW_UP_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
const SYNC_FOLLOW_UP_PENDING_GRACE: std::time::Duration = std::time::Duration::from_secs(10);
const SYNC_FOLLOW_UP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug, Clone)]
struct SyncFollowUpBaseline {
    current_state_failed_runs: u64,
    enrichment: EnrichmentFailureCounts,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub(super) struct EnrichmentFailureCounts {
    pub(super) failed_jobs: u64,
    pub(super) failed_semantic_jobs: u64,
    pub(super) failed_embedding_jobs: u64,
    pub(super) failed_clone_edges_rebuild_jobs: u64,
}

impl EnrichmentFailureCounts {
    fn from_status(status: &daemon::EnrichmentQueueStatus) -> Self {
        Self {
            failed_jobs: status.state.failed_jobs,
            failed_semantic_jobs: status.state.failed_semantic_jobs,
            failed_embedding_jobs: status.state.failed_embedding_jobs,
            failed_clone_edges_rebuild_jobs: status.state.failed_clone_edges_rebuild_jobs,
        }
    }

    fn has_new_failures_since(self, baseline: Self) -> bool {
        self.failed_jobs > baseline.failed_jobs
            || self.failed_semantic_jobs > baseline.failed_semantic_jobs
            || self.failed_embedding_jobs > baseline.failed_embedding_jobs
            || self.failed_clone_edges_rebuild_jobs > baseline.failed_clone_edges_rebuild_jobs
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum DaemonStartPolicy {
    AutoStart,
    RequireRunning,
}

#[cfg(test)]
type TaskDaemonBootstrapHook = dyn Fn(&Path) -> Result<()> + 'static;

#[cfg(test)]
type TaskDaemonBootstrapHookCell = std::cell::RefCell<Option<std::rc::Rc<TaskDaemonBootstrapHook>>>;

#[cfg(test)]
thread_local! {
    static TASK_DAEMON_BOOTSTRAP_HOOK: TaskDaemonBootstrapHookCell =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
thread_local! {
    static GRAPHQL_EXECUTOR_HOOK: std::cell::RefCell<Option<std::rc::Rc<GraphqlExecutorHook>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
type GraphqlExecutorHook =
    dyn Fn(&Path, &str, &serde_json::Value) -> Result<serde_json::Value> + 'static;

#[cfg(test)]
thread_local! {
    static SCHEMA_SDL_FETCH_HOOK: std::cell::RefCell<Option<std::rc::Rc<SchemaSdlFetchHook>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
type SchemaSdlFetchHook =
    dyn Fn(&str, Option<&SlimCliRepoScope>) -> Result<(u16, String)> + 'static;

#[cfg(test)]
struct ThreadLocalHookGuard(fn());

#[cfg(test)]
impl Drop for ThreadLocalHookGuard {
    fn drop(&mut self) {
        (self.0)();
    }
}

pub(crate) async fn execute_devql_graphql<T: DeserializeOwned>(
    scope: &SlimCliRepoScope,
    query: &str,
    variables: serde_json::Value,
) -> Result<T> {
    #[cfg(test)]
    if let Some(data) =
        maybe_execute_devql_graphql_via_hook(scope.repo_root.as_path(), query, &variables)
    {
        return Ok(serde_json::from_value(data?)?);
    }

    crate::daemon::execute_slim_graphql(scope.repo_root.as_path(), scope, query, variables).await
}

pub(crate) async fn execute_runtime_graphql<T: DeserializeOwned>(
    scope: &SlimCliRepoScope,
    query: &str,
    variables: serde_json::Value,
) -> Result<T> {
    #[cfg(test)]
    if let Some(data) =
        maybe_execute_devql_graphql_via_hook(scope.repo_root.as_path(), query, &variables)
    {
        return Ok(serde_json::from_value(data?)?);
    }

    crate::daemon::execute_runtime_graphql(scope.repo_root.as_path(), query, variables).await
}

pub(crate) async fn run_init_via_graphql(scope: &SlimCliRepoScope) -> Result<()> {
    let response: InitSchemaMutationData =
        execute_devql_graphql(scope, INIT_SCHEMA_MUTATION, json!({})).await?;
    println!("{}", format_init_schema_summary(&response.init_schema));
    Ok(())
}

pub(crate) async fn fetch_slim_schema_sdl_via_daemon(scope: &SlimCliRepoScope) -> Result<String> {
    fetch_schema_sdl_via_daemon("/devql/sdl", Some(scope)).await
}

pub(crate) async fn fetch_global_schema_sdl_via_daemon() -> Result<String> {
    fetch_schema_sdl_via_daemon("/devql/global/sdl", None).await
}

pub(crate) async fn start_init_via_runtime_graphql(
    scope: &SlimCliRepoScope,
    input: &RuntimeStartInitInput,
) -> Result<StartInitResultGraphqlRecord> {
    #[cfg(test)]
    let should_require_daemon = !graphql_executor_hook_installed();
    #[cfg(not(test))]
    let should_require_daemon = true;

    if should_require_daemon {
        ensure_daemon_available_for_tasks(
            scope.repo_root.as_path(),
            DaemonStartPolicy::RequireRunning,
        )
        .await?;
    }

    let response: StartInitMutationData = execute_runtime_graphql(
        scope,
        START_INIT_MUTATION,
        json!({
            "repoId": input.repo_id,
            "input": runtime_start_input_json(input),
        }),
    )
    .await?;
    Ok(response.start_init)
}

pub(crate) async fn runtime_snapshot_via_graphql(
    scope: &SlimCliRepoScope,
    repo_id: &str,
) -> Result<RuntimeSnapshotGraphqlRecord> {
    let response: RuntimeSnapshotQueryData = execute_runtime_graphql(
        scope,
        RUNTIME_SNAPSHOT_QUERY,
        json!({
            "repoId": repo_id,
        }),
    )
    .await?;
    Ok(response.runtime_snapshot)
}

pub(crate) async fn enqueue_sync_task_via_graphql(
    scope: &SlimCliRepoScope,
    full: bool,
    paths: Option<Vec<String>>,
    repair: bool,
    validate: bool,
    source: &str,
    require_daemon: bool,
) -> Result<(TaskGraphqlRecord, bool)> {
    ensure_daemon_available_for_tasks(
        scope.repo_root.as_path(),
        daemon_start_policy(require_daemon),
    )
    .await?;

    let response: EnqueueTaskMutationData = execute_devql_graphql(
        scope,
        ENQUEUE_TASK_MUTATION,
        json!({
            "input": {
                "kind": "SYNC",
                "sync": {
                    "full": full,
                    "paths": paths,
                    "repair": repair,
                    "validate": validate,
                    "source": source,
                }
            }
        }),
    )
    .await?;
    Ok((response.enqueue_task.task, response.enqueue_task.merged))
}

pub(crate) async fn enqueue_ingest_task_via_graphql(
    scope: &SlimCliRepoScope,
    backfill: Option<usize>,
    require_daemon: bool,
) -> Result<(TaskGraphqlRecord, bool)> {
    ensure_daemon_available_for_tasks(
        scope.repo_root.as_path(),
        daemon_start_policy(require_daemon),
    )
    .await?;

    let response: EnqueueTaskMutationData = execute_devql_graphql(
        scope,
        ENQUEUE_TASK_MUTATION,
        json!({
            "input": {
                "kind": "INGEST",
                "ingest": {
                    "backfill": backfill,
                }
            }
        }),
    )
    .await?;
    Ok((response.enqueue_task.task, response.enqueue_task.merged))
}

pub(crate) async fn query_task_via_graphql(
    scope: &SlimCliRepoScope,
    task_id: &str,
) -> Result<Option<TaskGraphqlRecord>> {
    let response: TaskQueryData = execute_devql_graphql(
        scope,
        TASK_QUERY,
        json!({
            "id": task_id,
        }),
    )
    .await?;
    Ok(response.task)
}

pub(crate) async fn list_tasks_via_graphql(
    scope: &SlimCliRepoScope,
    kind: Option<&str>,
    status: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<TaskGraphqlRecord>> {
    let response: TasksQueryData = execute_devql_graphql(
        scope,
        TASKS_QUERY,
        json!({
            "kind": kind.map(graphql_enum_name),
            "status": status.map(graphql_enum_name),
            "limit": limit.map(|value| i32::try_from(value).unwrap_or(i32::MAX)),
        }),
    )
    .await?;
    Ok(response.tasks)
}

pub(crate) async fn task_queue_status_via_graphql(
    scope: &SlimCliRepoScope,
) -> Result<TaskQueueGraphqlRecord> {
    let response: TaskQueueQueryData =
        execute_devql_graphql(scope, TASK_QUEUE_QUERY, json!({})).await?;
    Ok(response.task_queue)
}

pub(crate) async fn pause_task_queue_via_graphql(
    scope: &SlimCliRepoScope,
    reason: Option<&str>,
) -> Result<TaskQueueControlGraphqlRecord> {
    let response: PauseTaskQueueMutationData = execute_devql_graphql(
        scope,
        PAUSE_TASK_QUEUE_MUTATION,
        json!({
            "reason": reason,
        }),
    )
    .await?;
    Ok(response.pause_task_queue)
}

pub(crate) async fn resume_task_queue_via_graphql(
    scope: &SlimCliRepoScope,
) -> Result<TaskQueueControlGraphqlRecord> {
    let response: ResumeTaskQueueMutationData = execute_devql_graphql(
        scope,
        RESUME_TASK_QUEUE_MUTATION,
        json!({
            "repoId": null,
        }),
    )
    .await?;
    Ok(response.resume_task_queue)
}

pub(crate) async fn cancel_task_via_graphql(
    scope: &SlimCliRepoScope,
    task_id: &str,
) -> Result<TaskGraphqlRecord> {
    let response: CancelTaskMutationData = execute_devql_graphql(
        scope,
        CANCEL_TASK_MUTATION,
        json!({
            "id": task_id,
        }),
    )
    .await?;
    Ok(response.cancel_task)
}

pub(crate) async fn watch_task_via_graphql(
    scope: &SlimCliRepoScope,
    initial_task: TaskGraphqlRecord,
) -> Result<Option<TaskGraphqlRecord>> {
    let task_id = initial_task.task_id.clone();
    let follow_up_baseline = sync_follow_up_baseline_if_needed(&initial_task)?;
    let mut renderer = TaskProgressRenderer::new();
    renderer.render(&initial_task)?;

    if initial_task.is_terminal() {
        wait_for_sync_follow_up_work_if_needed(&initial_task, follow_up_baseline.as_ref()).await?;
        renderer.finish()?;
        return handle_terminal_task(task_id.as_str(), initial_task).map(Some);
    }

    match watch_task_via_subscription(scope, task_id.as_str(), &mut renderer).await {
        Ok(final_task) => {
            if let Some(final_task) = final_task.as_ref() {
                wait_for_sync_follow_up_work_if_needed(final_task, follow_up_baseline.as_ref())
                    .await?;
            }
            renderer.finish()?;
            return Ok(final_task);
        }
        Err(err) => {
            log::debug!("task subscription unavailable; falling back to polling: {err:#}");
        }
    }

    let mut latest_task = initial_task;
    let mut poll_interval = tokio::time::interval(TASK_PROGRESS_POLL_INTERVAL);
    let mut render_tick = tokio::time::interval(TASK_RENDER_TICK_INTERVAL);
    loop {
        tokio::select! {
            _ = render_tick.tick(), if renderer.is_interactive() => {
                renderer.tick(&latest_task)?;
            }
            _ = poll_interval.tick() => {
                let Some(task) = query_task_via_graphql(scope, task_id.as_str()).await? else {
                    renderer.finish()?;
                    return Ok(None);
                };
                latest_task = task;
                renderer.render(&latest_task)?;
                match latest_task.status.to_ascii_lowercase().as_str() {
                    "completed" => {
                        wait_for_sync_follow_up_work_if_needed(
                            &latest_task,
                            follow_up_baseline.as_ref(),
                        )
                        .await?;
                        renderer.finish()?;
                        return Ok(Some(latest_task));
                    }
                    "failed" | "cancelled" => {
                        renderer.finish()?;
                        return handle_terminal_task(task_id.as_str(), latest_task).map(Some);
                    }
                    _ => {}
                }
            }
        }
    }
}

fn sync_follow_up_baseline_if_needed(
    task: &TaskGraphqlRecord,
) -> Result<Option<SyncFollowUpBaseline>> {
    if !task.is_sync() {
        return Ok(None);
    }
    capture_sync_follow_up_baseline(task.repo_id.as_str()).map(Some)
}

fn capture_sync_follow_up_baseline(repo_id: &str) -> Result<SyncFollowUpBaseline> {
    let current_state = daemon::current_state_consumer_status(Some(repo_id))?;
    let enrichment = daemon::enrichment_status()?;
    Ok(SyncFollowUpBaseline {
        current_state_failed_runs: current_state.state.failed_runs,
        enrichment: EnrichmentFailureCounts::from_status(&enrichment),
    })
}

async fn wait_for_sync_follow_up_work_if_needed(
    task: &TaskGraphqlRecord,
    baseline: Option<&SyncFollowUpBaseline>,
) -> Result<()> {
    if !task.is_sync() || !task.status.eq_ignore_ascii_case("completed") {
        return Ok(());
    }
    let captured_baseline;
    let baseline = match baseline {
        Some(baseline) => baseline,
        None => {
            captured_baseline = capture_sync_follow_up_baseline(task.repo_id.as_str())?;
            &captured_baseline
        }
    };

    let started = std::time::Instant::now();
    let mut poll_interval = tokio::time::interval(SYNC_FOLLOW_UP_POLL_INTERVAL);
    let mut current_state_idle_since: Option<std::time::Instant> = None;
    let mut observed_enrichment_activity = false;
    loop {
        let current_state = daemon::current_state_consumer_status(Some(task.repo_id.as_str()))?;
        let enrichment = daemon::enrichment_status()?;
        let current_state_idle = current_state.current_repo_run.is_none();
        let enrichment_running = enrichment.state.running_embedding_jobs > 0
            || enrichment.state.running_clone_edges_rebuild_jobs > 0;
        let enrichment_pending = enrichment.state.pending_embedding_jobs > 0
            || enrichment.state.pending_clone_edges_rebuild_jobs > 0;
        let enrichment_idle = !enrichment_pending && !enrichment_running;

        if enrichment_running {
            observed_enrichment_activity = true;
        }

        if current_state.state.failed_runs > baseline.current_state_failed_runs {
            bail!(
                "{}",
                current_state_follow_up_failure_message(&task.repo_id, baseline, &current_state)
            );
        }
        let enrichment_failures = EnrichmentFailureCounts::from_status(&enrichment);
        if enrichment_failures.has_new_failures_since(baseline.enrichment) {
            bail!(
                "{}",
                enrichment_follow_up_failure_message(
                    &task.repo_id,
                    &baseline.enrichment,
                    &enrichment,
                )
            );
        }
        if current_state_idle {
            let idle_since = current_state_idle_since.get_or_insert_with(std::time::Instant::now);
            if enrichment_idle {
                return Ok(());
            }
            if !observed_enrichment_activity
                && !enrichment_running
                && idle_since.elapsed() >= SYNC_FOLLOW_UP_PENDING_GRACE
            {
                return Ok(());
            }
        } else {
            current_state_idle_since = None;
        }
        if started.elapsed() >= SYNC_FOLLOW_UP_TIMEOUT {
            bail!(
                "timed out waiting for sync follow-up work to finish for repo `{}`",
                task.repo_id
            );
        }

        poll_interval.tick().await;
    }
}

fn current_state_follow_up_failure_message(
    repo_id: &str,
    baseline: &SyncFollowUpBaseline,
    status: &daemon::CapabilityEventQueueStatus,
) -> String {
    format!(
        "sync task completed but current-state consumer follow-up work failed for repo `{repo_id}` \
(failed runs increased from {} to {}). Inspect with `bitloops daemon status`.",
        baseline.current_state_failed_runs, status.state.failed_runs
    )
}

pub(super) fn enrichment_follow_up_failure_message(
    repo_id: &str,
    baseline: &EnrichmentFailureCounts,
    status: &daemon::EnrichmentQueueStatus,
) -> String {
    let current = EnrichmentFailureCounts::from_status(status);
    let pool_deltas = enrichment_failure_pool_deltas(*baseline, current);
    let stage = if pool_deltas.is_empty() {
        "semantic enrichment".to_string()
    } else {
        format!("semantic enrichment ({})", pool_deltas.join(", "))
    };
    let mut message = format!(
        "sync task completed but {stage} follow-up work failed for repo `{repo_id}` \
(failed jobs increased from {} to {})",
        baseline.failed_jobs, current.failed_jobs
    );
    let embedding_failure_delta = current
        .failed_embedding_jobs
        .saturating_sub(baseline.failed_embedding_jobs);
    if embedding_failure_delta > 0
        && let Some(failed) = status.last_failed_embedding.as_ref()
    {
        message.push_str(&format!(
            "; last failed embedding job `{}` repo={} kind={} artefacts={} attempts={}",
            failed.job_id,
            failed.repo_id,
            failed.representation_kind,
            failed.artefact_count,
            failed.attempts
        ));
        if let Some(error) = failed.error.as_ref() {
            message.push_str(&format!(" error={error}"));
        }
    }
    message.push_str(". Inspect with `bitloops daemon enrichments status`.");
    message
}

pub(super) fn enrichment_failure_pool_deltas(
    baseline: EnrichmentFailureCounts,
    current: EnrichmentFailureCounts,
) -> Vec<String> {
    let mut deltas = Vec::new();
    push_enrichment_failure_delta(
        &mut deltas,
        "Code summaries",
        current
            .failed_semantic_jobs
            .saturating_sub(baseline.failed_semantic_jobs),
    );
    push_enrichment_failure_delta(
        &mut deltas,
        "Semantic search indexing",
        current
            .failed_embedding_jobs
            .saturating_sub(baseline.failed_embedding_jobs),
    );
    push_enrichment_failure_delta(
        &mut deltas,
        "Clone matching",
        current
            .failed_clone_edges_rebuild_jobs
            .saturating_sub(baseline.failed_clone_edges_rebuild_jobs),
    );
    deltas
}

fn push_enrichment_failure_delta(deltas: &mut Vec<String>, label: &str, delta: u64) {
    if delta > 0 {
        deltas.push(format!("{label} +{delta}"));
    }
}

pub(crate) async fn watch_task_id_via_graphql(
    scope: &SlimCliRepoScope,
    task_id: &str,
    require_daemon: bool,
) -> Result<Option<TaskGraphqlRecord>> {
    ensure_daemon_available_for_tasks(
        scope.repo_root.as_path(),
        daemon_start_policy(require_daemon),
    )
    .await?;
    let Some(initial_task) = query_task_via_graphql(scope, task_id).await? else {
        bail!("unknown task `{task_id}`");
    };
    watch_task_via_graphql(scope, initial_task).await
}

fn handle_terminal_task(task_id: &str, task: TaskGraphqlRecord) -> Result<TaskGraphqlRecord> {
    match task.status.to_ascii_lowercase().as_str() {
        "completed" => Ok(task),
        "failed" | "cancelled" => {
            if let Some(error) = task.error.clone() {
                bail!("task {task_id} failed: {error}");
            }
            bail!("task {task_id} ended with status {}", task.status);
        }
        _ => Ok(task),
    }
}

async fn fetch_schema_sdl_via_daemon(
    endpoint_path: &str,
    scope: Option<&SlimCliRepoScope>,
) -> Result<String> {
    #[cfg(test)]
    if let Some(result) = maybe_fetch_schema_sdl_via_hook(endpoint_path, scope) {
        let (status, body) = result?;
        return decode_schema_sdl_response(
            ReqwestStatusCode::from_u16(status)
                .context("decoding test DevQL schema SDL response status")?,
            body,
        );
    }

    let daemon_url = daemon::daemon_url()?
        .context("Bitloops daemon is not running. Start it with `bitloops daemon start`.")?;
    let client = daemon::daemon_http_client(&daemon_url)?;
    let endpoint = format!("{}{}", daemon_url.trim_end_matches('/'), endpoint_path);

    let mut request = client.get(endpoint);
    if let Some(scope) = scope {
        request = crate::devql_transport::attach_repo_daemon_binding_headers(
            request,
            scope.repo_root.as_path(),
        )?;
        request = crate::devql_transport::attach_slim_cli_scope_headers(request, scope);
    }

    let response = request
        .send()
        .await
        .context("sending DevQL schema SDL request to Bitloops daemon")?;
    let status = response.status();
    let body = response
        .text()
        .await
        .context("decoding DevQL schema SDL response from Bitloops daemon")?;

    decode_schema_sdl_response(status, body)
}

fn decode_schema_sdl_response(status: ReqwestStatusCode, body: String) -> Result<String> {
    if status != ReqwestStatusCode::OK {
        if let Some(snippet) = schema_sdl_error_body_snippet(body.as_str()) {
            bail!("Bitloops daemon returned HTTP {status} for DevQL schema SDL: {snippet}");
        }
        bail!("Bitloops daemon returned HTTP {status} for DevQL schema SDL");
    }

    Ok(body)
}

fn schema_sdl_error_body_snippet(body: &str) -> Option<String> {
    const MAX_SNIPPET_CHARS: usize = 160;

    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }

    let mut snippet = collapsed
        .chars()
        .take(MAX_SNIPPET_CHARS)
        .collect::<String>();
    if collapsed.chars().count() > MAX_SNIPPET_CHARS {
        snippet.push_str("...");
    }
    Some(snippet)
}

fn daemon_start_policy(require_daemon: bool) -> DaemonStartPolicy {
    if require_daemon {
        DaemonStartPolicy::RequireRunning
    } else {
        DaemonStartPolicy::AutoStart
    }
}

fn graphql_enum_name(raw: &str) -> String {
    raw.trim().to_ascii_uppercase().replace('-', "_")
}

fn runtime_start_input_json(input: &RuntimeStartInitInput) -> serde_json::Value {
    let embeddings_bootstrap = input
        .embeddings_bootstrap
        .as_ref()
        .map(runtime_embeddings_bootstrap_input_json);
    let summaries_bootstrap = input
        .summaries_bootstrap
        .as_ref()
        .map(runtime_summary_bootstrap_input_json);
    json!({
        "runSync": input.run_sync,
        "runIngest": input.run_ingest,
        "runCodeEmbeddings": input.run_code_embeddings,
        "runSummaries": input.run_summaries,
        "runSummaryEmbeddings": input.run_summary_embeddings,
        "ingestBackfill": input.ingest_backfill,
        "embeddingsBootstrap": embeddings_bootstrap,
        "summariesBootstrap": summaries_bootstrap,
    })
}

fn runtime_embeddings_bootstrap_input_json(
    input: &RuntimeEmbeddingsBootstrapRequestInput,
) -> serde_json::Value {
    json!({
        "configPath": input.config_path,
        "profileName": input.profile_name,
        "mode": graphql_enum_name(input.mode.as_str()),
        "gatewayUrlOverride": input.gateway_url_override,
        "apiKeyEnv": input.api_key_env,
    })
}

fn runtime_summary_bootstrap_input_json(
    input: &RuntimeSummaryBootstrapRequestInput,
) -> serde_json::Value {
    json!({
        "action": graphql_enum_name(input.action.as_str()),
        "message": input.message,
        "modelName": input.model_name,
        "gatewayUrlOverride": input.gateway_url_override,
        "apiKeyEnv": input.api_key_env,
    })
}

async fn ensure_daemon_available_for_tasks(
    repo_root: &Path,
    policy: DaemonStartPolicy,
) -> Result<()> {
    #[cfg(test)]
    if matches!(policy, DaemonStartPolicy::AutoStart)
        && let Some(result) = maybe_bootstrap_daemon_via_hook(repo_root)
    {
        return result;
    }

    if daemon::daemon_url()?.is_some() {
        return Ok(());
    }

    if matches!(policy, DaemonStartPolicy::RequireRunning) {
        bail!("Bitloops daemon is not running. Start it with `bitloops daemon start`.");
    }

    let report = daemon::status().await?;
    let daemon_config_path = crate::config::resolve_bound_daemon_config_path_for_repo(repo_root)
        .or_else(|_| crate::config::resolve_daemon_config_path_for_repo(repo_root))?;
    let daemon_config = daemon::resolve_daemon_config(Some(daemon_config_path.as_path()))?;
    let config = DashboardServerConfig {
        host: None,
        port: crate::api::DEFAULT_DASHBOARD_PORT,
        no_open: true,
        force_http: false,
        recheck_local_dashboard_net: false,
        bundle_dir: None,
    };

    if report.service.is_some() {
        let _ = daemon::start_service(&daemon_config, config, None).await?;
    } else {
        let _ = daemon::start_detached(&daemon_config, config, None).await?;
    }

    Ok(())
}

#[cfg(test)]
pub(crate) fn with_task_daemon_bootstrap_hook<T>(
    hook: impl Fn(&Path) -> Result<()> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    TASK_DAEMON_BOOTSTRAP_HOOK.with(|cell: &TaskDaemonBootstrapHookCell| {
        assert!(
            cell.borrow().is_none(),
            "task daemon hook already installed"
        );
        *cell.borrow_mut() = Some(std::rc::Rc::new(hook));
    });
    let _guard = ThreadLocalHookGuard(clear_task_daemon_bootstrap_hook);
    f()
}

#[cfg(test)]
pub(crate) async fn with_task_daemon_bootstrap_hook_async<T, Fut>(
    hook: impl Fn(&Path) -> Result<()> + 'static,
    f: impl FnOnce() -> Fut,
) -> T
where
    Fut: Future<Output = T>,
{
    TASK_DAEMON_BOOTSTRAP_HOOK.with(|cell: &TaskDaemonBootstrapHookCell| {
        assert!(
            cell.borrow().is_none(),
            "task daemon hook already installed"
        );
        *cell.borrow_mut() = Some(std::rc::Rc::new(hook));
    });
    let _guard = ThreadLocalHookGuard(clear_task_daemon_bootstrap_hook);
    f().await
}

#[cfg(test)]
pub(crate) fn with_ingest_daemon_bootstrap_hook<T>(
    hook: impl Fn(&Path) -> Result<()> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    with_task_daemon_bootstrap_hook(hook, f)
}

#[cfg(test)]
pub(crate) async fn with_ingest_daemon_bootstrap_hook_async<T, Fut>(
    hook: impl Fn(&Path) -> Result<()> + 'static,
    f: impl FnOnce() -> Fut,
) -> T
where
    Fut: Future<Output = T>,
{
    with_task_daemon_bootstrap_hook_async(hook, f).await
}

#[cfg(test)]
fn maybe_bootstrap_daemon_via_hook(repo_root: &Path) -> Option<Result<()>> {
    TASK_DAEMON_BOOTSTRAP_HOOK.with(|hook: &TaskDaemonBootstrapHookCell| {
        hook.borrow().as_ref().map(|hook| hook(repo_root))
    })
}

#[cfg(test)]
fn clear_task_daemon_bootstrap_hook() {
    TASK_DAEMON_BOOTSTRAP_HOOK.with(|cell: &TaskDaemonBootstrapHookCell| {
        *cell.borrow_mut() = None;
    });
}

#[cfg(test)]
pub(crate) fn with_schema_sdl_fetch_hook<T>(
    hook: impl Fn(&str, Option<&SlimCliRepoScope>) -> Result<(u16, String)> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    SCHEMA_SDL_FETCH_HOOK.with(
        |cell: &std::cell::RefCell<Option<std::rc::Rc<SchemaSdlFetchHook>>>| {
            assert!(
                cell.borrow().is_none(),
                "schema SDL fetch hook already installed"
            );
            *cell.borrow_mut() = Some(std::rc::Rc::new(hook));
        },
    );
    let _guard = ThreadLocalHookGuard(clear_schema_sdl_fetch_hook);
    f()
}

#[cfg(test)]
fn maybe_fetch_schema_sdl_via_hook(
    endpoint_path: &str,
    scope: Option<&SlimCliRepoScope>,
) -> Option<Result<(u16, String)>> {
    SCHEMA_SDL_FETCH_HOOK.with(
        |hook: &std::cell::RefCell<Option<std::rc::Rc<SchemaSdlFetchHook>>>| {
            hook.borrow()
                .as_ref()
                .map(|hook| hook(endpoint_path, scope))
        },
    )
}

#[cfg(test)]
fn clear_schema_sdl_fetch_hook() {
    SCHEMA_SDL_FETCH_HOOK.with(
        |cell: &std::cell::RefCell<Option<std::rc::Rc<SchemaSdlFetchHook>>>| {
            *cell.borrow_mut() = None;
        },
    );
}

#[cfg(test)]
pub(crate) fn with_graphql_executor_hook<T>(
    hook: impl Fn(&Path, &str, &serde_json::Value) -> Result<serde_json::Value> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    GRAPHQL_EXECUTOR_HOOK.with(
        |cell: &std::cell::RefCell<Option<std::rc::Rc<GraphqlExecutorHook>>>| {
            assert!(
                cell.borrow().is_none(),
                "graphql executor hook already installed"
            );
            *cell.borrow_mut() = Some(std::rc::Rc::new(hook));
        },
    );
    let _guard = ThreadLocalHookGuard(clear_graphql_executor_hook);
    f()
}

#[cfg(test)]
pub(crate) async fn with_graphql_executor_hook_async<T, Fut>(
    hook: impl Fn(&Path, &str, &serde_json::Value) -> Result<serde_json::Value> + 'static,
    f: impl FnOnce() -> Fut,
) -> T
where
    Fut: Future<Output = T>,
{
    GRAPHQL_EXECUTOR_HOOK.with(
        |cell: &std::cell::RefCell<Option<std::rc::Rc<GraphqlExecutorHook>>>| {
            assert!(
                cell.borrow().is_none(),
                "graphql executor hook already installed"
            );
            *cell.borrow_mut() = Some(std::rc::Rc::new(hook));
        },
    );
    let _guard = ThreadLocalHookGuard(clear_graphql_executor_hook);
    f().await
}

#[cfg(test)]
fn maybe_execute_devql_graphql_via_hook(
    repo_root: &Path,
    query: &str,
    variables: &serde_json::Value,
) -> Option<Result<serde_json::Value>> {
    GRAPHQL_EXECUTOR_HOOK.with(
        |hook: &std::cell::RefCell<Option<std::rc::Rc<GraphqlExecutorHook>>>| {
            hook.borrow()
                .as_ref()
                .map(|hook| hook(repo_root, query, variables))
        },
    )
}

#[cfg(test)]
fn graphql_executor_hook_installed() -> bool {
    GRAPHQL_EXECUTOR_HOOK.with(
        |hook: &std::cell::RefCell<Option<std::rc::Rc<GraphqlExecutorHook>>>| {
            hook.borrow().is_some()
        },
    )
}

#[cfg(test)]
fn clear_graphql_executor_hook() {
    GRAPHQL_EXECUTOR_HOOK.with(
        |cell: &std::cell::RefCell<Option<std::rc::Rc<GraphqlExecutorHook>>>| {
            *cell.borrow_mut() = None;
        },
    );
}
