use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use serde::de::DeserializeOwned;
use serde_json::json;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use terminal_size::{Width, terminal_size};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::{Connector, connect_async, connect_async_tls_with_config};

use crate::devql_transport::SlimCliRepoScope;
use crate::host::devql::{
    IngestionCounters, InitSchemaSummary, SyncSummary, SyncValidationFileDrift,
    SyncValidationSummary, format_ingestion_summary, format_init_schema_summary,
};
use crate::utils::branding::{BITLOOPS_PURPLE_HEX, color_hex_if_enabled};
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

const SYNC_PROGRESS_POLL_INTERVAL: Duration = Duration::from_secs(1);
const SYNC_RENDER_TICK_INTERVAL: Duration = Duration::from_millis(120);
const SYNC_SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

struct SyncProgressRenderer {
    interactive: bool,
    terminal_width: Option<usize>,
    spinner_index: usize,
    last_frame: Option<String>,
    wrote_in_place: bool,
}

impl SyncProgressRenderer {
    fn new() -> Self {
        Self {
            interactive: std::io::stdout().is_terminal() && std::env::var("ACCESSIBLE").is_err(),
            terminal_width: terminal_size().map(|(Width(width), _)| width as usize),
            spinner_index: 0,
            last_frame: None,
            wrote_in_place: false,
        }
    }

    fn is_interactive(&self) -> bool {
        self.interactive
    }

    fn render(&mut self, task: &SyncTaskGraphqlRecord) -> Result<()> {
        let frame = self.render_frame(task);
        self.write_frame(frame, false)
    }

    fn tick(&mut self, task: &SyncTaskGraphqlRecord) -> Result<()> {
        if !self.interactive || !matches!(task.status.as_str(), "queued" | "running") {
            return Ok(());
        }
        self.spinner_index = (self.spinner_index + 1) % SYNC_SPINNER_FRAMES.len();
        let frame = self.render_frame(task);
        self.write_frame(frame, true)
    }

    fn finish(&mut self) -> Result<()> {
        if self.interactive && self.wrote_in_place {
            let mut stdout = io::stdout();
            writeln!(stdout)?;
            stdout.flush()?;
            self.wrote_in_place = false;
        }
        Ok(())
    }

    fn spinner_frame(&self) -> String {
        color_hex_if_enabled(SYNC_SPINNER_FRAMES[self.spinner_index], BITLOOPS_PURPLE_HEX)
    }

    fn render_frame(&self, task: &SyncTaskGraphqlRecord) -> String {
        if self.interactive {
            let bar =
                format_live_sync_progress_bar_line(task, self.spinner_index, self.terminal_width);
            let status =
                format_live_sync_task_status_line(task, &self.spinner_frame(), self.terminal_width);
            format!("{bar}\n{status}")
        } else {
            format_live_sync_task_status_line(task, &self.spinner_frame(), self.terminal_width)
        }
    }

    fn write_frame(&mut self, frame: String, force: bool) -> Result<()> {
        if self.interactive {
            if !force && self.last_frame.as_deref() == Some(frame.as_str()) {
                return Ok(());
            }
            let mut stdout = io::stdout();
            if self.wrote_in_place {
                write!(stdout, "\r\x1b[2K\x1b[1A\r\x1b[2K{frame}")?;
            } else {
                write!(stdout, "{frame}")?;
            }
            stdout.flush()?;
            self.last_frame = Some(frame);
            self.wrote_in_place = true;
            return Ok(());
        }

        if self.last_frame.as_deref() != Some(frame.as_str()) {
            println!("{frame}");
            self.last_frame = Some(frame);
        }
        Ok(())
    }
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
                            anyhow::bail!("sync task {task_id} failed: {error}");
                        }
                        anyhow::bail!("sync task {task_id} ended with status {}", latest_task.status);
                    }
                    _ => {}
                }
            }
        }
    }
}

