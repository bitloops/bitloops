use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Instant;

use async_graphql::futures_util::{Stream, stream};
use async_graphql::{
    Context, Enum, ID, InputObject, Object, Result, Schema, SimpleObject, Subscription,
};
use async_graphql_axum::{GraphQLProtocol, GraphQLRequest, GraphQLWebSocket};
use axum::{
    extract::{State, WebSocketUpgrade},
    http::HeaderMap,
    response::{IntoResponse, Response as AxumResponse},
};

use super::handlers::resolve_repo_root_from_repo_id;
use super::{ApiError, DashboardState};
use crate::daemon::{
    CapabilityEventQueueStatus, CapabilityEventRunRecord, EmbeddingsBootstrapGateStatus,
    EmbeddingsBootstrapMode, InitEmbeddingsBootstrapRequest, InitRuntimeLaneProgressView,
    InitRuntimeLaneQueueView, InitRuntimeLaneView, InitRuntimeLaneWarningView,
    InitRuntimeSessionView, InitRuntimeSnapshot, InitRuntimeWorkplaneMailboxSnapshot,
    InitRuntimeWorkplanePoolSnapshot, InitRuntimeWorkplaneSnapshot, RuntimeEventRecord,
    StartInitSessionSelections, SummaryBootstrapAction, SummaryBootstrapRequest,
    SummaryBootstrapResultRecord, SummaryBootstrapRunRecord,
};
use crate::devql_transport::parse_repo_root_header;
use crate::graphql::{
    GraphqlActionTelemetry, MAX_DEVQL_QUERY_DEPTH, TaskQueueStatusObject, bad_user_input_error,
    execute_graphql_request, graphql_error, graphql_playground_response, graphql_request_signature,
    track_graphql_action, validate_repo_daemon_binding,
};

pub(crate) type RuntimeGraphqlSchema =
    Schema<RuntimeQueryRoot, RuntimeMutationRoot, RuntimeSubscriptionRoot>;

// The runtime surface is internal-only and powers init/status/dashboard operational views.
// Its snapshot query is intentionally richer than the public DevQL surfaces.
const MAX_RUNTIME_QUERY_COMPLEXITY: usize = 4096;

#[derive(Debug, Clone, Default)]
pub(crate) struct RuntimeRequestContext {
    bound_repo_root: Option<PathBuf>,
}

#[derive(Default)]
pub(crate) struct RuntimeQueryRoot;

#[Object]
impl RuntimeQueryRoot {
    #[graphql(name = "runtimeSnapshot")]
    async fn runtime_snapshot(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: String,
    ) -> Result<RuntimeSnapshotObject> {
        let state = ctx.data_unchecked::<DashboardState>();
        let request_context = ctx
            .data_opt::<RuntimeRequestContext>()
            .cloned()
            .unwrap_or_default();
        let cfg = resolve_runtime_devql_config(state, &request_context, repo_id.as_str())
            .await
            .map_err(map_runtime_api_error)?;
        crate::daemon::shared_init_runtime_coordinator()
            .snapshot_for_repo(&cfg)
            .map(Into::into)
            .map_err(|err| {
                graphql_error(
                    "internal",
                    format!("failed to load runtime snapshot: {err:#}"),
                )
            })
    }
}

#[derive(Default)]
pub(crate) struct RuntimeMutationRoot;

#[Object]
impl RuntimeMutationRoot {
    #[graphql(name = "startInit")]
    async fn start_init(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: String,
        input: StartInitInput,
    ) -> Result<StartInitResult> {
        let state = ctx.data_unchecked::<DashboardState>();
        let request_context = ctx
            .data_opt::<RuntimeRequestContext>()
            .cloned()
            .unwrap_or_default();
        let cfg = resolve_runtime_devql_config(state, &request_context, repo_id.as_str())
            .await
            .map_err(map_runtime_api_error)?;
        let selections = input.into_selections().map_err(bad_user_input_error)?;
        crate::daemon::shared_init_runtime_coordinator()
            .start_session(&cfg, selections)
            .map(|handle| StartInitResult {
                init_session_id: ID::from(handle.init_session_id),
            })
            .map_err(|err| {
                graphql_error("internal", format!("failed to start init session: {err:#}"))
            })
    }
}

#[derive(Default)]
pub(crate) struct RuntimeSubscriptionRoot;

type RuntimeEventStream = Pin<Box<dyn Stream<Item = RuntimeEventObject> + Send>>;

