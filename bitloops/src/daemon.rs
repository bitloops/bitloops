use anyhow::{Context, Result, bail};
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use clap::{Args, ValueEnum};
use reqwest::StatusCode as ReqwestStatusCode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use crate::api::{self, DashboardReadyHook, DashboardRuntimeOptions, DashboardServerConfig};
use crate::devql_transport::{SlimCliRepoScope, attach_slim_cli_scope_headers};

#[path = "daemon/auth.rs"]
mod auth;
#[path = "daemon/capability_events.rs"]
mod capability_events;
#[path = "daemon/config.rs"]
mod config;
#[path = "daemon/embeddings_bootstrap.rs"]
mod embeddings_bootstrap;
#[path = "daemon/enrichment.rs"]
mod enrichment;
#[path = "daemon/graphql_client.rs"]
mod graphql_client;
#[path = "daemon/lifecycle.rs"]
mod lifecycle;
#[path = "daemon/log_file.rs"]
mod log_file;
#[path = "daemon/logger.rs"]
mod logger;
#[path = "daemon/process.rs"]
mod process;
#[path = "daemon/server_runtime.rs"]
mod server_runtime;
#[path = "daemon/service_files.rs"]
mod service_files;
#[path = "daemon/service_manager.rs"]
mod service_manager;
#[path = "daemon/state_store.rs"]
mod state_store;
#[path = "daemon/supervisor_api.rs"]
mod supervisor_api;
#[path = "daemon/supervisor_client.rs"]
mod supervisor_client;
#[path = "daemon/tasks.rs"]
mod tasks;
#[path = "daemon/types.rs"]
mod types;

#[cfg(test)]
#[path = "daemon/tests.rs"]
mod tests;

pub(crate) use self::auth::PersistedWorkosAuthSessionState;
pub(crate) use self::auth::{
    PLATFORM_GATEWAY_TOKEN_ENV, load_workos_session_details_cached, platform_gateway_bearer_token,
};
pub use self::auth::{
    WorkosDeviceLoginStart, WorkosLoginStart, WorkosSessionDetails, complete_workos_device_login,
    logout_workos_session, prepare_workos_device_login, resolve_workos_session_status,
};
pub use self::capability_events::{CapabilityEventCoordinator, CapabilityEventEnqueueResult};
pub use self::enrichment::EnrichmentControlResult;
pub use self::enrichment::EnrichmentCoordinator;
pub use self::enrichment::EnrichmentJobTarget;
pub(crate) use self::enrichment::EnrichmentQueueState as PersistedEnrichmentQueueState;
pub use self::logger::{ProcessLogContext, daemon_log_file_path, init_process_logger};
pub use self::tasks::{DevqlTaskCoordinator, DevqlTaskEnqueueResult};
pub(crate) use self::types::EmbeddingsBootstrapState as PersistedEmbeddingsBootstrapState;
pub use self::types::{
    CapabilityEventQueueState, CapabilityEventQueueStatus, CapabilityEventRunRecord,
    CapabilityEventRunStatus, DaemonHealthSummary, DaemonMode, DaemonProcessModeArg,
    DaemonRuntimeState, DaemonServiceMetadata, DaemonStatusReport, DevqlTaskControlResult,
    DevqlTaskKind, DevqlTaskKindCounts, DevqlTaskProgress, DevqlTaskQueueState,
    DevqlTaskQueueStatus, DevqlTaskRecord, DevqlTaskResult, DevqlTaskSource, DevqlTaskSpec,
    DevqlTaskStatus, EmbeddingsBootstrapGateEntry, EmbeddingsBootstrapGateStatus,
    EmbeddingsBootstrapPhase, EmbeddingsBootstrapProgress, EmbeddingsBootstrapReadiness,
    EmbeddingsBootstrapResult, EmbeddingsBootstrapTaskSpec, EnrichmentQueueMode,
    EnrichmentQueueState, EnrichmentQueueStatus, FailedEmbeddingJobSummary, IngestTaskSpec,
    InternalDaemonProcessArgs, InternalDaemonSupervisorArgs, PostCommitSnapshotSpec,
    RepoTaskControlState, ResolvedDaemonConfig, ServiceManagerKind, SupervisorRuntimeState,
    SupervisorServiceMetadata, SyncTaskMode, SyncTaskSpec,
};
pub(crate) use self::types::{
    ENRICHMENT_STATE_FILE_NAME, SUPERVISOR_RUNTIME_STATE_FILE_NAME, SYNC_STATE_FILE_NAME,
};

