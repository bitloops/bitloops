use super::*;

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
pub(super) const STOP_TIMEOUT: Duration = Duration::from_secs(10);

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

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentQueueState {
    pub version: u8,
    pub mode: EnrichmentQueueMode,
    pub pending_jobs: u64,
    pub pending_semantic_jobs: u64,
    pub pending_embedding_jobs: u64,
    pub pending_clone_edges_rebuild_jobs: u64,
    pub running_jobs: u64,
    pub running_semantic_jobs: u64,
    pub running_embedding_jobs: u64,
    pub running_clone_edges_rebuild_jobs: u64,
    pub failed_jobs: u64,
    pub failed_semantic_jobs: u64,
    pub failed_embedding_jobs: u64,
    pub failed_clone_edges_rebuild_jobs: u64,
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
            pending_jobs: 0,
            pending_semantic_jobs: 0,
            pending_embedding_jobs: 0,
            pending_clone_edges_rebuild_jobs: 0,
            running_jobs: 0,
            running_semantic_jobs: 0,
            running_embedding_jobs: 0,
            running_clone_edges_rebuild_jobs: 0,
            failed_jobs: 0,
            failed_semantic_jobs: 0,
            failed_embedding_jobs: 0,
            failed_clone_edges_rebuild_jobs: 0,
            retried_failed_jobs: 0,
            last_action: Some("initialized".to_string()),
            last_updated_unix: 0,
            paused_reason: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EnrichmentQueueStatus {
    pub state: EnrichmentQueueState,
    pub persisted: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncTaskSource {
    Init,
    ManualCli,
    Watcher,
    PostCommit,
    PostMerge,
    PostCheckout,
}

impl fmt::Display for SyncTaskSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Init => write!(f, "init"),
            Self::ManualCli => write!(f, "manual_cli"),
            Self::Watcher => write!(f, "watcher"),
            Self::PostCommit => write!(f, "post_commit"),
            Self::PostMerge => write!(f, "post_merge"),
            Self::PostCheckout => write!(f, "post_checkout"),
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
pub enum SyncTaskStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl fmt::Display for SyncTaskStatus {
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
pub struct SyncTaskRecord {
    pub task_id: String,
    pub repo_id: String,
    pub repo_name: String,
    pub repo_provider: String,
    pub repo_organisation: String,
    pub repo_identity: String,
    pub daemon_config_root: PathBuf,
    pub repo_root: PathBuf,
    pub source: SyncTaskSource,
    pub mode: SyncTaskMode,
    pub status: SyncTaskStatus,
    pub submitted_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub updated_at_unix: u64,
    pub completed_at_unix: Option<u64>,
    pub queue_position: Option<u64>,
    pub tasks_ahead: Option<u64>,
    pub progress: crate::host::devql::SyncProgressUpdate,
    pub error: Option<String>,
    pub summary: Option<crate::host::devql::SyncSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncQueueState {
    pub version: u8,
    pub pending_tasks: u64,
    pub running_tasks: u64,
    pub failed_tasks: u64,
    pub completed_recent_tasks: u64,
    pub last_action: Option<String>,
    pub last_updated_unix: u64,
}

impl Default for SyncQueueState {
    fn default() -> Self {
        Self {
            version: 1,
            pending_tasks: 0,
            running_tasks: 0,
            failed_tasks: 0,
            completed_recent_tasks: 0,
            last_action: Some("initialized".to_string()),
            last_updated_unix: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SyncQueueStatus {
    pub state: SyncQueueState,
    pub persisted: bool,
    pub current_repo_task: Option<SyncTaskRecord>,
}

#[derive(Debug, Clone)]
pub struct DaemonStatusReport {
    pub runtime: Option<DaemonRuntimeState>,
    pub service: Option<DaemonServiceMetadata>,
    pub service_running: bool,
    pub health: Option<DaemonHealthSummary>,
    pub enrichment: Option<EnrichmentQueueStatus>,
    pub sync: Option<SyncQueueStatus>,
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

pub fn runtime_state_path(repo_root: &Path) -> PathBuf {
    let _ = repo_root;
    global_daemon_dir_fallback().join(RUNTIME_STATE_FILE_NAME)
}

pub fn service_metadata_path(repo_root: &Path) -> PathBuf {
    let _ = repo_root;
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
