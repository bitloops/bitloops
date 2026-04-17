use super::*;
use std::collections::BTreeMap;

pub(super) const RUNTIME_STATE_FILE_NAME: &str = "runtime.json";
pub(super) const SERVICE_STATE_FILE_NAME: &str = "service.json";
pub(crate) const ENRICHMENT_STATE_FILE_NAME: &str = "enrichment.json";
pub(crate) const SYNC_STATE_FILE_NAME: &str = "sync.json";
pub(super) const INTERNAL_DAEMON_COMMAND_NAME: &str = "__daemon-process";
pub(super) const INTERNAL_SUPERVISOR_COMMAND_NAME: &str = "__daemon-supervisor";
pub(super) const GLOBAL_SUPERVISOR_SERVICE_NAME: &str = "com.bitloops.daemon";
pub(crate) const SUPERVISOR_RUNTIME_STATE_FILE_NAME: &str = "supervisor-runtime.json";
pub(super) const SUPERVISOR_SERVICE_STATE_FILE_NAME: &str = "supervisor-service.json";
pub(super) const READY_TIMEOUT: Duration = Duration::from_secs(20);
pub(super) const STOP_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DaemonMode {
    Foreground,
    Detached,
    Service,
}

impl fmt::Display for DaemonMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Foreground => write!(f, "foreground"),
            Self::Detached => write!(f, "detached"),
            Self::Service => write!(f, "always-on service"),
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DaemonProcessModeArg {
    Detached,
    Service,
}

impl From<DaemonProcessModeArg> for DaemonMode {
    fn from(value: DaemonProcessModeArg) -> Self {
        match value {
            DaemonProcessModeArg::Detached => Self::Detached,
            DaemonProcessModeArg::Service => Self::Service,
        }
    }
}

#[derive(Debug, Clone, Args)]
pub struct InternalDaemonProcessArgs {
    #[arg(long)]
    pub config_path: PathBuf,

    #[arg(long, value_enum)]
    pub mode: DaemonProcessModeArg,

    #[arg(long)]
    pub host: Option<String>,

    #[arg(long, default_value_t = crate::api::DEFAULT_DASHBOARD_PORT)]
    pub port: u16,

    #[arg(long, default_value_t = false)]
    pub http: bool,

    #[arg(long = "recheck-local-dashboard-net", default_value_t = false)]
    pub recheck_local_dashboard_net: bool,

    #[arg(long = "bundle-dir")]
    pub bundle_dir: Option<PathBuf>,

    #[arg(long)]
    pub service_name: Option<String>,

    #[arg(long)]
    pub telemetry: Option<bool>,
}

#[derive(Debug, Clone, Args, Default)]
pub struct InternalDaemonSupervisorArgs {}

impl InternalDaemonProcessArgs {
    pub fn from_server_config(
        daemon_config: &ResolvedDaemonConfig,
        mode: DaemonMode,
        service_name: Option<String>,
        config: &DashboardServerConfig,
        telemetry: Option<bool>,
    ) -> Self {
        Self {
            config_path: daemon_config.config_path.clone(),
            mode: match mode {
                DaemonMode::Detached => DaemonProcessModeArg::Detached,
                DaemonMode::Service => DaemonProcessModeArg::Service,
                DaemonMode::Foreground => DaemonProcessModeArg::Detached,
            },
            host: config.host.clone(),
            port: config.port,
            http: config.force_http,
            recheck_local_dashboard_net: config.recheck_local_dashboard_net,
            bundle_dir: config.bundle_dir.clone(),
            service_name,
            telemetry,
        }
    }

    pub fn server_config(&self) -> DashboardServerConfig {
        DashboardServerConfig {
            host: self.host.clone(),
            port: self.port,
            no_open: true,
            force_http: self.http,
            recheck_local_dashboard_net: self.recheck_local_dashboard_net,
            bundle_dir: self.bundle_dir.clone(),
        }
    }