use self::process::*;
use self::server_runtime::*;
use self::service_files::*;
use self::service_manager::*;
use self::state_store::*;
use self::supervisor_client::*;
#[cfg(test)]
use self::types::RUNTIME_STATE_FILE_NAME;
#[cfg(test)]
use self::types::global_daemon_dir_fallback;
use self::types::{
    GLOBAL_SUPERVISOR_SERVICE_NAME, INTERNAL_SUPERVISOR_COMMAND_NAME, READY_TIMEOUT, STOP_TIMEOUT,
    SupervisorAppState, SupervisorHealthResponse, SupervisorStartRequest, SupervisorStopRequest,
    global_daemon_dir, supervisor_service_metadata_path, unix_timestamp_now,
};

pub fn runtime_state_path(repo_root: &Path) -> PathBuf {
    types::runtime_state_path(repo_root)
}

#[cfg(test)]
pub(crate) fn repo_local_runtime_state_path_for_tests(repo_root: &Path) -> Option<PathBuf> {
    if repo_root.as_os_str().is_empty() || repo_root == Path::new(".") {
        return None;
    }
    Some(
        repo_root
            .join(".bitloops-test-state")
            .join("daemon")
            .join(RUNTIME_STATE_FILE_NAME),
    )
}

pub fn service_metadata_path(repo_root: &Path) -> PathBuf {
    types::service_metadata_path(repo_root)
}

pub fn current_binary_fingerprint() -> Result<String> {
    process::current_binary_fingerprint()
}

pub fn require_current_repo_runtime(
    repo_root: &Path,
    operation: &str,
) -> Result<DaemonRuntimeState> {
    #[cfg(test)]
    let runtime = repo_local_runtime_state_path_for_tests(repo_root)
        .map(|path| state_store::read_json::<DaemonRuntimeState>(&path))
        .transpose()?
        .flatten()
        .filter(|runtime| {
            runtime.pid == std::process::id() || process_is_running(runtime.pid).unwrap_or(false)
        })
        .or(
            state_store::read_runtime_state_legacy(repo_root)?.filter(|runtime| {
                runtime.pid == std::process::id()
                    || process_is_running(runtime.pid).unwrap_or(false)
            }),
        )
        .filter(|runtime| {
            runtime.pid == std::process::id() || process_is_running(runtime.pid).unwrap_or(false)
        })
        .or(state_store::read_runtime_state(repo_root)?);

    #[cfg(not(test))]
    let runtime = state_store::read_runtime_state(repo_root)?;

    let runtime = runtime.ok_or_else(|| {
        anyhow::anyhow!(
            "Bitloops daemon is not running for this repository. Run `bitloops init` or `bitloops daemon restart` before {operation}."
        )
    })?;
    let current = current_binary_fingerprint().unwrap_or_default();
    if !runtime.binary_fingerprint.is_empty()
        && !current.is_empty()
        && runtime.binary_fingerprint != current
    {
        if matches!(runtime.mode, DaemonMode::Foreground) {
            bail!(
                "Bitloops daemon is running in foreground with an older CLI binary. Restart it manually and rerun `bitloops init` before {operation}."
            );
        }
        bail!(
            "Bitloops daemon is stale for this repository. Run `bitloops init` or `bitloops daemon restart` before {operation}."
        );
    }
    Ok(runtime)
}

pub fn resolve_daemon_config(explicit_config_path: Option<&Path>) -> Result<ResolvedDaemonConfig> {
    config::resolve_daemon_config(explicit_config_path)
}

pub async fn run_internal_process(args: InternalDaemonProcessArgs) -> Result<()> {
    lifecycle::run_internal_process(args).await
}

pub async fn run_internal_supervisor(args: InternalDaemonSupervisorArgs) -> Result<()> {
    supervisor_api::run_internal_supervisor(args).await
}

pub async fn start_foreground(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
    open_browser: bool,
    ready_subject: &str,
    telemetry: Option<bool>,
) -> Result<()> {
    lifecycle::start_foreground(
        daemon_config,
        config,
        open_browser,
        ready_subject,
        telemetry,
    )
    .await
}

