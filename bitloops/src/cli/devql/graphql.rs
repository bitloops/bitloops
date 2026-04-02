use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::json;
use std::path::Path;
use std::time::Duration;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

use crate::devql_transport::SlimCliRepoScope;
use crate::host::devql::{
    IngestionCounters, InitSchemaSummary, SyncSummary, SyncValidationFileDrift,
    SyncValidationSummary, format_ingestion_summary, format_init_schema_summary,
};
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

const INIT_SCHEMA_MUTATION: &str = r#"
    mutation InitSchema {
      initSchema {
        success
        repoIdentity
        repoId
        relationalBackend
        eventsBackend
      }
    }
"#;

const INGEST_MUTATION: &str = r#"
    mutation Ingest($input: IngestInput!) {
      ingest(input: $input) {
        success
        checkpointsProcessed
        eventsInserted
        artefactsUpserted
        checkpointsWithoutCommit
        temporaryRowsPromoted
        semanticFeatureRowsUpserted
        semanticFeatureRowsSkipped
        symbolEmbeddingRowsUpserted
        symbolEmbeddingRowsSkipped
        symbolCloneEdgesUpserted
        symbolCloneSourcesScored
      }
    }
"#;

const ENQUEUE_SYNC_MUTATION: &str = r#"
    mutation EnqueueSync($input: EnqueueSyncInput!) {
      enqueueSync(input: $input) {
        merged
        task {
          taskId
          repoId
          repoName
          repoIdentity
          source
          mode
          status
          phase
          submittedAtUnix
          startedAtUnix
          updatedAtUnix
          completedAtUnix
          queuePosition
          tasksAhead
          currentPath
          pathsTotal
          pathsCompleted
          pathsRemaining
          pathsUnchanged
          pathsAdded
          pathsChanged
          pathsRemoved
          cacheHits
          cacheMisses
          parseErrors
          error
          summary {
            success
            mode
            parserVersion
            extractorVersion
            activeBranch
            headCommitSha
            headTreeSha
            pathsUnchanged
            pathsAdded
            pathsChanged
            pathsRemoved
            cacheHits
            cacheMisses
            parseErrors
            validation {
              valid
              expectedArtefacts
              actualArtefacts
              expectedEdges
              actualEdges
              missingArtefacts
              staleArtefacts
              mismatchedArtefacts
              missingEdges
              staleEdges
              mismatchedEdges
              filesWithDrift {
                path
                missingArtefacts
                staleArtefacts
                mismatchedArtefacts
                missingEdges
                staleEdges
                mismatchedEdges
              }
            }
          }
        }
      }
    }
"#;

const SYNC_TASK_QUERY: &str = r#"
    query SyncTask($id: String!) {
      syncTask(id: $id) {
        taskId
        repoId
        repoName
        repoIdentity
        source
        mode
        status
        phase
        submittedAtUnix
        startedAtUnix
        updatedAtUnix
        completedAtUnix
        queuePosition
        tasksAhead
        currentPath
        pathsTotal
        pathsCompleted
        pathsRemaining
        pathsUnchanged
        pathsAdded
        pathsChanged
        pathsRemoved
        cacheHits
        cacheMisses
        parseErrors
        error
        summary {
          success
          mode
          parserVersion
          extractorVersion
          activeBranch
          headCommitSha
          headTreeSha
          pathsUnchanged
          pathsAdded
          pathsChanged
          pathsRemoved
          cacheHits
          cacheMisses
          parseErrors
          validation {
            valid
            expectedArtefacts
            actualArtefacts
            expectedEdges
            actualEdges
            missingArtefacts
            staleArtefacts
            mismatchedArtefacts
            missingEdges
            staleEdges
            mismatchedEdges
            filesWithDrift {
              path
              missingArtefacts
              staleArtefacts
              mismatchedArtefacts
              missingEdges
              staleEdges
              mismatchedEdges
            }
          }
        }
      }
    }
"#;

const SYNC_PROGRESS_SUBSCRIPTION: &str = r#"
    subscription SyncProgress($taskId: String!) {
      syncProgress(taskId: $taskId) {
        taskId
        repoId
        repoName
        repoIdentity
        source
        mode
        status
        phase
        submittedAtUnix
        startedAtUnix
        updatedAtUnix
        completedAtUnix
        queuePosition
        tasksAhead
        currentPath
        pathsTotal
        pathsCompleted
        pathsRemaining
        pathsUnchanged
        pathsAdded
        pathsChanged
        pathsRemoved
        cacheHits
        cacheMisses
        parseErrors
        error
        summary {
          success
          mode
          parserVersion
          extractorVersion
          activeBranch
          headCommitSha
          headTreeSha
          pathsUnchanged
          pathsAdded
          pathsChanged
          pathsRemoved
          cacheHits
          cacheMisses
          parseErrors
          validation {
            valid
            expectedArtefacts
            actualArtefacts
            expectedEdges
            actualEdges
            missingArtefacts
            staleArtefacts
            mismatchedArtefacts
            missingEdges
            staleEdges
            mismatchedEdges
            filesWithDrift {
              path
              missingArtefacts
              staleArtefacts
              mismatchedArtefacts
              missingEdges
              staleEdges
              mismatchedEdges
            }
          }
        }
      }
    }