#[Subscription]
impl RuntimeSubscriptionRoot {
    #[graphql(name = "runtimeEvents")]
    async fn runtime_events(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "repoId")] repo_id: String,
        #[graphql(name = "initSessionId")] init_session_id: Option<ID>,
    ) -> RuntimeEventStream {
        let receiver = ctx
            .data_unchecked::<DashboardState>()
            .subscription_hub()
            .subscribe_runtime_events();
        let init_session_id = init_session_id.map(|value| value.to_string());

        Box::pin(stream::unfold(
            (receiver, repo_id, init_session_id),
            |(mut receiver, repo_id, init_session_id)| async move {
                loop {
                    match receiver.recv().await {
                        Ok(event) => {
                            if event.repo_id != repo_id {
                                continue;
                            }
                            if init_session_id.as_ref().is_some_and(|session_id| {
                                event.init_session_id.as_deref() != Some(session_id.as_str())
                            }) {
                                continue;
                            }
                            return Some((event.into(), (receiver, repo_id, init_session_id)));
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            return Some((
                                RuntimeEventObject {
                                    domain: "lagged".to_string(),
                                    repo_id: repo_id.clone(),
                                    init_session_id: init_session_id.clone().map(ID::from),
                                    updated_at_unix: to_graphql_i64(current_unix_timestamp()),
                                    task_id: None,
                                    run_id: None,
                                    mailbox_name: None,
                                },
                                (receiver, repo_id, init_session_id),
                            ));
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                    }
                }
            },
        ))
    }
}

#[derive(Debug, Clone, InputObject)]
pub(crate) struct StartInitInput {
    #[graphql(name = "runSync")]
    pub run_sync: bool,
    #[graphql(name = "runIngest")]
    pub run_ingest: bool,
    #[graphql(name = "ingestBackfill")]
    pub ingest_backfill: Option<i32>,
    #[graphql(name = "embeddingsBootstrap")]
    pub embeddings_bootstrap: Option<InitEmbeddingsBootstrapRequestInput>,
    #[graphql(name = "summariesBootstrap")]
    pub summaries_bootstrap: Option<SummaryBootstrapRequestInput>,
}

impl StartInitInput {
    fn into_selections(self) -> std::result::Result<StartInitSessionSelections, String> {
        if !self.run_ingest && self.ingest_backfill.is_some() {
            return Err("`ingestBackfill` requires `runIngest=true`".to_string());
        }
        Ok(StartInitSessionSelections {
            run_sync: self.run_sync,
            run_ingest: self.run_ingest,
            ingest_backfill: self
                .ingest_backfill
                .map(|value| usize::try_from(value.max(0)).unwrap_or(usize::MAX)),
            embeddings_bootstrap: self.embeddings_bootstrap.map(Into::into),
            summaries_bootstrap: self.summaries_bootstrap.map(Into::into),
        })
    }
}

#[derive(Debug, Clone, InputObject)]
pub(crate) struct InitEmbeddingsBootstrapRequestInput {
    #[graphql(name = "configPath")]
    pub config_path: String,
    #[graphql(name = "profileName")]
    pub profile_name: String,
    pub mode: Option<EmbeddingsBootstrapModeInput>,
    #[graphql(name = "gatewayUrlOverride")]
    pub gateway_url_override: Option<String>,
    #[graphql(name = "apiKeyEnv")]
    pub api_key_env: Option<String>,
}