async fn watch_sync_task_via_subscription(
    task_id: &str,
    renderer: &mut SyncProgressRenderer,
) -> Result<Option<SyncSummary>> {
    let endpoint = devql_global_websocket_endpoint()?;
    let mut request = endpoint
        .as_str()
        .into_client_request()
        .context("building DevQL websocket subscription request")?;
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_static("graphql-transport-ws"),
    );

    let (mut websocket, _) = connect_devql_websocket(request, &endpoint)
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

    let mut latest_task = None::<SyncTaskGraphqlRecord>;
    let mut render_tick = tokio::time::interval(SYNC_RENDER_TICK_INTERVAL);
    loop {
        tokio::select! {
            _ = render_tick.tick(), if renderer.is_interactive() => {
                if let Some(task) = latest_task.as_ref() {
                    renderer.tick(task)?;
                }
            }
            message = websocket.next() => {
                let Some(message) = message else {
                    return Ok(None);
                };
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
                                latest_task = Some(task.clone());
                                renderer.render(&task)?;
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
        }
    }
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

async fn connect_devql_websocket(
    request: tokio_tungstenite::tungstenite::http::Request<()>,
    endpoint: &str,
) -> Result<(
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    tokio_tungstenite::tungstenite::handshake::client::Response,
)> {
    if should_accept_invalid_daemon_websocket_certs(endpoint) {
        let connector = Connector::Rustls(insecure_loopback_websocket_tls_config()?);
        return connect_async_tls_with_config(request, None, false, Some(connector))
            .await
            .map_err(Into::into);
    }

    connect_async(request).await.map_err(Into::into)
}

fn should_accept_invalid_daemon_websocket_certs(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "wss" {
        return false;
    }

    matches!(
        parsed.host_str(),
        Some("localhost") | Some("127.0.0.1") | Some("::1") | Some("[::1]")
    )
}

fn insecure_loopback_websocket_tls_config() -> Result<Arc<rustls::ClientConfig>> {
    static CONFIG: OnceLock<Result<Arc<rustls::ClientConfig>, String>> = OnceLock::new();
    let config = CONFIG.get_or_init(|| {
        ensure_rustls_crypto_provider()
            .map_err(|err| err.to_string())
            .map(|_| {
                Arc::new(
                    rustls::ClientConfig::builder_with_provider(Arc::new(
                        rustls::crypto::aws_lc_rs::default_provider(),
                    ))
                    .with_safe_default_protocol_versions()
                    .expect("safe default TLS versions are valid")
                    .dangerous()
                    .with_custom_certificate_verifier(SkipLoopbackServerVerification::new())
                    .with_no_client_auth(),
                )
            })
    });

    config
        .as_ref()
        .map(Arc::clone)
        .map_err(|message| anyhow::anyhow!(message.clone()))
}

fn ensure_rustls_crypto_provider() -> Result<()> {
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();
    let init = INIT.get_or_init(|| {
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            return rustls::crypto::aws_lc_rs::default_provider()
                .install_default()
                .map_err(|err| format!("install rustls aws_lc_rs crypto provider: {err:?}"));
        }
        Ok(())
    });
    init.as_ref()
        .map(|_| ())
        .map_err(|message| anyhow::anyhow!(message.clone()))
}

#[derive(Debug)]
struct SkipLoopbackServerVerification(Arc<rustls::crypto::CryptoProvider>);

impl SkipLoopbackServerVerification {
    fn new() -> Arc<Self> {
        Arc::new(Self(
            Arc::new(rustls::crypto::aws_lc_rs::default_provider()),
        ))
    }
}

impl ServerCertVerifier for SkipLoopbackServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

pub(super) fn format_live_sync_task_status_line(
    task: &SyncTaskGraphqlRecord,
    spinner: &str,
    terminal_width: Option<usize>,
) -> String {
    let status = match task.status.as_str() {
        "queued" => format!(
            "Sync queued for {} · mode={} · {} ahead",
            task.repo_name,
            task.mode,
            task.tasks_ahead.unwrap_or(0),
        ),
        "running" => {
            let mut line = format!(
                "Syncing {} · {}",
                task.repo_name,
                humanise_sync_phase(task.phase.as_str()),
            );
            if task.paths_total > 0 {
                line.push_str(&format!(" · {}/{}", task.paths_completed, task.paths_total));
            }
            if let Some(path) = task.current_path.as_ref() {
                line.push_str(&format!(" · {path}"));
            }
            line
        }
        "completed" => format!("✓ Sync complete for {}", task.repo_name),
        "failed" => format!("✖ Sync failed for {}", task.repo_name),
        "cancelled" => format!("✖ Sync cancelled for {}", task.repo_name),
        other => format!("Sync {other} for {}", task.repo_name),
    };
    let fitted = fit_live_status_text(
        status.as_str(),
        terminal_width.map(|width| width.saturating_sub(2)),
    );
    format!("{spinner} {fitted}")
}

