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

#[path = "daemon/config.rs"]
mod config;
#[path = "daemon/enrichment.rs"]
mod enrichment;
#[path = "daemon/graphql_client.rs"]
mod graphql_client;
#[path = "daemon/lifecycle.rs"]
mod lifecycle;
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
#[path = "daemon/sync.rs"]
mod sync;
#[path = "daemon/state_store.rs"]
mod state_store;
#[path = "daemon/supervisor_api.rs"]
mod supervisor_api;
#[path = "daemon/supervisor_client.rs"]
mod supervisor_client;
#[path = "daemon/types.rs"]
mod types;

#[cfg(test)]
#[path = "daemon/tests.rs"]
mod tests;

pub use self::enrichment::EnrichmentControlResult;
pub use self::enrichment::EnrichmentCoordinator;
pub use self::enrichment::EnrichmentJobTarget;
pub use self::logger::{ProcessLogContext, daemon_log_file_path, init_process_logger};
pub use self::sync::{SyncCoordinator, SyncEnqueueResult};
pub use self::types::{
    DaemonHealthSummary, DaemonMode, DaemonProcessModeArg, DaemonRuntimeState,
    DaemonServiceMetadata, DaemonStatusReport, EnrichmentQueueMode, EnrichmentQueueState,
    EnrichmentQueueStatus, InternalDaemonProcessArgs, InternalDaemonSupervisorArgs,
    ResolvedDaemonConfig, ServiceManagerKind, SupervisorRuntimeState, SupervisorServiceMetadata,
    SyncQueueState, SyncQueueStatus, SyncTaskMode, SyncTaskRecord, SyncTaskSource,
    SyncTaskStatus,
};

use self::process::*;
use self::server_runtime::*;
use self::service_files::*;
use self::service_manager::*;
use self::state_store::*;
use self::supervisor_client::*;
#[cfg(test)]
use self::types::global_daemon_dir_fallback;
use self::types::{
    GLOBAL_SUPERVISOR_SERVICE_NAME, INTERNAL_SUPERVISOR_COMMAND_NAME, READY_TIMEOUT, STOP_TIMEOUT,
    SupervisorAppState, SupervisorHealthResponse, SupervisorStartRequest, SupervisorStopRequest,
    global_daemon_dir, supervisor_runtime_state_path, supervisor_service_metadata_path,
    unix_timestamp_now,
};

pub fn runtime_state_path(repo_root: &Path) -> PathBuf {
    types::runtime_state_path(repo_root)
}

pub fn service_metadata_path(repo_root: &Path) -> PathBuf {
    types::service_metadata_path(repo_root)
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

pub fn shared_sync_coordinator() -> Arc<SyncCoordinator> {
    SyncCoordinator::shared()
}

pub(crate) fn activate_sync_worker(subscription_hub: Arc<crate::graphql::SubscriptionHub>) {
    SyncCoordinator::shared().activate_worker(Some(subscription_hub));
}

pub fn sync_status(repo_id: Option<&str>) -> Result<SyncQueueStatus> {
    SyncCoordinator::shared().snapshot(repo_id)
}

pub fn sync_task(task_id: &str) -> Result<Option<SyncTaskRecord>> {
    SyncCoordinator::shared().task(task_id)
}

pub fn sync_tasks(repo_id: Option<&str>, limit: Option<usize>) -> Result<Vec<SyncTaskRecord>> {
    SyncCoordinator::shared().tasks(repo_id, limit)
}

pub fn enqueue_sync_for_config(
    cfg: &crate::host::devql::DevqlConfig,
    source: SyncTaskSource,
    mode: crate::host::devql::SyncMode,
) -> Result<SyncEnqueueResult> {
    SyncCoordinator::shared().enqueue(cfg, source, mode)
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
