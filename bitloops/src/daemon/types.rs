use super::*;

pub(super) const RUNTIME_STATE_FILE_NAME: &str = "runtime.json";
pub(super) const SERVICE_STATE_FILE_NAME: &str = "service.json";
pub(super) const INTERNAL_DAEMON_COMMAND_NAME: &str = "__daemon-process";
pub(super) const INTERNAL_SUPERVISOR_COMMAND_NAME: &str = "__daemon-supervisor";
pub(super) const GLOBAL_SUPERVISOR_SERVICE_NAME: &str = "com.bitloops.daemon";
pub(super) const SUPERVISOR_RUNTIME_STATE_FILE_NAME: &str = "supervisor-runtime.json";
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

#[derive(Debug, Clone)]
pub struct DaemonStatusReport {
    pub runtime: Option<DaemonRuntimeState>,
    pub service: Option<DaemonServiceMetadata>,
    pub service_running: bool,
    pub health: Option<DaemonHealthSummary>,
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

pub(super) fn supervisor_runtime_state_path() -> Result<PathBuf> {
    Ok(global_daemon_dir()?.join(SUPERVISOR_RUNTIME_STATE_FILE_NAME))
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