"#;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitSchemaMutationData {
    init_schema: InitSchemaSummary,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct IngestMutationData {
    ingest: IngestionCounters,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnqueueSyncMutationData {
    enqueue_sync: EnqueueSyncMutationResult,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnqueueSyncMutationResult {
    merged: bool,
    task: SyncTaskGraphqlRecord,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncTaskQueryData {
    sync_task: Option<SyncTaskGraphqlRecord>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncProgressSubscriptionData {
    sync_progress: SyncTaskGraphqlRecord,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncMutationResult {
    success: bool,
    mode: String,
    parser_version: String,
    extractor_version: String,
    active_branch: Option<String>,
    head_commit_sha: Option<String>,
    head_tree_sha: Option<String>,
    paths_unchanged: usize,
    paths_added: usize,
    paths_changed: usize,
    paths_removed: usize,
    cache_hits: usize,
    cache_misses: usize,
    parse_errors: usize,
    validation: Option<SyncValidationMutationResult>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncValidationMutationResult {
    valid: bool,
    expected_artefacts: usize,
    actual_artefacts: usize,
    expected_edges: usize,
    actual_edges: usize,
    missing_artefacts: usize,
    stale_artefacts: usize,
    mismatched_artefacts: usize,
    missing_edges: usize,
    stale_edges: usize,
    mismatched_edges: usize,
    files_with_drift: Vec<SyncValidationFileDriftMutationResult>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncValidationFileDriftMutationResult {
    path: String,
    missing_artefacts: usize,
    stale_artefacts: usize,
    mismatched_artefacts: usize,
    missing_edges: usize,
    stale_edges: usize,
    mismatched_edges: usize,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub(crate) struct SyncTaskGraphqlRecord {
    pub task_id: String,
    pub repo_id: String,
    pub repo_name: String,
    pub repo_identity: String,
    pub source: String,
    pub mode: String,
    pub status: String,
    pub phase: String,
    pub submitted_at_unix: i64,
    pub started_at_unix: Option<i64>,
    pub updated_at_unix: i64,
    pub completed_at_unix: Option<i64>,
    pub queue_position: Option<i32>,
    pub tasks_ahead: Option<i32>,
    pub current_path: Option<String>,
    pub paths_total: i32,
    pub paths_completed: i32,
    pub paths_remaining: i32,
    pub paths_unchanged: i32,
    pub paths_added: i32,
    pub paths_changed: i32,
    pub paths_removed: i32,
    pub cache_hits: i32,
    pub cache_misses: i32,
    pub parse_errors: i32,
    pub error: Option<String>,
    pub summary: Option<SyncMutationResult>,
}

impl From<SyncMutationResult> for SyncSummary {
    fn from(value: SyncMutationResult) -> Self {
        Self {
            success: value.success,
            mode: value.mode,
            parser_version: value.parser_version,
            extractor_version: value.extractor_version,
            active_branch: value.active_branch,
            head_commit_sha: value.head_commit_sha,
            head_tree_sha: value.head_tree_sha,
            paths_unchanged: value.paths_unchanged,
            paths_added: value.paths_added,
            paths_changed: value.paths_changed,
            paths_removed: value.paths_removed,
            cache_hits: value.cache_hits,
            cache_misses: value.cache_misses,
            parse_errors: value.parse_errors,
            validation: value.validation.map(|validation| SyncValidationSummary {
                valid: validation.valid,
                expected_artefacts: validation.expected_artefacts,
                actual_artefacts: validation.actual_artefacts,
                expected_edges: validation.expected_edges,
                actual_edges: validation.actual_edges,
                missing_artefacts: validation.missing_artefacts,
                stale_artefacts: validation.stale_artefacts,
                mismatched_artefacts: validation.mismatched_artefacts,
                missing_edges: validation.missing_edges,
                stale_edges: validation.stale_edges,
                mismatched_edges: validation.mismatched_edges,
                files_with_drift: validation
                    .files_with_drift
                    .into_iter()
                    .map(|file| SyncValidationFileDrift {
                        path: file.path,
                        missing_artefacts: file.missing_artefacts,
                        stale_artefacts: file.stale_artefacts,
                        mismatched_artefacts: file.mismatched_artefacts,
                        missing_edges: file.missing_edges,
                        stale_edges: file.stale_edges,
                        mismatched_edges: file.mismatched_edges,
                    })
                    .collect(),
            }),
        }
    }
}

#[cfg(test)]
thread_local! {
    static GRAPHQL_EXECUTOR_HOOK: std::cell::RefCell<Option<std::rc::Rc<GraphqlExecutorHook>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
type GraphqlExecutorHook =
    dyn Fn(&Path, &str, &serde_json::Value) -> Result<serde_json::Value> + 'static;

pub(super) async fn execute_devql_graphql<T: DeserializeOwned>(
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

pub(super) async fn run_init_via_graphql(scope: &SlimCliRepoScope) -> Result<()> {
    let response: InitSchemaMutationData =
        execute_devql_graphql(scope, INIT_SCHEMA_MUTATION, json!({})).await?;
    println!("{}", format_init_schema_summary(&response.init_schema));
    Ok(())
}

pub(super) async fn run_ingest_via_graphql(
    scope: &SlimCliRepoScope,
    max_checkpoints: usize,
) -> Result<()> {
    ensure_daemon_available_for_ingest(scope.repo_root.as_path()).await?;
    let response: IngestMutationData = execute_devql_graphql(
        scope,
        INGEST_MUTATION,
        json!({
            "input": {
                "maxCheckpoints": max_checkpoints,
            }
        }),
    )
    .await?;
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
    task_id: &str,
) -> Result<Option<SyncSummary>> {
    match watch_sync_task_via_subscription(task_id).await {
        Ok(summary) => return Ok(summary),
        Err(err) => eprintln!("sync subscription unavailable; falling back to polling: {err}"),
    }

    let mut last_rendered = None::<String>;
    loop {
        let Some(task) = query_sync_task_via_graphql(scope, task_id).await? else {
            return Ok(None);
        };
        let rendered = format_sync_task_status_line(&task);
        if last_rendered.as_deref() != Some(rendered.as_str()) {
            println!("{rendered}");
            last_rendered = Some(rendered);
        }
        match task.status.as_str() {
            "completed" => return Ok(task.summary.map(Into::into)),
            "failed" | "cancelled" => {
                if let Some(error) = task.error {
                    anyhow::bail!("sync task {task_id} failed: {error}");
                }
                anyhow::bail!("sync task {task_id} ended with status {}", task.status);
            }
            _ => tokio::time::sleep(Duration::from_millis(350)).await,
        }
    }
}

async fn watch_sync_task_via_subscription(task_id: &str) -> Result<Option<SyncSummary>> {
    let endpoint = devql_global_websocket_endpoint()?;
    let mut request = endpoint
        .into_client_request()
        .context("building DevQL websocket subscription request")?;
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_static("graphql-transport-ws"),
    );

    let (mut websocket, _) = connect_async(request)
        .await
        .context("connecting to Bitloops daemon websocket")?;
    websocket
        .send(Message::Text(
            json!({
                "type": "connection_init",
                "payload": {},
            })
            .to_string()
            .into(),
        ))
        .await
        .context("sending GraphQL websocket connection init")?;

    loop {
        let message = websocket
            .next()
            .await
            .transpose()
            .context("waiting for GraphQL websocket connection ack")?
            .context(
                "Bitloops daemon closed the websocket before acknowledging the subscription",
            )?;
        match message {
            Message::Text(payload) => {
                let envelope: serde_json::Value = serde_json::from_str(payload.as_str())
                    .context("decoding GraphQL websocket connection message")?;
                match envelope
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                {
                    "connection_ack" => break,
                    "ping" => {
                        websocket
                            .send(Message::Text(json!({ "type": "pong" }).to_string().into()))
                            .await
                            .context("sending GraphQL websocket pong")?;
                    }
                    "error" | "connection_error" => {
                        bail!(
                            "{}",
                            graphql_websocket_error_message(&envelope).unwrap_or_else(|| {
                                "Bitloops daemon rejected the websocket subscription".to_string()
                            })
                        );
                    }
                    _ => {}
                }
            }
            Message::Ping(payload) => {
                websocket
                    .send(Message::Pong(payload))
                    .await
                    .context("replying to websocket ping")?;
            }
            Message::Close(frame) => {
                let detail = frame
                    .as_ref()
                    .map(|frame| frame.reason.to_string())
                    .filter(|reason| !reason.is_empty())
                    .unwrap_or_else(|| "no close reason".to_string());
                bail!(
                    "Bitloops daemon closed the websocket before acknowledging the subscription: {detail}"
                );
            }
            _ => {}
        }
    }

    websocket
        .send(Message::Text(
            json!({
                "id": "sync-progress",
                "type": "subscribe",
                "payload": {
                    "query": SYNC_PROGRESS_SUBSCRIPTION,
                    "variables": {
                        "taskId": task_id,
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .context("sending sync progress subscription")?;

    let mut last_rendered = None::<String>;
    while let Some(message) = websocket.next().await {
        let message = message.context("reading sync progress subscription message")?;
        match message {
            Message::Text(payload) => {
                let envelope: serde_json::Value = serde_json::from_str(payload.as_str())
                    .context("decoding sync progress subscription message")?;
                match envelope
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                {
                    "next" => {
                        let payload = envelope
                            .get("payload")
                            .cloned()
                            .context("subscription event missing payload")?;
                        if let Some(errors) = payload.get("errors") {
                            bail!("Bitloops daemon returned subscription errors: {errors}");
                        }
                        let data = payload
                            .get("data")
                            .cloned()
                            .context("subscription event missing data")?;
                        let response: SyncProgressSubscriptionData =
                            serde_json::from_value(data)
                                .context("decoding sync progress subscription data")?;
                        let task = response.sync_progress;
                        let rendered = format_sync_task_status_line(&task);
                        if last_rendered.as_deref() != Some(rendered.as_str()) {
                            println!("{rendered}");
                            last_rendered = Some(rendered);
                        }
                        match task.status.as_str() {
                            "completed" => return Ok(task.summary.map(Into::into)),
                            "failed" | "cancelled" => {
                                if let Some(error) = task.error {
                                    bail!("sync task {task_id} failed: {error}");
                                }
                                bail!("sync task {task_id} ended with status {}", task.status);
                            }
                            _ => {}
                        }
                    }
                    "complete" => return Ok(None),
                    "ping" => {
                        websocket
                            .send(Message::Text(json!({ "type": "pong" }).to_string().into()))
                            .await
                            .context("sending GraphQL websocket pong")?;
                    }
                    "error" => {
                        bail!(
                            "{}",
                            graphql_websocket_error_message(&envelope).unwrap_or_else(|| {
                                "Bitloops daemon returned a websocket subscription error"
                                    .to_string()
                            })
                        );
                    }
                    _ => {}
                }
            }
            Message::Ping(payload) => {
                websocket
                    .send(Message::Pong(payload))
                    .await
                    .context("replying to websocket ping")?;
            }
            Message::Close(frame) => {
                let detail = frame
                    .as_ref()
                    .map(|frame| frame.reason.to_string())
                    .filter(|reason| !reason.is_empty())
                    .unwrap_or_else(|| "no close reason".to_string());
                bail!("Bitloops daemon closed the websocket sync subscription: {detail}");
            }
            _ => {}
        }
    }

    Ok(None)
}

fn devql_global_websocket_endpoint() -> Result<String> {
    let runtime_url = daemon::daemon_url()?.context(
        "Bitloops daemon is not running for this repository. Start it with `bitloops daemon start`.",
    )?;
    let base = runtime_url.trim_end_matches('/');
    if let Some(rest) = base.strip_prefix("https://") {
        return Ok(format!("wss://{rest}/devql/global"));
    }
    if let Some(rest) = base.strip_prefix("http://") {
        return Ok(format!("ws://{rest}/devql/global"));
    }
    bail!("unsupported Bitloops daemon url `{runtime_url}`");
}

fn graphql_websocket_error_message(envelope: &serde_json::Value) -> Option<String> {
    if let Some(message) = envelope.get("message").and_then(serde_json::Value::as_str) {
        return Some(message.to_string());
    }
    envelope
        .get("payload")
        .and_then(|payload| payload.get("message").or_else(|| payload.get("errors")))
        .map(|value| value.to_string())
}

pub(super) fn format_sync_task_status_line(task: &SyncTaskGraphqlRecord) -> String {
    match task.status.as_str() {
        "queued" => format!(
            "sync queued: task={} repo={} mode={} position={} ahead={}",
            task.task_id,
            task.repo_name,
            task.mode,
            task.queue_position.unwrap_or(0),
            task.tasks_ahead.unwrap_or(0),
        ),
        "running" => {
            let mut line = format!(
                "sync progress: task={} phase={} {}/{} paths complete",
                task.task_id, task.phase, task.paths_completed, task.paths_total
            );
            if let Some(path) = task.current_path.as_ref() {
                line.push_str(&format!(" path={path}"));
            }
            line
        }
        "completed" => format!("sync completed: task={}", task.task_id),
        "failed" => format!("sync failed: task={}", task.task_id),
        other => format!("sync {}: task={}", other, task.task_id),
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
pub(super) fn with_ingest_daemon_bootstrap_hook<T>(
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