impl From<InitEmbeddingsBootstrapRequestInput> for InitEmbeddingsBootstrapRequest {
    fn from(value: InitEmbeddingsBootstrapRequestInput) -> Self {
        Self {
            config_path: value.config_path.into(),
            profile_name: value.profile_name,
            mode: value.mode.map(Into::into).unwrap_or_default(),
            gateway_url_override: value.gateway_url_override,
            api_key_env: value.api_key_env,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub(crate) enum EmbeddingsBootstrapModeInput {
    Local,
    Platform,
}

impl From<EmbeddingsBootstrapModeInput> for EmbeddingsBootstrapMode {
    fn from(value: EmbeddingsBootstrapModeInput) -> Self {
        match value {
            EmbeddingsBootstrapModeInput::Local => Self::Local,
            EmbeddingsBootstrapModeInput::Platform => Self::Platform,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub(crate) enum SummaryBootstrapActionInput {
    InstallRuntimeOnly,
    InstallRuntimeOnlyPendingProbe,
    ConfigureLocal,
    ConfigureCloud,
}

impl From<SummaryBootstrapActionInput> for SummaryBootstrapAction {
    fn from(value: SummaryBootstrapActionInput) -> Self {
        match value {
            SummaryBootstrapActionInput::InstallRuntimeOnly => Self::InstallRuntimeOnly,
            SummaryBootstrapActionInput::InstallRuntimeOnlyPendingProbe => {
                Self::InstallRuntimeOnlyPendingProbe
            }
            SummaryBootstrapActionInput::ConfigureLocal => Self::ConfigureLocal,
            SummaryBootstrapActionInput::ConfigureCloud => Self::ConfigureCloud,
        }
    }
}

#[derive(Debug, Clone, InputObject)]
pub(crate) struct SummaryBootstrapRequestInput {
    pub action: SummaryBootstrapActionInput,
    pub message: Option<String>,
    #[graphql(name = "modelName")]
    pub model_name: Option<String>,
    #[graphql(name = "gatewayUrlOverride")]
    pub gateway_url_override: Option<String>,
}

impl From<SummaryBootstrapRequestInput> for SummaryBootstrapRequest {
    fn from(value: SummaryBootstrapRequestInput) -> Self {
        Self {
            action: value.action.into(),
            message: value.message,
            model_name: value.model_name,
            gateway_url_override: value.gateway_url_override,
        }
    }
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct StartInitResult {
    #[graphql(name = "initSessionId")]
    pub init_session_id: ID,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeSnapshotObject {
    #[graphql(name = "repoId")]
    pub repo_id: String,
    #[graphql(name = "taskQueue")]
    pub task_queue: TaskQueueStatusObject,
    #[graphql(name = "currentStateConsumer")]
    pub current_state_consumer: RuntimeCurrentStateConsumerObject,
    pub workplane: RuntimeWorkplaneObject,
    #[graphql(name = "blockedMailboxes")]
    pub blocked_mailboxes: Vec<RuntimeBlockedMailboxObject>,
    #[graphql(name = "embeddingsReadinessGate")]
    pub embeddings_readiness_gate: Option<RuntimeEmbeddingsReadinessGateObject>,
    #[graphql(name = "summariesBootstrap")]
    pub summaries_bootstrap: Option<RuntimeSummaryBootstrapRunObject>,
    #[graphql(name = "currentInitSession")]
    pub current_init_session: Option<RuntimeInitSessionObject>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeCurrentStateConsumerObject {
    pub persisted: bool,
    #[graphql(name = "pendingRuns")]
    pub pending_runs: i32,
    #[graphql(name = "runningRuns")]
    pub running_runs: i32,
    #[graphql(name = "failedRuns")]
    pub failed_runs: i32,
    #[graphql(name = "completedRecentRuns")]
    pub completed_recent_runs: i32,
    #[graphql(name = "lastAction")]
    pub last_action: Option<String>,
    #[graphql(name = "lastUpdatedUnix")]
    pub last_updated_unix: i64,
    #[graphql(name = "currentRepoRun")]
    pub current_repo_run: Option<RuntimeCurrentStateRunObject>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeCurrentStateRunObject {
    #[graphql(name = "runId")]
    pub run_id: String,
    #[graphql(name = "repoId")]
    pub repo_id: String,
    #[graphql(name = "capabilityId")]
    pub capability_id: String,
    #[graphql(name = "initSessionId")]
    pub init_session_id: Option<ID>,
    #[graphql(name = "consumerId")]
    pub consumer_id: String,
    #[graphql(name = "handlerId")]
    pub handler_id: String,
    #[graphql(name = "fromGenerationSeq")]
    pub from_generation_seq: i64,
    #[graphql(name = "toGenerationSeq")]
    pub to_generation_seq: i64,
    #[graphql(name = "reconcileMode")]
    pub reconcile_mode: String,
    pub status: String,
    pub attempts: i32,
    #[graphql(name = "submittedAtUnix")]
    pub submitted_at_unix: i64,
    #[graphql(name = "startedAtUnix")]
    pub started_at_unix: Option<i64>,
    #[graphql(name = "updatedAtUnix")]
    pub updated_at_unix: i64,
    #[graphql(name = "completedAtUnix")]
    pub completed_at_unix: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeWorkplaneObject {
    #[graphql(name = "pendingJobs")]
    pub pending_jobs: i32,
    #[graphql(name = "runningJobs")]
    pub running_jobs: i32,
    #[graphql(name = "failedJobs")]
    pub failed_jobs: i32,
    #[graphql(name = "completedRecentJobs")]
    pub completed_recent_jobs: i32,
    pub pools: Vec<RuntimeWorkplanePoolObject>,
    pub mailboxes: Vec<RuntimeWorkplaneMailboxObject>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeWorkplanePoolObject {
    #[graphql(name = "poolName")]
    pub pool_name: String,
    #[graphql(name = "displayName")]
    pub display_name: String,
    #[graphql(name = "workerBudget")]
    pub worker_budget: i32,
    #[graphql(name = "activeWorkers")]
    pub active_workers: i32,
    #[graphql(name = "pendingJobs")]
    pub pending_jobs: i32,
    #[graphql(name = "runningJobs")]
    pub running_jobs: i32,
    #[graphql(name = "failedJobs")]
    pub failed_jobs: i32,
    #[graphql(name = "completedRecentJobs")]
    pub completed_recent_jobs: i32,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeWorkplaneMailboxObject {
    #[graphql(name = "mailboxName")]
    pub mailbox_name: String,
    #[graphql(name = "displayName")]
    pub display_name: String,
    #[graphql(name = "pendingJobs")]
    pub pending_jobs: i32,
    #[graphql(name = "runningJobs")]
    pub running_jobs: i32,
    #[graphql(name = "failedJobs")]
    pub failed_jobs: i32,
    #[graphql(name = "completedRecentJobs")]
    pub completed_recent_jobs: i32,
    #[graphql(name = "pendingCursorRuns")]
    pub pending_cursor_runs: i32,
    #[graphql(name = "runningCursorRuns")]
    pub running_cursor_runs: i32,
    #[graphql(name = "failedCursorRuns")]
    pub failed_cursor_runs: i32,
    #[graphql(name = "completedRecentCursorRuns")]
    pub completed_recent_cursor_runs: i32,
    #[graphql(name = "intentActive")]
    pub intent_active: bool,
    #[graphql(name = "blockedReason")]
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeBlockedMailboxObject {
    #[graphql(name = "mailboxName")]
    pub mailbox_name: String,
    #[graphql(name = "displayName")]
    pub display_name: String,
    pub reason: String,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeEmbeddingsReadinessGateObject {
    pub blocked: bool,
    pub readiness: Option<String>,
    pub reason: Option<String>,
    #[graphql(name = "activeTaskId")]
    pub active_task_id: Option<String>,
    #[graphql(name = "profileName")]
    pub profile_name: Option<String>,
    #[graphql(name = "configPath")]
    pub config_path: Option<String>,
    #[graphql(name = "lastError")]
    pub last_error: Option<String>,
    #[graphql(name = "lastUpdatedUnix")]
    pub last_updated_unix: i64,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeSummaryBootstrapRunObject {
    #[graphql(name = "runId")]
    pub run_id: String,
    #[graphql(name = "repoId")]
    pub repo_id: String,
    #[graphql(name = "initSessionId")]
    pub init_session_id: ID,
    pub status: String,
    pub progress: RuntimeSummaryBootstrapProgressObject,
    pub request: RuntimeSummaryBootstrapRequestObject,
    pub result: Option<RuntimeSummaryBootstrapResultObject>,
    pub error: Option<String>,
    #[graphql(name = "submittedAtUnix")]
    pub submitted_at_unix: i64,
    #[graphql(name = "startedAtUnix")]
    pub started_at_unix: Option<i64>,
    #[graphql(name = "updatedAtUnix")]
    pub updated_at_unix: i64,
    #[graphql(name = "completedAtUnix")]
    pub completed_at_unix: Option<i64>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeSummaryBootstrapRequestObject {
    pub action: String,
    pub message: Option<String>,
    #[graphql(name = "modelName")]
    pub model_name: Option<String>,
    #[graphql(name = "gatewayUrlOverride")]
    pub gateway_url_override: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeSummaryBootstrapProgressObject {
    pub phase: String,
    #[graphql(name = "assetName")]
    pub asset_name: Option<String>,
    #[graphql(name = "bytesDownloaded")]
    pub bytes_downloaded: i64,
    #[graphql(name = "bytesTotal")]
    pub bytes_total: Option<i64>,
    pub version: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeSummaryBootstrapResultObject {
    #[graphql(name = "outcomeKind")]
    pub outcome_kind: String,
    #[graphql(name = "modelName")]
    pub model_name: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeInitSessionObject {
    #[graphql(name = "initSessionId")]
    pub init_session_id: ID,
    pub status: String,
    #[graphql(name = "waitingReason")]
    pub waiting_reason: Option<String>,
    #[graphql(name = "warningSummary")]
    pub warning_summary: Option<String>,
    #[graphql(name = "followUpSyncRequired")]
    pub follow_up_sync_required: bool,
    #[graphql(name = "runSync")]
    pub run_sync: bool,
    #[graphql(name = "runIngest")]
    pub run_ingest: bool,
    #[graphql(name = "embeddingsSelected")]
    pub embeddings_selected: bool,
    #[graphql(name = "summariesSelected")]
    pub summaries_selected: bool,
    #[graphql(name = "initialSyncTaskId")]
    pub initial_sync_task_id: Option<String>,
    #[graphql(name = "ingestTaskId")]
    pub ingest_task_id: Option<String>,
    #[graphql(name = "followUpSyncTaskId")]
    pub follow_up_sync_task_id: Option<String>,
    #[graphql(name = "embeddingsBootstrapTaskId")]
    pub embeddings_bootstrap_task_id: Option<String>,
    #[graphql(name = "summaryBootstrapTaskId")]
    pub summary_bootstrap_task_id: Option<ID>,
    #[graphql(name = "terminalError")]
    pub terminal_error: Option<String>,
    #[graphql(name = "topPipelineLane")]
    pub top_pipeline_lane: RuntimeInitLaneObject,
    #[graphql(name = "embeddingsLane")]
    pub embeddings_lane: RuntimeInitLaneObject,
    #[graphql(name = "summariesLane")]
    pub summaries_lane: RuntimeInitLaneObject,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeInitLaneObject {
    pub status: String,
    #[graphql(name = "waitingReason")]
    pub waiting_reason: Option<String>,
    pub detail: Option<String>,
    #[graphql(name = "activityLabel")]
    pub activity_label: Option<String>,
    #[graphql(name = "taskId")]
    pub task_id: Option<String>,
    #[graphql(name = "runId")]
    pub run_id: Option<ID>,
    pub progress: Option<RuntimeInitLaneProgressObject>,
    pub queue: RuntimeInitLaneQueueObject,
    pub warnings: Vec<RuntimeInitLaneWarningObject>,
    #[graphql(name = "pendingCount")]
    pub pending_count: i32,
    #[graphql(name = "runningCount")]
    pub running_count: i32,
    #[graphql(name = "failedCount")]
    pub failed_count: i32,
    #[graphql(name = "completedCount")]
    pub completed_count: i32,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeInitLaneProgressObject {
    pub completed: i32,
    pub total: i32,
    pub remaining: i32,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeInitLaneQueueObject {
    pub queued: i32,
    pub running: i32,
    pub failed: i32,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeInitLaneWarningObject {
    #[graphql(name = "componentLabel")]
    pub component_label: String,
    pub message: String,
    #[graphql(name = "retryCommand")]
    pub retry_command: String,
}

#[derive(Debug, Clone, SimpleObject)]
pub(crate) struct RuntimeEventObject {
    pub domain: String,
    #[graphql(name = "repoId")]
    pub repo_id: String,
    #[graphql(name = "initSessionId")]
    pub init_session_id: Option<ID>,
    #[graphql(name = "updatedAtUnix")]
    pub updated_at_unix: i64,
    #[graphql(name = "taskId")]
    pub task_id: Option<String>,
    #[graphql(name = "runId")]
    pub run_id: Option<String>,
    #[graphql(name = "mailboxName")]
    pub mailbox_name: Option<String>,
}

impl From<InitRuntimeSnapshot> for RuntimeSnapshotObject {
    fn from(value: InitRuntimeSnapshot) -> Self {
        Self {
            repo_id: value.repo_id,
            task_queue: value.task_queue.into(),
            current_state_consumer: value.current_state_consumer.into(),
            workplane: value.workplane.into(),
            blocked_mailboxes: value
                .blocked_mailboxes
                .into_iter()
                .map(Into::into)
                .collect(),
            embeddings_readiness_gate: value.embeddings_readiness_gate.map(Into::into),
            summaries_bootstrap: value.summaries_bootstrap.map(Into::into),
            current_init_session: value.current_init_session.map(Into::into),
        }
    }
}

impl From<CapabilityEventQueueStatus> for RuntimeCurrentStateConsumerObject {
    fn from(value: CapabilityEventQueueStatus) -> Self {
        Self {
            persisted: value.persisted,
            pending_runs: to_graphql_i32(value.state.pending_runs),
            running_runs: to_graphql_i32(value.state.running_runs),
            failed_runs: to_graphql_i32(value.state.failed_runs),
            completed_recent_runs: to_graphql_i32(value.state.completed_recent_runs),
            last_action: value.state.last_action,
            last_updated_unix: to_graphql_i64(value.state.last_updated_unix),
            current_repo_run: value.current_repo_run.map(Into::into),
        }
    }
}

impl From<CapabilityEventRunRecord> for RuntimeCurrentStateRunObject {
    fn from(value: CapabilityEventRunRecord) -> Self {
        Self {
            run_id: value.run_id,
            repo_id: value.repo_id,
            capability_id: value.capability_id,
            init_session_id: value.init_session_id.map(ID::from),
            consumer_id: value.consumer_id,
            handler_id: value.handler_id,
            from_generation_seq: to_graphql_i64(value.from_generation_seq),
            to_generation_seq: to_graphql_i64(value.to_generation_seq),
            reconcile_mode: value.reconcile_mode,
            status: value.status.to_string(),
            attempts: to_graphql_i32(value.attempts),
            submitted_at_unix: to_graphql_i64(value.submitted_at_unix),
            started_at_unix: value.started_at_unix.map(to_graphql_i64),
            updated_at_unix: to_graphql_i64(value.updated_at_unix),
            completed_at_unix: value.completed_at_unix.map(to_graphql_i64),
            error: value.error,
        }
    }
}

impl From<InitRuntimeWorkplaneSnapshot> for RuntimeWorkplaneObject {
    fn from(value: InitRuntimeWorkplaneSnapshot) -> Self {
        Self {
            pending_jobs: to_graphql_i32(value.pending_jobs),
            running_jobs: to_graphql_i32(value.running_jobs),
            failed_jobs: to_graphql_i32(value.failed_jobs),
            completed_recent_jobs: to_graphql_i32(value.completed_recent_jobs),
            pools: value.pools.into_iter().map(Into::into).collect(),
            mailboxes: value.mailboxes.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<InitRuntimeWorkplanePoolSnapshot> for RuntimeWorkplanePoolObject {
    fn from(value: InitRuntimeWorkplanePoolSnapshot) -> Self {
        Self {
            pool_name: value.pool_name,
            display_name: value.display_name,
            worker_budget: to_graphql_i32(value.worker_budget),
            active_workers: to_graphql_i32(value.active_workers),
            pending_jobs: to_graphql_i32(value.pending_jobs),
            running_jobs: to_graphql_i32(value.running_jobs),
            failed_jobs: to_graphql_i32(value.failed_jobs),
            completed_recent_jobs: to_graphql_i32(value.completed_recent_jobs),
        }
    }
}

impl From<InitRuntimeWorkplaneMailboxSnapshot> for RuntimeWorkplaneMailboxObject {
    fn from(value: InitRuntimeWorkplaneMailboxSnapshot) -> Self {
        Self {
            mailbox_name: value.mailbox_name,
            display_name: value.display_name,
            pending_jobs: to_graphql_i32(value.pending_jobs),
            running_jobs: to_graphql_i32(value.running_jobs),
            failed_jobs: to_graphql_i32(value.failed_jobs),
            completed_recent_jobs: to_graphql_i32(value.completed_recent_jobs),
            pending_cursor_runs: to_graphql_i32(value.pending_cursor_runs),
            running_cursor_runs: to_graphql_i32(value.running_cursor_runs),
            failed_cursor_runs: to_graphql_i32(value.failed_cursor_runs),
            completed_recent_cursor_runs: to_graphql_i32(value.completed_recent_cursor_runs),
            intent_active: value.intent_active,
            blocked_reason: value.blocked_reason,
        }
    }
}

impl From<crate::daemon::BlockedMailboxStatus> for RuntimeBlockedMailboxObject {
    fn from(value: crate::daemon::BlockedMailboxStatus) -> Self {
        Self {
            display_name: crate::runtime_presentation::mailbox_label(&value.mailbox_name)
                .to_string(),
            mailbox_name: value.mailbox_name,
            reason: value.reason,
        }
    }
}

impl From<EmbeddingsBootstrapGateStatus> for RuntimeEmbeddingsReadinessGateObject {
    fn from(value: EmbeddingsBootstrapGateStatus) -> Self {
        Self {
            blocked: value.blocked,
            readiness: value.readiness.map(|readiness| readiness.to_string()),
            reason: value.reason,
            active_task_id: value.active_task_id,
            profile_name: value.profile_name,
            config_path: value.config_path.map(|path| path.display().to_string()),
            last_error: value.last_error,
            last_updated_unix: to_graphql_i64(value.last_updated_unix),
        }
    }
}

impl From<SummaryBootstrapRunRecord> for RuntimeSummaryBootstrapRunObject {
    fn from(value: SummaryBootstrapRunRecord) -> Self {
        Self {
            run_id: value.run_id,
            repo_id: value.repo_id,
            init_session_id: ID::from(value.init_session_id),
            status: value.status.to_string(),
            progress: RuntimeSummaryBootstrapProgressObject {
                phase: value.progress.phase.to_string(),
                asset_name: value.progress.asset_name,
                bytes_downloaded: to_graphql_i64(value.progress.bytes_downloaded),
                bytes_total: value.progress.bytes_total.map(to_graphql_i64),
                version: value.progress.version,
                message: value.progress.message,
            },
            request: RuntimeSummaryBootstrapRequestObject {
                action: summary_bootstrap_action_name(value.request.action).to_string(),
                message: value.request.message,
                model_name: value.request.model_name,
                gateway_url_override: value.request.gateway_url_override,
            },
            result: value.result.map(Into::into),
            error: value.error,
            submitted_at_unix: to_graphql_i64(value.submitted_at_unix),
            started_at_unix: value.started_at_unix.map(to_graphql_i64),
            updated_at_unix: to_graphql_i64(value.updated_at_unix),
            completed_at_unix: value.completed_at_unix.map(to_graphql_i64),
        }
    }
}

impl From<SummaryBootstrapResultRecord> for RuntimeSummaryBootstrapResultObject {
    fn from(value: SummaryBootstrapResultRecord) -> Self {
        Self {
            outcome_kind: value.outcome_kind,
            model_name: value.model_name,
            message: value.message,
        }
    }
}

impl From<InitRuntimeSessionView> for RuntimeInitSessionObject {
    fn from(value: InitRuntimeSessionView) -> Self {
        Self {
            init_session_id: ID::from(value.init_session_id),
            status: value.status,
            waiting_reason: value.waiting_reason,
            warning_summary: value.warning_summary,
            follow_up_sync_required: value.follow_up_sync_required,
            run_sync: value.run_sync,
            run_ingest: value.run_ingest,
            embeddings_selected: value.embeddings_selected,
            summaries_selected: value.summaries_selected,
            initial_sync_task_id: value.initial_sync_task_id,
            ingest_task_id: value.ingest_task_id,
            follow_up_sync_task_id: value.follow_up_sync_task_id,
            embeddings_bootstrap_task_id: value.embeddings_bootstrap_task_id,
            summary_bootstrap_task_id: value.summary_bootstrap_task_id.map(ID::from),
            terminal_error: value.terminal_error,
            top_pipeline_lane: value.top_pipeline_lane.into(),
            embeddings_lane: value.embeddings_lane.into(),
            summaries_lane: value.summaries_lane.into(),
        }
    }
}

impl From<InitRuntimeLaneView> for RuntimeInitLaneObject {
    fn from(value: InitRuntimeLaneView) -> Self {
        Self {
            status: value.status,
            waiting_reason: value.waiting_reason,
            detail: value.detail,
            activity_label: value.activity_label,
            task_id: value.task_id,
            run_id: value.run_id.map(ID::from),
            progress: value.progress.map(Into::into),
            queue: value.queue.into(),
            warnings: value.warnings.into_iter().map(Into::into).collect(),
            pending_count: to_graphql_i32(value.pending_count),
            running_count: to_graphql_i32(value.running_count),
            failed_count: to_graphql_i32(value.failed_count),
            completed_count: to_graphql_i32(value.completed_count),
        }
    }
}

impl From<InitRuntimeLaneProgressView> for RuntimeInitLaneProgressObject {
    fn from(value: InitRuntimeLaneProgressView) -> Self {
        Self {
            completed: to_graphql_i32(value.completed),
            total: to_graphql_i32(value.total),
            remaining: to_graphql_i32(value.remaining),
        }
    }
}

impl From<InitRuntimeLaneQueueView> for RuntimeInitLaneQueueObject {
    fn from(value: InitRuntimeLaneQueueView) -> Self {
        Self {
            queued: to_graphql_i32(value.queued),
            running: to_graphql_i32(value.running),
            failed: to_graphql_i32(value.failed),
        }
    }
}

impl From<InitRuntimeLaneWarningView> for RuntimeInitLaneWarningObject {
    fn from(value: InitRuntimeLaneWarningView) -> Self {
        Self {
            component_label: value.component_label,
            message: value.message,
            retry_command: value.retry_command,
        }
    }
}

impl From<RuntimeEventRecord> for RuntimeEventObject {
    fn from(value: RuntimeEventRecord) -> Self {
        Self {
            domain: value.domain,
            repo_id: value.repo_id,
            init_session_id: value.init_session_id.map(ID::from),
            updated_at_unix: to_graphql_i64(value.updated_at_unix),
            task_id: value.task_id,
            run_id: value.run_id,
            mailbox_name: value.mailbox_name,
        }
    }
}

pub(crate) fn build_runtime_schema(
    state: DashboardState,
    request_context: RuntimeRequestContext,
) -> RuntimeGraphqlSchema {
    Schema::build(
        RuntimeQueryRoot,
        RuntimeMutationRoot,
        RuntimeSubscriptionRoot,
    )
    .data(state)
    .data(request_context)
    .limit_depth(MAX_DEVQL_QUERY_DEPTH)
    .limit_complexity(MAX_RUNTIME_QUERY_COMPLEXITY)
    .finish()
}

pub(crate) fn build_runtime_schema_template() -> RuntimeGraphqlSchema {
    Schema::build(
        RuntimeQueryRoot,
        RuntimeMutationRoot,
        RuntimeSubscriptionRoot,
    )
    .limit_depth(MAX_DEVQL_QUERY_DEPTH)
    .limit_complexity(MAX_RUNTIME_QUERY_COMPLEXITY)
    .finish()
}

pub fn runtime_schema_sdl() -> String {
    build_runtime_schema_template().sdl()
}

pub(crate) async fn runtime_graphql_handler(
    State(state): State<DashboardState>,
    headers: HeaderMap,
    request: GraphQLRequest,
) -> AxumResponse {
    let started = Instant::now();
    let request = request.into_inner();
    let signature = graphql_request_signature(&request);
    let repo_root = match parse_repo_root_header(&headers) {
        Ok(repo_root) => repo_root,
        Err(err) => {
            let response = crate::graphql::graphql_error_response(err).into_response();
            track_graphql_action(GraphqlActionTelemetry {
                repo_root: state.repo_root.as_path(),
                event: "bitloops devql runtime http",
                scope: "runtime",
                transport: "http",
                request_kind: &signature.0,
                operation_family: &signature.1,
                success: false,
                status: response.status(),
                duration: started.elapsed(),
            });
            return response;
        }
    };
    if let Err(err) = validate_repo_daemon_binding(&headers, &state, repo_root.as_deref()) {
        let response = crate::graphql::graphql_error_response(err).into_response();
        track_graphql_action(GraphqlActionTelemetry {
            repo_root: repo_root.as_deref().unwrap_or(state.repo_root.as_path()),
            event: "bitloops devql runtime http",
            scope: "runtime",
            transport: "http",
            request_kind: &signature.0,
            operation_family: &signature.1,
            success: false,
            status: response.status(),
            duration: started.elapsed(),
        });
        return response;
    }
    let request_context = RuntimeRequestContext {
        bound_repo_root: repo_root.clone(),
    };
    let (response, success) = execute_graphql_request(
        state.runtime_graphql_schema(),
        request.data(state.clone()).data(request_context),
        &headers,
    )
    .await;
    let response = response.into_response();
    track_graphql_action(GraphqlActionTelemetry {
        repo_root: state.repo_root.as_path(),
        event: "bitloops devql runtime http",
        scope: "runtime",
        transport: "http",
        request_kind: &signature.0,
        operation_family: &signature.1,
        success,
        status: response.status(),
        duration: started.elapsed(),
    });
    response
}

pub(crate) async fn runtime_graphql_ws_handler(
    State(state): State<DashboardState>,
    protocol: GraphQLProtocol,
    upgrade: WebSocketUpgrade,
    headers: HeaderMap,
) -> impl IntoResponse {
    let started = Instant::now();
    let repo_root = match parse_repo_root_header(&headers) {
        Ok(repo_root) => repo_root,
        Err(err) => return crate::graphql::graphql_error_response(err).into_response(),
    };
    if let Err(err) = validate_repo_daemon_binding(&headers, &state, repo_root.as_deref()) {
        return crate::graphql::graphql_error_response(err).into_response();
    }
    let schema = build_runtime_schema(
        state.clone(),
        RuntimeRequestContext {
            bound_repo_root: repo_root,
        },
    );
    let response = upgrade
        .protocols(async_graphql::http::ALL_WEBSOCKET_PROTOCOLS)
        .on_upgrade(move |stream| GraphQLWebSocket::new(stream, schema, protocol).serve())
        .into_response();
    track_graphql_action(GraphqlActionTelemetry {
        repo_root: state.repo_root.as_path(),
        event: "bitloops devql runtime ws",
        scope: "runtime",
        transport: "ws",
        request_kind: "subscription",
        operation_family: "anonymous",
        success: response.status().is_success()
            || response.status() == axum::http::StatusCode::SWITCHING_PROTOCOLS,
        status: response.status(),
        duration: started.elapsed(),
    });
    response
}

pub(crate) async fn runtime_graphql_playground_handler() -> impl IntoResponse {
    graphql_playground_response(
        "/devql/runtime",
        Some("/devql/runtime/ws"),
        "DevQL Runtime Explorer",
    )
}

pub(crate) async fn runtime_graphql_sdl_handler(
    State(state): State<DashboardState>,
) -> AxumResponse {
    (
        [("content-type", "text/plain; charset=utf-8")],
        state.runtime_graphql_schema().sdl(),
    )
        .into_response()
}

async fn resolve_runtime_devql_config(
    state: &DashboardState,
    request_context: &RuntimeRequestContext,
    repo_id: &str,
) -> std::result::Result<crate::host::devql::DevqlConfig, ApiError> {
    let repo_id = repo_id.trim();
    if repo_id.is_empty() {
        return Err(ApiError::bad_request("repo_id is required"));
    }

    if let Some(bound_repo_root) = request_context.bound_repo_root.as_deref() {
        let (bound_repo_root, bound_repo) = resolve_runtime_repo_identity(bound_repo_root)?;
        if bound_repo.repo_id != repo_id {
            return Err(ApiError::bad_request(format!(
                "runtime request repoId `{repo_id}` does not match bound repository `{}`",
                bound_repo.repo_id
            )));
        }
        return build_runtime_devql_config(state, bound_repo_root, bound_repo);
    }

    if let Ok((state_repo_root, state_repo)) = resolve_runtime_repo_identity(&state.repo_root)
        && state_repo.repo_id == repo_id
    {
        return build_runtime_devql_config(state, state_repo_root, state_repo);
    }

    let repo_root = resolve_repo_root_from_repo_id(state, repo_id).await?;
    let (repo_root, repo) = resolve_runtime_repo_identity(&repo_root)?;
    build_runtime_devql_config(state, repo_root, repo)
}

fn resolve_runtime_repo_identity(
    repo_root: &Path,
) -> std::result::Result<(PathBuf, crate::host::devql::RepoIdentity), ApiError> {
    let repo_root = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf());
    let repo = crate::host::devql::resolve_repo_identity(&repo_root).map_err(|err| {
        ApiError::internal(format!("failed to resolve repository identity: {err:#}"))
    })?;
    Ok((repo_root, repo))
}

fn build_runtime_devql_config(
    state: &DashboardState,
    repo_root: PathBuf,
    repo: crate::host::devql::RepoIdentity,
) -> std::result::Result<crate::host::devql::DevqlConfig, ApiError> {
    crate::host::devql::DevqlConfig::from_roots(state.config_root.clone(), repo_root, repo)
        .map_err(|err| ApiError::internal(format!("failed to resolve runtime config: {err:#}")))
}

fn map_runtime_api_error(error: ApiError) -> async_graphql::Error {
    match error.code {
        "bad_request" | "not_found" => bad_user_input_error(error.message),
        other => graphql_error(other, error.message),
    }
}

fn to_graphql_i32(value: impl TryInto<i32>) -> i32 {
    value.try_into().unwrap_or(i32::MAX)
}

fn to_graphql_i64(value: impl TryInto<i64>) -> i64 {
    value.try_into().unwrap_or(i64::MAX)
}

fn current_unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn summary_bootstrap_action_name(action: SummaryBootstrapAction) -> &'static str {
    match action {
        SummaryBootstrapAction::InstallRuntimeOnly => "install_runtime_only",
        SummaryBootstrapAction::InstallRuntimeOnlyPendingProbe => {
            "install_runtime_only_pending_probe"
        }
        SummaryBootstrapAction::ConfigureLocal => "configure_local",
        SummaryBootstrapAction::ConfigureCloud => "configure_cloud",
    }
}
