use std::path::Path;

use anyhow::{Context, Result, bail};
use reqwest::StatusCode as ReqwestStatusCode;
use serde::de::DeserializeOwned;
use serde_json::json;

use super::documents::{
    CANCEL_TASK_MUTATION, ENQUEUE_TASK_MUTATION, INIT_SCHEMA_MUTATION,
    PAUSE_TASK_QUEUE_MUTATION, RESUME_TASK_QUEUE_MUTATION, TASK_QUERY, TASK_QUEUE_QUERY,
    TASKS_QUERY,
};
use super::progress::{TASK_PROGRESS_POLL_INTERVAL, TASK_RENDER_TICK_INTERVAL, TaskProgressRenderer};
use super::subscription::watch_task_via_subscription;
use super::types::{
    CancelTaskMutationData, EnqueueTaskMutationData, InitSchemaMutationData,
    PauseTaskQueueMutationData, ResumeTaskQueueMutationData, TaskGraphqlRecord,
    TaskQueryData, TaskQueueControlGraphqlRecord, TaskQueueGraphqlRecord, TaskQueueQueryData,
    TasksQueryData,
};
use crate::devql_transport::SlimCliRepoScope;
use crate::host::devql::format_init_schema_summary;
use crate::{api::DashboardServerConfig, daemon};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum DaemonStartPolicy {
    AutoStart,
    RequireRunning,
}

#[cfg(test)]
type TaskDaemonBootstrapHook = dyn Fn(&Path) -> Result<()> + 'static;

#[cfg(test)]
type TaskDaemonBootstrapHookCell =
    std::cell::RefCell<Option<std::rc::Rc<TaskDaemonBootstrapHook>>>;

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
    let mut renderer = TaskProgressRenderer::new();
    renderer.render(&initial_task)?;

    if initial_task.is_terminal() {
        renderer.finish()?;
        return handle_terminal_task(task_id.as_str(), initial_task).map(Some);
    }

    match watch_task_via_subscription(scope, task_id.as_str(), &mut renderer).await {
        Ok(final_task) => {
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
    let daemon_config_path = crate::config::resolve_bound_daemon_config_path_for_repo(repo_root)?;
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
        assert!(cell.borrow().is_none(), "task daemon hook already installed");
        *cell.borrow_mut() = Some(std::rc::Rc::new(hook));
    });
    let _guard = ThreadLocalHookGuard(clear_task_daemon_bootstrap_hook);
    f()
}

#[cfg(test)]
pub(crate) fn with_ingest_daemon_bootstrap_hook<T>(
    hook: impl Fn(&Path) -> Result<()> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    with_task_daemon_bootstrap_hook(hook, f)
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
fn clear_graphql_executor_hook() {
    GRAPHQL_EXECUTOR_HOOK.with(
        |cell: &std::cell::RefCell<Option<std::rc::Rc<GraphqlExecutorHook>>>| {
            *cell.borrow_mut() = None;
        },
    );
}
