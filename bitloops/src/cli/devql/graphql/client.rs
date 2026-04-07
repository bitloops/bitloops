use std::path::Path;

use anyhow::{Result, bail};
use serde::de::DeserializeOwned;
use serde_json::json;

use super::documents::{
    ENQUEUE_SYNC_MUTATION, INGEST_MUTATION, INIT_SCHEMA_MUTATION, SYNC_TASK_QUERY,
};
use super::progress::{
    SYNC_PROGRESS_POLL_INTERVAL, SYNC_RENDER_TICK_INTERVAL, SyncProgressRenderer,
};
use super::subscription::watch_sync_task_via_subscription;
use super::types::{
    EnqueueSyncMutationData, IngestMutationData, InitSchemaMutationData, SyncTaskGraphqlRecord,
    SyncTaskQueryData,
};
use crate::devql_transport::SlimCliRepoScope;
use crate::host::devql::{SyncSummary, format_ingestion_summary, format_init_schema_summary};
use crate::{api::DashboardServerConfig, daemon};

#[cfg(test)]
type IngestDaemonBootstrapHook = dyn Fn(&Path) -> Result<()> + 'static;

#[cfg(test)]
type IngestDaemonBootstrapHookCell =
    std::cell::RefCell<Option<std::rc::Rc<IngestDaemonBootstrapHook>>>;

#[cfg(test)]
thread_local! {
    static INGEST_DAEMON_BOOTSTRAP_HOOK: IngestDaemonBootstrapHookCell =
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

pub(crate) async fn run_ingest_via_graphql(
    scope: &SlimCliRepoScope,
    backfill: Option<usize>,
) -> Result<()> {
    ensure_daemon_available_for_ingest(scope.repo_root.as_path()).await?;
    let variables = backfill.map_or_else(
        || json!({}),
        |backfill| {
            json!({
                "input": {
                    "backfill": backfill,
                }
            })
        },
    );
    let response: IngestMutationData =
        execute_devql_graphql(scope, INGEST_MUTATION, variables).await?;
    println!("{}", format_ingestion_summary(&response.ingest));
    Ok(())
}

pub(crate) async fn enqueue_sync_via_graphql(
    scope: &SlimCliRepoScope,
    full: bool,
    paths: Option<Vec<String>>,
    repair: bool,
    validate: bool,
    source: &str,
) -> Result<(SyncTaskGraphqlRecord, bool)> {
    ensure_daemon_available_for_ingest(scope.repo_root.as_path()).await?;
    let response: EnqueueSyncMutationData = execute_devql_graphql(
        scope,
        ENQUEUE_SYNC_MUTATION,
        json!({
            "input": {
                "full": full,
                "paths": paths,
                "repair": repair,
                "validate": validate,
                "source": source,
            }
        }),
    )
    .await?;
    Ok((response.enqueue_sync.task, response.enqueue_sync.merged))
}

pub(crate) async fn query_sync_task_via_graphql(
    scope: &SlimCliRepoScope,
    task_id: &str,
) -> Result<Option<SyncTaskGraphqlRecord>> {
    let response: SyncTaskQueryData = execute_devql_graphql(
        scope,
        SYNC_TASK_QUERY,
        json!({
            "id": task_id,
        }),
    )
    .await?;
    Ok(response.sync_task)
}

pub(crate) async fn watch_sync_task_via_graphql(
    scope: &SlimCliRepoScope,
    initial_task: SyncTaskGraphqlRecord,
) -> Result<Option<SyncSummary>> {
    let task_id = initial_task.task_id.clone();
    let mut renderer = SyncProgressRenderer::new();
    renderer.render(&initial_task)?;

    match watch_sync_task_via_subscription(task_id.as_str(), &mut renderer).await {
        Ok(summary) => {
            renderer.finish()?;
            return Ok(summary);
        }
        Err(err) => {
            log::debug!("sync subscription unavailable; falling back to polling: {err:#}");
        }
    }

    let mut latest_task = initial_task;
    let mut poll_interval = tokio::time::interval(SYNC_PROGRESS_POLL_INTERVAL);
    let mut render_tick = tokio::time::interval(SYNC_RENDER_TICK_INTERVAL);
    loop {
        tokio::select! {
            _ = render_tick.tick(), if renderer.is_interactive() => {
                renderer.tick(&latest_task)?;
            }
            _ = poll_interval.tick() => {
                let Some(task) = query_sync_task_via_graphql(scope, task_id.as_str()).await? else {
                    renderer.finish()?;
                    return Ok(None);
                };
                latest_task = task;
                renderer.render(&latest_task)?;
                match latest_task.status.as_str() {
                    "completed" => {
                        renderer.finish()?;
                        return Ok(latest_task.summary.clone().map(Into::into));
                    }
                    "failed" | "cancelled" => {
                        renderer.finish()?;
                        if let Some(error) = latest_task.error.clone() {
                            bail!("sync task {task_id} failed: {error}");
                        }
                        bail!("sync task {task_id} ended with status {}", latest_task.status);
                    }
                    _ => {}
                }
            }
        }
    }
}

async fn ensure_daemon_available_for_ingest(_repo_root: &Path) -> Result<()> {
    #[cfg(test)]
    if let Some(result) = maybe_bootstrap_daemon_via_hook(_repo_root) {
        return result;
    }

    if daemon::daemon_url()?.is_some() {
        return Ok(());
    }

    let report = daemon::status().await?;
    let daemon_config = daemon::resolve_daemon_config(None)?;
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
pub(crate) fn with_ingest_daemon_bootstrap_hook<T>(
    hook: impl Fn(&Path) -> Result<()> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    INGEST_DAEMON_BOOTSTRAP_HOOK.with(|cell: &IngestDaemonBootstrapHookCell| {
        assert!(
            cell.borrow().is_none(),
            "ingest daemon hook already installed"
        );
        *cell.borrow_mut() = Some(std::rc::Rc::new(hook));
    });
    let result = f();
    INGEST_DAEMON_BOOTSTRAP_HOOK.with(|cell: &IngestDaemonBootstrapHookCell| {
        *cell.borrow_mut() = None;
    });
    result
}

#[cfg(test)]
fn maybe_bootstrap_daemon_via_hook(repo_root: &Path) -> Option<Result<()>> {
    INGEST_DAEMON_BOOTSTRAP_HOOK.with(|hook: &IngestDaemonBootstrapHookCell| {
        hook.borrow().as_ref().map(|hook| hook(repo_root))
    })
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
    let result = f();
    GRAPHQL_EXECUTOR_HOOK.with(
        |cell: &std::cell::RefCell<Option<std::rc::Rc<GraphqlExecutorHook>>>| {
            *cell.borrow_mut() = None;
        },
    );
    result
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