    pub fn argv(&self) -> Vec<OsString> {
        let mut argv = vec![
            OsString::from(INTERNAL_DAEMON_COMMAND_NAME),
            OsString::from("--config-path"),
            self.config_path.clone().into_os_string(),
            OsString::from("--mode"),
            OsString::from(match self.mode {
                DaemonProcessModeArg::Detached => "detached",
                DaemonProcessModeArg::Service => "service",
            }),
        ];
        if let Some(host) = &self.host {
            argv.push(OsString::from("--host"));
            argv.push(OsString::from(host));
        }
        argv.push(OsString::from("--port"));
        argv.push(OsString::from(self.port.to_string()));
        if self.http {
            argv.push(OsString::from("--http"));
        }
        if self.recheck_local_dashboard_net {
            argv.push(OsString::from("--recheck-local-dashboard-net"));
        }
        if let Some(bundle_dir) = &self.bundle_dir {
            argv.push(OsString::from("--bundle-dir"));
            argv.push(bundle_dir.clone().into_os_string());
        }
        if let Some(service_name) = &self.service_name {
            argv.push(OsString::from("--service-name"));
            argv.push(OsString::from(service_name));
        }
        if let Some(telemetry) = self.telemetry {
            argv.push(OsString::from("--telemetry"));
            argv.push(OsString::from(telemetry.to_string()));
        }
        argv
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonRuntimeState {
    pub version: u8,
    pub config_path: PathBuf,
    pub config_root: PathBuf,
    pub pid: u32,
    pub mode: DaemonMode,
    pub service_name: Option<String>,
    pub url: String,
    pub host: String,
    pub port: u16,
    pub bundle_dir: PathBuf,
    pub relational_db_path: PathBuf,
    pub events_db_path: PathBuf,
    pub blob_store_path: PathBuf,
    pub repo_registry_path: PathBuf,
    pub binary_fingerprint: String,
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisorRuntimeState {
    pub version: u8,
    pub pid: u32,
    pub control_url: String,
    pub binary_fingerprint: String,
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceManagerKind {
    Launchd,
    SystemdUser,
    WindowsTask,
}

impl fmt::Display for ServiceManagerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Launchd => write!(f, "launchd"),
            Self::SystemdUser => write!(f, "systemd --user"),
            Self::WindowsTask => write!(f, "Windows Scheduled Task"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonServiceMetadata {
    pub version: u8,
    pub config_path: PathBuf,
    pub config_root: PathBuf,
    pub manager: ServiceManagerKind,
    pub service_name: String,
    pub service_file: Option<PathBuf>,
    pub config: DashboardServerConfig,
    pub last_url: Option<String>,
    pub last_pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisorServiceMetadata {
    pub version: u8,
    pub manager: ServiceManagerKind,
    pub service_name: String,
    pub service_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DaemonHealthSummary {
    pub relational_backend: Option<String>,
    pub relational_connected: Option<bool>,
    pub events_backend: Option<String>,
    pub events_connected: Option<bool>,
    pub blob_backend: Option<String>,
    pub blob_connected: Option<bool>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnrichmentQueueMode {
    Running,
    Paused,
}

impl fmt::Display for EnrichmentQueueMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Paused => write!(f, "paused"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnrichmentWorkerPoolKind {
    SummaryRefresh,
    Embeddings,
    CloneRebuild,
}

impl fmt::Display for EnrichmentWorkerPoolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SummaryRefresh => write!(f, "summary_refresh"),
            Self::Embeddings => write!(f, "embeddings"),
            Self::CloneRebuild => write!(f, "clone_rebuild"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentWorkerPoolStatus {
    pub kind: EnrichmentWorkerPoolKind,
    pub worker_budget: u64,
    pub active_workers: u64,
    pub pending_jobs: u64,
    pub running_jobs: u64,
    pub failed_jobs: u64,
    pub completed_recent_jobs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentQueueState {
    pub version: u8,
    pub mode: EnrichmentQueueMode,
    #[serde(default)]
    pub worker_pools: Vec<EnrichmentWorkerPoolStatus>,
    pub pending_jobs: u64,
    pub pending_work_items: u64,
    pub pending_semantic_jobs: u64,
    pub pending_semantic_work_items: u64,
    pub pending_embedding_jobs: u64,
    pub pending_embedding_work_items: u64,
    pub pending_clone_edges_rebuild_jobs: u64,
    pub pending_clone_edges_rebuild_work_items: u64,
    #[serde(default)]
    pub completed_recent_jobs: u64,
    pub running_jobs: u64,
    pub running_work_items: u64,
    pub running_semantic_jobs: u64,
    pub running_semantic_work_items: u64,
    pub running_embedding_jobs: u64,
    pub running_embedding_work_items: u64,
    pub running_clone_edges_rebuild_jobs: u64,
    pub running_clone_edges_rebuild_work_items: u64,
    pub failed_jobs: u64,
    pub failed_work_items: u64,
    pub failed_semantic_jobs: u64,
    pub failed_semantic_work_items: u64,
    pub failed_embedding_jobs: u64,
    pub failed_embedding_work_items: u64,
    pub failed_clone_edges_rebuild_jobs: u64,
    pub failed_clone_edges_rebuild_work_items: u64,
    pub retried_failed_jobs: u64,
    pub last_action: Option<String>,
    pub last_updated_unix: u64,
    pub paused_reason: Option<String>,
}

impl Default for EnrichmentQueueState {
    fn default() -> Self {
        Self {
            version: 1,
            mode: EnrichmentQueueMode::Running,
            worker_pools: Vec::new(),
            pending_jobs: 0,
            pending_work_items: 0,
            pending_semantic_jobs: 0,
            pending_semantic_work_items: 0,
            pending_embedding_jobs: 0,
            pending_embedding_work_items: 0,
            pending_clone_edges_rebuild_jobs: 0,
            pending_clone_edges_rebuild_work_items: 0,
            completed_recent_jobs: 0,
            running_jobs: 0,
            running_work_items: 0,
            running_semantic_jobs: 0,
            running_semantic_work_items: 0,
            running_embedding_jobs: 0,
            running_embedding_work_items: 0,
            running_clone_edges_rebuild_jobs: 0,
            running_clone_edges_rebuild_work_items: 0,
            failed_jobs: 0,
            failed_work_items: 0,
            failed_semantic_jobs: 0,
            failed_semantic_work_items: 0,
            failed_embedding_jobs: 0,
            failed_embedding_work_items: 0,
            failed_clone_edges_rebuild_jobs: 0,
            failed_clone_edges_rebuild_work_items: 0,
            retried_failed_jobs: 0,
            last_action: Some("initialized".to_string()),
            last_updated_unix: 0,
            paused_reason: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FailedEmbeddingJobSummary {
    pub job_id: String,
    pub repo_id: String,
    pub branch: String,
    pub representation_kind: String,
    pub artefact_count: u64,
    pub attempts: u32,
    pub error: Option<String>,
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlockedMailboxStatus {
    pub mailbox_name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnrichmentQueueStatus {
    pub state: EnrichmentQueueState,
    pub persisted: bool,
    pub embeddings_gate: Option<EmbeddingsBootstrapGateStatus>,
    pub blocked_mailboxes: Vec<BlockedMailboxStatus>,
    pub last_failed_embedding: Option<FailedEmbeddingJobSummary>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingsBootstrapReadiness {
    Pending,
    Ready,
    Failed,
}

impl fmt::Display for EmbeddingsBootstrapReadiness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Ready => write!(f, "ready"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingsBootstrapGateEntry {
    pub config_path: PathBuf,
    pub profile_name: String,
    pub readiness: EmbeddingsBootstrapReadiness,
    pub active_task_id: Option<String>,
    pub last_error: Option<String>,
    pub last_updated_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingsBootstrapState {
    pub version: u8,
    pub entries: BTreeMap<String, EmbeddingsBootstrapGateEntry>,
    pub last_action: Option<String>,
    pub updated_at_unix: u64,
}

impl Default for EmbeddingsBootstrapState {
    fn default() -> Self {
        Self {
            version: 1,
            entries: BTreeMap::new(),
            last_action: Some("initialized".to_string()),
            updated_at_unix: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EmbeddingsBootstrapGateStatus {
    pub blocked: bool,
    pub readiness: Option<EmbeddingsBootstrapReadiness>,
    pub reason: Option<String>,
    pub active_task_id: Option<String>,
    pub profile_name: Option<String>,
    pub config_path: Option<PathBuf>,
    pub last_error: Option<String>,
    pub last_updated_unix: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SummaryBootstrapPhase {
    Queued,
    ResolvingRelease,
    DownloadingRuntime,
    ExtractingRuntime,
    RewritingRuntime,
    WritingProfile,
    Complete,
}

impl fmt::Display for SummaryBootstrapPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Queued => write!(f, "queued"),
            Self::ResolvingRelease => write!(f, "resolving_release"),
            Self::DownloadingRuntime => write!(f, "downloading_runtime"),
            Self::ExtractingRuntime => write!(f, "extracting_runtime"),
            Self::RewritingRuntime => write!(f, "rewriting_runtime"),
            Self::WritingProfile => write!(f, "writing_profile"),
            Self::Complete => write!(f, "complete"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SummaryBootstrapProgress {
    pub phase: SummaryBootstrapPhase,
    pub asset_name: Option<String>,
    pub bytes_downloaded: u64,
    pub bytes_total: Option<u64>,
    pub version: Option<String>,
    pub message: Option<String>,
}

impl Default for SummaryBootstrapProgress {
    fn default() -> Self {
        Self {
            phase: SummaryBootstrapPhase::Queued,
            asset_name: None,
            bytes_downloaded: 0,
            bytes_total: None,
            version: None,
            message: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SummaryBootstrapStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

impl fmt::Display for SummaryBootstrapStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Queued => write!(f, "queued"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SummaryBootstrapAction {
    InstallRuntimeOnly,
    InstallRuntimeOnlyPendingProbe,
    ConfigureLocal,
    ConfigureCloud,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SummaryBootstrapRequest {
    pub action: SummaryBootstrapAction,
    pub message: Option<String>,
    pub model_name: Option<String>,
    pub gateway_url_override: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SummaryBootstrapResultRecord {
    pub outcome_kind: String,
    pub model_name: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SummaryBootstrapRunRecord {
    pub run_id: String,
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub init_session_id: String,
    pub request: SummaryBootstrapRequest,
    pub status: SummaryBootstrapStatus,
    pub progress: SummaryBootstrapProgress,
    pub result: Option<SummaryBootstrapResultRecord>,
    pub error: Option<String>,
    pub submitted_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub updated_at_unix: u64,
    pub completed_at_unix: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SummaryBootstrapState {
    pub version: u8,
    pub runs: Vec<SummaryBootstrapRunRecord>,
    pub last_action: Option<String>,
    pub updated_at_unix: u64,
}

impl Default for SummaryBootstrapState {
    fn default() -> Self {
        Self {
            version: 1,
            runs: Vec::new(),
            last_action: Some("initialized".to_string()),
            updated_at_unix: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InitEmbeddingsBootstrapRequest {
    pub config_path: PathBuf,
    pub profile_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartInitSessionSelections {
    pub run_sync: bool,
    pub run_ingest: bool,
    pub ingest_backfill: Option<usize>,
    pub embeddings_bootstrap: Option<InitEmbeddingsBootstrapRequest>,
    pub summaries_bootstrap: Option<SummaryBootstrapRequest>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InitSessionTerminalStatus {
    Completed,
    CompletedWithWarnings,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InitSessionRecord {
    pub init_session_id: String,
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub daemon_config_root: PathBuf,
    pub selections: StartInitSessionSelections,
    pub initial_sync_task_id: Option<String>,
    pub ingest_task_id: Option<String>,
    pub embeddings_bootstrap_task_id: Option<String>,
    pub summary_bootstrap_run_id: Option<String>,
    pub follow_up_sync_required: bool,
    pub follow_up_sync_task_id: Option<String>,
    pub submitted_at_unix: u64,
    pub updated_at_unix: u64,
    pub terminal_status: Option<InitSessionTerminalStatus>,
    pub terminal_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InitSessionState {
    pub version: u8,
    pub sessions: Vec<InitSessionRecord>,
    pub last_action: Option<String>,
    pub updated_at_unix: u64,
}

impl Default for InitSessionState {
    fn default() -> Self {
        Self {
            version: 1,
            sessions: Vec::new(),
            last_action: Some("initialized".to_string()),
            updated_at_unix: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityEventRunStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl fmt::Display for CapabilityEventRunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Queued => write!(f, "queued"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityEventRunRecord {
    pub run_id: String,
    pub repo_id: String,
    pub capability_id: String,
    #[serde(default)]
    pub init_session_id: Option<String>,
    #[serde(default)]
    pub consumer_id: String,
    #[serde(default)]
    pub handler_id: String,
    #[serde(default)]
    pub from_generation_seq: u64,
    #[serde(default)]
    pub to_generation_seq: u64,
    #[serde(default)]
    pub reconcile_mode: String,
    #[serde(default)]
    pub event_kind: String,
    #[serde(default)]
    pub lane_key: String,
    #[serde(default)]
    pub event_payload_json: String,
    pub status: CapabilityEventRunStatus,
    pub attempts: u32,
    pub submitted_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub updated_at_unix: u64,
    pub completed_at_unix: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityEventQueueState {
    pub version: u8,
    pub pending_runs: u64,
    pub running_runs: u64,
    pub failed_runs: u64,
    pub completed_recent_runs: u64,
    pub last_action: Option<String>,
    pub last_updated_unix: u64,
}

impl Default for CapabilityEventQueueState {
    fn default() -> Self {
        Self {
            version: 1,
            pending_runs: 0,
            running_runs: 0,
            failed_runs: 0,
            completed_recent_runs: 0,
            last_action: Some("initialized".to_string()),
            last_updated_unix: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityEventQueueStatus {
    pub state: CapabilityEventQueueState,
    pub persisted: bool,
    pub current_repo_run: Option<CapabilityEventRunRecord>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DevqlTaskKind {
    Sync,
    Ingest,
    EmbeddingsBootstrap,
}

impl fmt::Display for DevqlTaskKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sync => write!(f, "sync"),
            Self::Ingest => write!(f, "ingest"),
            Self::EmbeddingsBootstrap => write!(f, "embeddings_bootstrap"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DevqlTaskSource {
    Init,
    ManualCli,
    Watcher,
    PostCommit,
    PostMerge,
    PostCheckout,
    RepoPolicyChange,
}

impl fmt::Display for DevqlTaskSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Init => write!(f, "init"),
            Self::ManualCli => write!(f, "manual_cli"),
            Self::Watcher => write!(f, "watcher"),
            Self::PostCommit => write!(f, "post_commit"),
            Self::PostMerge => write!(f, "post_merge"),
            Self::PostCheckout => write!(f, "post_checkout"),
            Self::RepoPolicyChange => write!(f, "repo_policy_change"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyncTaskMode {
    Auto,
    Full,
    Paths { paths: Vec<String> },
    Repair,
    Validate,
}

impl fmt::Display for SyncTaskMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => write!(f, "auto"),
            Self::Full => write!(f, "full"),
            Self::Paths { .. } => write!(f, "paths"),
            Self::Repair => write!(f, "repair"),
            Self::Validate => write!(f, "validate"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DevqlTaskStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl fmt::Display for DevqlTaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Queued => write!(f, "queued"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostCommitSnapshotSpec {
    pub commit_sha: String,
    pub changed_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncTaskSpec {
    pub mode: SyncTaskMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_commit_snapshot: Option<PostCommitSnapshotSpec>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IngestTaskSpec {
    pub backfill: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingsBootstrapTaskSpec {
    pub config_path: PathBuf,
    pub profile_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum DevqlTaskSpec {
    Sync(SyncTaskSpec),
    Ingest(IngestTaskSpec),
    EmbeddingsBootstrap(EmbeddingsBootstrapTaskSpec),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingsBootstrapPhase {
    Queued,
    PreparingConfig,
    ResolvingRelease,
    DownloadingRuntime,
    ExtractingRuntime,
    RewritingRuntime,
    WarmingProfile,
    Complete,
    Failed,
}

impl EmbeddingsBootstrapPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::PreparingConfig => "preparing_config",
            Self::ResolvingRelease => "resolving_release",
            Self::DownloadingRuntime => "downloading_runtime",
            Self::ExtractingRuntime => "extracting_runtime",
            Self::RewritingRuntime => "rewriting_runtime",
            Self::WarmingProfile => "warming_profile",
            Self::Complete => "complete",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingsBootstrapProgress {
    pub phase: EmbeddingsBootstrapPhase,
    pub asset_name: Option<String>,
    pub bytes_downloaded: u64,
    pub bytes_total: Option<u64>,
    pub version: Option<String>,
    pub message: Option<String>,
}

impl Default for EmbeddingsBootstrapProgress {
    fn default() -> Self {
        Self {
            phase: EmbeddingsBootstrapPhase::Queued,
            asset_name: None,
            bytes_downloaded: 0,
            bytes_total: None,
            version: None,
            message: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingsBootstrapResult {
    pub version: Option<String>,
    pub binary_path: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    pub runtime_name: Option<String>,
    pub model_name: Option<String>,
    pub freshly_installed: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum DevqlTaskProgress {
    Sync(crate::host::devql::SyncProgressUpdate),
    Ingest(crate::host::devql::IngestionProgressUpdate),
    EmbeddingsBootstrap(EmbeddingsBootstrapProgress),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum DevqlTaskResult {
    Sync(Box<crate::host::devql::SyncSummary>),
    Ingest(crate::host::devql::IngestionCounters),
    EmbeddingsBootstrap(EmbeddingsBootstrapResult),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DevqlTaskRecord {
    pub task_id: String,
    pub repo_id: String,
    pub repo_name: String,
    pub repo_provider: String,
    pub repo_organisation: String,
    pub repo_identity: String,
    #[serde(alias = "config_root", default)]
    pub daemon_config_root: PathBuf,
    pub repo_root: PathBuf,
    #[serde(default)]
    pub init_session_id: Option<String>,
    pub kind: DevqlTaskKind,
    pub source: DevqlTaskSource,
    pub spec: DevqlTaskSpec,
    pub status: DevqlTaskStatus,
    pub submitted_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub updated_at_unix: u64,
    pub completed_at_unix: Option<u64>,
    pub queue_position: Option<u64>,
    pub tasks_ahead: Option<u64>,
    pub progress: DevqlTaskProgress,
    pub error: Option<String>,
    pub result: Option<DevqlTaskResult>,
}

impl DevqlTaskRecord {
    pub fn normalise_legacy_values(&mut self) {
        if self.daemon_config_root.as_os_str().is_empty() {
            self.daemon_config_root = self.repo_root.clone();
        }
    }

    pub fn sync_spec(&self) -> Option<&SyncTaskSpec> {
        match &self.spec {
            DevqlTaskSpec::Sync(spec) => Some(spec),
            DevqlTaskSpec::Ingest(_) | DevqlTaskSpec::EmbeddingsBootstrap(_) => None,
        }
    }

    pub fn ingest_spec(&self) -> Option<&IngestTaskSpec> {
        match &self.spec {
            DevqlTaskSpec::Sync(_) => None,
            DevqlTaskSpec::Ingest(spec) => Some(spec),
            DevqlTaskSpec::EmbeddingsBootstrap(_) => None,
        }
    }

    pub fn embeddings_bootstrap_spec(&self) -> Option<&EmbeddingsBootstrapTaskSpec> {
        match &self.spec {
            DevqlTaskSpec::EmbeddingsBootstrap(spec) => Some(spec),
            DevqlTaskSpec::Sync(_) | DevqlTaskSpec::Ingest(_) => None,
        }
    }

    pub fn sync_progress(&self) -> Option<&crate::host::devql::SyncProgressUpdate> {
        match &self.progress {
            DevqlTaskProgress::Sync(progress) => Some(progress),
            DevqlTaskProgress::Ingest(_) | DevqlTaskProgress::EmbeddingsBootstrap(_) => None,
        }
    }

    pub fn ingest_progress(&self) -> Option<&crate::host::devql::IngestionProgressUpdate> {
        match &self.progress {
            DevqlTaskProgress::Sync(_) | DevqlTaskProgress::EmbeddingsBootstrap(_) => None,
            DevqlTaskProgress::Ingest(progress) => Some(progress),
        }
    }

    pub fn embeddings_bootstrap_progress(&self) -> Option<&EmbeddingsBootstrapProgress> {
        match &self.progress {
            DevqlTaskProgress::EmbeddingsBootstrap(progress) => Some(progress),
            DevqlTaskProgress::Sync(_) | DevqlTaskProgress::Ingest(_) => None,
        }
    }

    pub fn sync_result(&self) -> Option<&crate::host::devql::SyncSummary> {
        match &self.result {
            Some(DevqlTaskResult::Sync(result)) => Some(result.as_ref()),
            _ => None,
        }
    }

    pub fn ingest_result(&self) -> Option<&crate::host::devql::IngestionCounters> {
        match &self.result {
            Some(DevqlTaskResult::Ingest(result)) => Some(result),
            _ => None,
        }
    }

    pub fn embeddings_bootstrap_result(&self) -> Option<&EmbeddingsBootstrapResult> {
        match &self.result {
            Some(DevqlTaskResult::EmbeddingsBootstrap(result)) => Some(result),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoTaskControlState {
    pub repo_id: String,
    pub paused: bool,
    pub paused_reason: Option<String>,
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevqlTaskKindCounts {
    pub kind: DevqlTaskKind,
    pub queued_tasks: u64,
    pub running_tasks: u64,
    pub failed_tasks: u64,
    pub completed_recent_tasks: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevqlTaskQueueState {
    pub version: u8,
    pub queued_tasks: u64,
    pub running_tasks: u64,
    pub failed_tasks: u64,
    pub completed_recent_tasks: u64,
    pub by_kind: Vec<DevqlTaskKindCounts>,
    pub last_action: Option<String>,
    pub last_updated_unix: u64,
}

impl Default for DevqlTaskQueueState {
    fn default() -> Self {
        Self {
            version: 1,
            queued_tasks: 0,
            running_tasks: 0,
            failed_tasks: 0,
            completed_recent_tasks: 0,
            by_kind: default_devql_task_kind_counts(),
            last_action: Some("initialized".to_string()),
            last_updated_unix: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DevqlTaskQueueStatus {
    pub state: DevqlTaskQueueState,
    pub persisted: bool,
    pub current_repo_tasks: Vec<DevqlTaskRecord>,
    pub current_repo_control: Option<RepoTaskControlState>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DevqlTaskControlResult {
    pub message: String,
    pub control: RepoTaskControlState,
}

fn default_devql_task_kind_counts() -> Vec<DevqlTaskKindCounts> {
    vec![
        DevqlTaskKindCounts {
            kind: DevqlTaskKind::Sync,
            queued_tasks: 0,
            running_tasks: 0,
            failed_tasks: 0,
            completed_recent_tasks: 0,
        },
        DevqlTaskKindCounts {
            kind: DevqlTaskKind::Ingest,
            queued_tasks: 0,
            running_tasks: 0,
            failed_tasks: 0,
            completed_recent_tasks: 0,
        },
    ]
}

#[derive(Debug, Clone, Serialize)]
pub struct DaemonStatusReport {
    pub runtime: Option<DaemonRuntimeState>,
    pub service: Option<DaemonServiceMetadata>,
    pub service_running: bool,
    pub health: Option<DaemonHealthSummary>,
    pub current_state_consumers: Option<CapabilityEventQueueStatus>,
    pub capability_events: Option<CapabilityEventQueueStatus>,
    pub enrichment: Option<EnrichmentQueueStatus>,
    pub devql_tasks: Option<DevqlTaskQueueStatus>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(super) struct SupervisorStartRequest {
    pub(super) config_path: PathBuf,
    pub(super) config: DashboardServerConfig,
    pub(super) telemetry: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(super) struct SupervisorStopRequest {}

#[derive(Debug, Clone)]
pub struct ResolvedDaemonConfig {
    pub config_path: PathBuf,
    pub config_root: PathBuf,
    pub relational_db_path: PathBuf,
    pub events_db_path: PathBuf,
    pub blob_store_path: PathBuf,
    pub repo_registry_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SupervisorHealthResponse {
    pub(super) status: String,
}

#[derive(Clone)]
pub(super) struct SupervisorAppState {
    pub(super) operation_lock: Arc<Mutex<()>>,
}

#[cfg(test)]
pub fn runtime_state_path(repo_root: &Path) -> PathBuf {
    if repo_root.as_os_str().is_empty() || repo_root == Path::new(".") {
        return global_daemon_dir_fallback().join(RUNTIME_STATE_FILE_NAME);
    }
    crate::utils::paths::default_runtime_state_dir(repo_root).join(RUNTIME_STATE_FILE_NAME)
}

#[cfg(not(test))]
pub fn runtime_state_path(_repo_root: &Path) -> PathBuf {
    global_daemon_dir_fallback().join(RUNTIME_STATE_FILE_NAME)
}

#[cfg(test)]
pub fn service_metadata_path(repo_root: &Path) -> PathBuf {
    if repo_root.as_os_str().is_empty() || repo_root == Path::new(".") {
        return global_daemon_dir_fallback().join(SERVICE_STATE_FILE_NAME);
    }
    crate::utils::paths::default_runtime_state_dir(repo_root).join(SERVICE_STATE_FILE_NAME)
}

#[cfg(not(test))]
pub fn service_metadata_path(_repo_root: &Path) -> PathBuf {
    global_daemon_dir_fallback().join(SERVICE_STATE_FILE_NAME)
}

pub(super) fn global_daemon_dir() -> Result<PathBuf> {
    crate::utils::platform_dirs::bitloops_state_dir().map(|dir| dir.join("daemon"))
}

pub(super) fn global_daemon_dir_fallback() -> PathBuf {
    crate::utils::platform_dirs::bitloops_state_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("bitloops").join("state"))
        .join("daemon")
}

pub(super) fn supervisor_service_metadata_path() -> Result<PathBuf> {
    Ok(global_daemon_dir()?.join(SUPERVISOR_SERVICE_STATE_FILE_NAME))
}

pub(super) fn unix_timestamp_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