pub async fn start_detached(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
    telemetry: Option<bool>,
) -> Result<DaemonRuntimeState> {
    lifecycle::start_detached(daemon_config, config, telemetry).await
}

pub async fn start_service(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
    telemetry: Option<bool>,
) -> Result<DaemonRuntimeState> {
    lifecycle::start_service(daemon_config, config, telemetry).await
}

pub async fn restart(config_override: Option<&ResolvedDaemonConfig>) -> Result<DaemonRuntimeState> {
    lifecycle::restart(config_override).await
}

pub async fn stop() -> Result<()> {
    lifecycle::stop().await
}

pub fn uninstall_supervisor_service() -> Result<()> {
    let metadata = read_supervisor_service_metadata()?.unwrap_or_else(|| {
        let manager = current_service_manager();
        let service_file = match manager {
            ServiceManagerKind::Launchd => {
                launch_agent_plist_path(GLOBAL_SUPERVISOR_SERVICE_NAME).ok()
            }
            ServiceManagerKind::SystemdUser => {
                systemd_user_unit_path(GLOBAL_SUPERVISOR_SERVICE_NAME).ok()
            }
            ServiceManagerKind::WindowsTask => None,
        };

        SupervisorServiceMetadata {
            version: 1,
            manager,
            service_name: GLOBAL_SUPERVISOR_SERVICE_NAME.to_string(),
            service_file,
        }
    });

    uninstall_configured_supervisor_service(&metadata)
}

pub async fn status() -> Result<DaemonStatusReport> {
    lifecycle::status().await
}

pub fn enrichment_status() -> Result<EnrichmentQueueStatus> {
    enrichment::snapshot()
}

pub fn capability_event_status(repo_id: Option<&str>) -> Result<CapabilityEventQueueStatus> {
    CapabilityEventCoordinator::try_shared()?.snapshot(repo_id)
}

pub fn current_state_consumer_status(repo_id: Option<&str>) -> Result<CapabilityEventQueueStatus> {
    capability_event_status(repo_id)
}

pub fn pause_enrichments(reason: Option<String>) -> Result<EnrichmentControlResult> {
    enrichment::pause_enrichments(reason)
}

pub fn resume_enrichments() -> Result<EnrichmentControlResult> {
    enrichment::resume_enrichments()
}

pub fn retry_failed_enrichments() -> Result<EnrichmentControlResult> {
    enrichment::retry_failed_enrichments()
}

pub fn shared_enrichment_coordinator() -> Arc<EnrichmentCoordinator> {
    EnrichmentCoordinator::shared()
}

pub fn shared_capability_event_coordinator() -> Arc<CapabilityEventCoordinator> {
    CapabilityEventCoordinator::shared()
}

pub fn shared_current_state_consumer_coordinator() -> Arc<CapabilityEventCoordinator> {
    shared_capability_event_coordinator()
}

pub fn shared_devql_task_coordinator() -> Arc<DevqlTaskCoordinator> {
    DevqlTaskCoordinator::shared()
}

pub(crate) fn activate_task_worker(
    config_root: &Path,
    repo_registry_path: Option<&Path>,
    subscription_hub: Arc<crate::graphql::SubscriptionHub>,
) {
    CapabilityEventCoordinator::shared().activate_worker();
    DevqlTaskCoordinator::shared().activate_worker(
        config_root,
        repo_registry_path,
        Some(subscription_hub),
    );
}

pub fn devql_task_status(repo_id: Option<&str>) -> Result<DevqlTaskQueueStatus> {
    DevqlTaskCoordinator::shared().snapshot(repo_id)
}

pub fn devql_task(task_id: &str) -> Result<Option<DevqlTaskRecord>> {
    DevqlTaskCoordinator::shared().task(task_id)
}

pub fn devql_tasks(
    repo_id: Option<&str>,
    kind: Option<DevqlTaskKind>,
    status: Option<DevqlTaskStatus>,
    limit: Option<usize>,
) -> Result<Vec<DevqlTaskRecord>> {
    DevqlTaskCoordinator::shared().tasks(repo_id, kind, status, limit)
}

pub fn enqueue_task_for_config(
    cfg: &crate::host::devql::DevqlConfig,
    source: DevqlTaskSource,
    spec: DevqlTaskSpec,
) -> Result<DevqlTaskEnqueueResult> {
    DevqlTaskCoordinator::shared().enqueue(cfg, source, spec)
}

