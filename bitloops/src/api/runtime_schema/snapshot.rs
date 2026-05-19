use std::path::PathBuf;

use async_graphql::{ID, Object, Result, SimpleObject};

use super::init_session::RuntimeInitSessionObject;
use super::util::{summary_bootstrap_action_name, to_graphql_i32, to_graphql_i64};
use crate::daemon::{
    CapabilityEventQueueStatus, CapabilityEventRunRecord, EmbeddingsBootstrapGateStatus,
    InitRuntimeOverviewSnapshot, InitRuntimeWorkplaneMailboxSnapshot,
    InitRuntimeWorkplanePoolSnapshot, InitRuntimeWorkplaneSnapshot, SummaryBootstrapResultRecord,
    SummaryBootstrapRunRecord,
};
use crate::graphql::{TaskQueueStatusObject, graphql_error};
use crate::host::devql::DevqlConfig;

#[derive(Clone)]
pub(crate) struct RuntimeSnapshotObject {
    repo_id: String,
    daemon_config_root: PathBuf,
    repo_root: PathBuf,
    task_queue: TaskQueueStatusObject,
    current_state_consumer: RuntimeCurrentStateConsumerObject,
    workplane: RuntimeWorkplaneObject,
    blocked_mailboxes: Vec<RuntimeBlockedMailboxObject>,
    embeddings_readiness_gate: Option<RuntimeEmbeddingsReadinessGateObject>,
    summaries_bootstrap: Option<RuntimeSummaryBootstrapRunObject>,
}

impl RuntimeSnapshotObject {
    pub(crate) fn from_overview(cfg: DevqlConfig, value: InitRuntimeOverviewSnapshot) -> Self {
        Self {
            daemon_config_root: cfg.daemon_config_root,
            repo_root: cfg.repo_root,
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
        }
    }
}

#[Object]
impl RuntimeSnapshotObject {
    #[graphql(name = "repoId")]
    async fn repo_id(&self) -> String {
        self.repo_id.clone()
    }

    #[graphql(name = "taskQueue")]
    async fn task_queue(&self) -> TaskQueueStatusObject {
        self.task_queue.clone()
    }

    #[graphql(name = "currentStateConsumer")]
    async fn current_state_consumer(&self) -> RuntimeCurrentStateConsumerObject {
        self.current_state_consumer.clone()
    }

    async fn workplane(&self) -> RuntimeWorkplaneObject {
        self.workplane.clone()
    }

    #[graphql(name = "blockedMailboxes")]
    async fn blocked_mailboxes(&self) -> Vec<RuntimeBlockedMailboxObject> {
        self.blocked_mailboxes.clone()
    }

    #[graphql(name = "embeddingsReadinessGate")]
    async fn embeddings_readiness_gate(&self) -> Option<RuntimeEmbeddingsReadinessGateObject> {
        self.embeddings_readiness_gate.clone()
    }

    #[graphql(name = "summariesBootstrap")]
    async fn summaries_bootstrap(&self) -> Option<RuntimeSummaryBootstrapRunObject> {
        self.summaries_bootstrap.clone()
    }

    #[graphql(name = "currentInitSession")]
    async fn current_init_session(&self) -> Result<Option<RuntimeInitSessionObject>> {
        crate::daemon::shared_init_runtime_coordinator()
            .current_session_for_repo_roots(
                &self.daemon_config_root,
                &self.repo_root,
                &self.repo_id,
            )
            .map(|session| session.map(Into::into))
            .map_err(|err| {
                graphql_error(
                    "internal",
                    format!("failed to load current init session: {err:#}"),
                )
            })
    }
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
    #[graphql(name = "apiKeyEnv")]
    pub api_key_env: Option<String>,
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
                api_key_env: value.request.api_key_env,
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