pub(super) fn format_live_sync_progress_bar_line(
    task: &SyncTaskGraphqlRecord,
    spinner_index: usize,
    terminal_width: Option<usize>,
) -> String {
    let available_width = terminal_width.unwrap_or(80).max(16);
    let ratio = progress_ratio(task);
    let summary = if let Some(ratio) = ratio {
        format!(
            " {:>3}% {}/{}",
            (ratio * 100.0).round() as usize,
            task.paths_completed,
            task.paths_total
        )
    } else {
        format!(" {} ", humanise_sync_phase(task.phase.as_str()))
    };
    let reserved = summary.chars().count() + 2;
    if available_width <= reserved + 1 {
        return fit_live_status_text(summary.trim(), Some(available_width));
    }

    let bar_width = available_width - reserved;
    let bar = if let Some(ratio) = ratio {
        render_determinate_progress_bar(bar_width, ratio)
    } else {
        render_indeterminate_progress_bar(bar_width, spinner_index)
    };

    format!("[{bar}]{summary}")
}

fn progress_ratio(task: &SyncTaskGraphqlRecord) -> Option<f64> {
    match task.status.as_str() {
        "completed" => Some(1.0),
        "failed" | "cancelled" => {
            if task.paths_total > 0 {
                Some((task.paths_completed as f64 / task.paths_total as f64).clamp(0.0, 1.0))
            } else {
                Some(0.0)
            }
        }
        _ if task.paths_total > 0 => {
            Some((task.paths_completed as f64 / task.paths_total as f64).clamp(0.0, 1.0))
        }
        _ => None,
    }
}

fn render_determinate_progress_bar(width: usize, ratio: f64) -> String {
    let filled = ((width as f64) * ratio).round() as usize;
    let filled = filled.min(width);
    let fill = color_hex_if_enabled(&"█".repeat(filled), BITLOOPS_PURPLE_HEX);
    let empty = "░".repeat(width.saturating_sub(filled));
    format!("{fill}{empty}")
}

fn render_indeterminate_progress_bar(width: usize, spinner_index: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let position = spinner_index % width;
    let prefix = "░".repeat(position);
    let pulse = color_hex_if_enabled("█", BITLOOPS_PURPLE_HEX);
    let suffix = "░".repeat(width.saturating_sub(position + 1));
    format!("{prefix}{pulse}{suffix}")
}

fn fit_live_status_text(text: &str, available_width: Option<usize>) -> String {
    let Some(max_width) = available_width else {
        return text.to_string();
    };
    if max_width == 0 {
        return String::new();
    }
    if text.chars().count() <= max_width {
        return text.to_string();
    }
    elide_middle(text, max_width)
}

fn elide_middle(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".to_string();
    }
    if text.chars().count() <= max_width {
        return text.to_string();
    }

    let prefix_len = (max_width - 1) / 2;
    let suffix_len = max_width - 1 - prefix_len;
    let prefix = text.chars().take(prefix_len).collect::<String>();
    let suffix = text
        .chars()
        .rev()
        .take(suffix_len)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{prefix}…{suffix}")
}