pub fn enqueue_sync_for_config(
    cfg: &crate::host::devql::DevqlConfig,
    source: DevqlTaskSource,
    mode: crate::host::devql::SyncMode,
) -> Result<DevqlTaskEnqueueResult> {
    enqueue_sync_for_config_with_snapshot(cfg, source, mode, None)
}

pub fn enqueue_sync_for_config_with_snapshot(
    cfg: &crate::host::devql::DevqlConfig,
    source: DevqlTaskSource,
    mode: crate::host::devql::SyncMode,
    post_commit_snapshot: Option<PostCommitSnapshotSpec>,
) -> Result<DevqlTaskEnqueueResult> {
    enqueue_task_for_config(
        cfg,
        source,
        DevqlTaskSpec::Sync(build_sync_task_spec(mode, post_commit_snapshot)),
    )
}

fn build_sync_task_spec(
    mode: crate::host::devql::SyncMode,
    post_commit_snapshot: Option<PostCommitSnapshotSpec>,
) -> SyncTaskSpec {
    SyncTaskSpec {
        mode: match mode {
            crate::host::devql::SyncMode::Auto => SyncTaskMode::Auto,
            crate::host::devql::SyncMode::Full => SyncTaskMode::Full,
            crate::host::devql::SyncMode::Paths(paths) => SyncTaskMode::Paths { paths },
            crate::host::devql::SyncMode::Repair => SyncTaskMode::Repair,
            crate::host::devql::SyncMode::Validate => SyncTaskMode::Validate,
        },
        post_commit_snapshot,
    }
}

pub fn enqueue_ingest_for_config(
    cfg: &crate::host::devql::DevqlConfig,
    source: DevqlTaskSource,
    backfill: Option<usize>,
) -> Result<DevqlTaskEnqueueResult> {
    enqueue_task_for_config(
        cfg,
        source,
        DevqlTaskSpec::Ingest(IngestTaskSpec { backfill }),
    )
}

pub fn enqueue_embeddings_bootstrap_for_config(
    cfg: &crate::host::devql::DevqlConfig,
    source: DevqlTaskSource,
    config_path: PathBuf,
    profile_name: String,
) -> Result<DevqlTaskEnqueueResult> {
    enqueue_task_for_config(
        cfg,
        source,
        DevqlTaskSpec::EmbeddingsBootstrap(EmbeddingsBootstrapTaskSpec {
            config_path,
            profile_name,
        }),
    )
}

pub fn pause_devql_tasks(repo_id: &str, reason: Option<String>) -> Result<DevqlTaskControlResult> {
    DevqlTaskCoordinator::shared().pause_repo(repo_id, reason)
}

pub fn resume_devql_tasks(repo_id: &str) -> Result<DevqlTaskControlResult> {
    DevqlTaskCoordinator::shared().resume_repo(repo_id)
}

pub fn cancel_devql_task(task_id: &str) -> Result<DevqlTaskRecord> {
    DevqlTaskCoordinator::shared().cancel_task(task_id)
}

pub async fn wait_until_ready(timeout: Duration) -> Result<DaemonRuntimeState> {
    lifecycle::wait_until_ready(timeout).await
}

pub async fn execute_graphql<T: DeserializeOwned>(
    repo_root: &Path,
    query: &str,
    variables: Value,
) -> Result<T> {
    graphql_client::execute_graphql(repo_root, query, variables).await
}

pub async fn execute_repo_graphql<T: DeserializeOwned>(
    repo_root: &Path,
    query: &str,
    variables: Value,
) -> Result<T> {
    graphql_client::execute_repo_graphql(repo_root, query, variables).await
}

pub(crate) async fn execute_slim_graphql<T: DeserializeOwned>(
    repo_root: &Path,
    scope: &SlimCliRepoScope,
    query: &str,
    variables: Value,
) -> Result<T> {
    graphql_client::execute_slim_graphql(repo_root, scope, query, variables).await
}

pub fn choose_dashboard_launch_mode() -> Result<Option<DaemonMode>> {
    graphql_client::choose_dashboard_launch_mode()
}

pub fn daemon_url() -> Result<Option<String>> {
    graphql_client::daemon_url()
}

pub(crate) fn daemon_http_client(url: &str) -> Result<reqwest::Client> {
    process::daemon_http_client(url)
}