fn humanise_sync_phase(phase: &str) -> &'static str {
    match phase {
        "queued" => "waiting in queue",
        "ensuring_schema" => "preparing schema",
        "inspecting_workspace" => "inspecting workspace",
        "building_manifest" => "building manifest",
        "loading_stored_state" => "loading stored state",
        "classifying_paths" => "classifying paths",
        "removing_paths" => "removing stale paths",
        "extracting_paths" => "extracting artefacts",
        "materialising_paths" => "materialising artefacts",
        "running_gc" => "cleaning caches",
        "complete" => "complete",
        "failed" => "failed",
        _ => "working",
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

#[cfg(test)]
mod tests {
    use super::{
        SyncTaskGraphqlRecord, format_live_sync_progress_bar_line,
        format_live_sync_task_status_line, should_accept_invalid_daemon_websocket_certs,
    };
    use crate::test_support::process_state::with_env_vars;

    #[test]
    fn websocket_client_only_relaxes_loopback_wss_urls() {
        assert!(should_accept_invalid_daemon_websocket_certs(
            "wss://localhost:5667/devql/global"
        ));
        assert!(should_accept_invalid_daemon_websocket_certs(
            "wss://127.0.0.1:5667/devql/global"
        ));
        assert!(should_accept_invalid_daemon_websocket_certs(
            "wss://[::1]:5667/devql/global"
        ));
        assert!(!should_accept_invalid_daemon_websocket_certs(
            "ws://127.0.0.1:5667/devql/global"
        ));
        assert!(!should_accept_invalid_daemon_websocket_certs(
            "wss://dev.internal:5667/devql/global"
        ));
        assert!(!should_accept_invalid_daemon_websocket_certs("not-a-url"));
    }

    #[test]
    fn live_sync_status_line_is_compact_and_single_line() {
        let task = SyncTaskGraphqlRecord {
            task_id: "sync-task-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_name: "bitloops".to_string(),
            repo_identity: "local/bitloops".to_string(),
            source: "init".to_string(),
            mode: "auto".to_string(),
            status: "running".to_string(),
            phase: "extracting_paths".to_string(),
            submitted_at_unix: 1,
            started_at_unix: Some(2),
            updated_at_unix: 3,
            completed_at_unix: None,
            queue_position: Some(1),
            tasks_ahead: Some(0),
            current_path: Some("src/lib.rs".to_string()),
            paths_total: 12,
            paths_completed: 4,
            paths_remaining: 8,
            paths_unchanged: 1,
            paths_added: 1,
            paths_changed: 2,
            paths_removed: 0,
            cache_hits: 1,
            cache_misses: 2,
            parse_errors: 0,
            error: None,
            summary: None,
        };

        let rendered = format_live_sync_task_status_line(&task, "*", None);
        assert_eq!(
            rendered,
            "* Syncing bitloops · extracting artefacts · 4/12 · src/lib.rs"
        );
        assert!(!rendered.contains('\n'));
    }

    #[test]
    fn live_sync_status_line_elides_to_terminal_width() {
        let task = SyncTaskGraphqlRecord {
            task_id: "sync-task-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_name: "bitloops".to_string(),
            repo_identity: "local/bitloops".to_string(),
            source: "init".to_string(),
            mode: "auto".to_string(),
            status: "running".to_string(),
            phase: "extracting_paths".to_string(),
            submitted_at_unix: 1,
            started_at_unix: Some(2),
            updated_at_unix: 3,
            completed_at_unix: None,
            queue_position: Some(1),
            tasks_ahead: Some(0),
            current_path: Some("bitloops/src/host/devql/commands_sync/orchestrator.rs".to_string()),
            paths_total: 764,
            paths_completed: 472,
            paths_remaining: 292,
            paths_unchanged: 0,
            paths_added: 0,
            paths_changed: 0,
            paths_removed: 0,
            cache_hits: 0,
            cache_misses: 0,
            parse_errors: 0,
            error: None,
            summary: None,
        };

        let rendered = format_live_sync_task_status_line(&task, "*", Some(48));
        assert!(rendered.chars().count() <= 48);
        assert!(rendered.contains('…'));
        assert!(!rendered.contains('\n'));
    }

    #[test]
    fn live_sync_progress_bar_line_fits_requested_width() {
        with_env_vars(&[("NO_COLOR", Some("1"))], || {
            let task = SyncTaskGraphqlRecord {
                task_id: "sync-task-1".to_string(),
                repo_id: "repo-1".to_string(),
                repo_name: "bitloops".to_string(),
                repo_identity: "local/bitloops".to_string(),
                source: "init".to_string(),
                mode: "auto".to_string(),
                status: "running".to_string(),
                phase: "materialising_paths".to_string(),
                submitted_at_unix: 1,
                started_at_unix: Some(2),
                updated_at_unix: 3,
                completed_at_unix: None,
                queue_position: Some(1),
                tasks_ahead: Some(0),
                current_path: None,
                paths_total: 764,
                paths_completed: 472,
                paths_remaining: 292,
                paths_unchanged: 0,
                paths_added: 0,
                paths_changed: 0,
                paths_removed: 0,
                cache_hits: 0,
                cache_misses: 0,
                parse_errors: 0,
                error: None,
                summary: None,
            };

            let rendered = format_live_sync_progress_bar_line(&task, 0, Some(48));
            assert!(rendered.chars().count() <= 48);
            assert!(rendered.contains("472/764"));
            assert!(!rendered.contains('\n'));
        });
    }
}
