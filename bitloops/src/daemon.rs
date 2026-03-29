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
use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, resolve_blob_local_path_for_repo,
    resolve_store_backend_config_for_repo,
};
use crate::devql_transport::{SlimCliRepoScope, attach_slim_cli_scope_headers};
const RUNTIME_STATE_FILE_NAME: &str = "runtime.json";
const SERVICE_STATE_FILE_NAME: &str = "service.json";
const INTERNAL_DAEMON_COMMAND_NAME: &str = "__daemon-process";
const INTERNAL_SUPERVISOR_COMMAND_NAME: &str = "__daemon-supervisor";
const GLOBAL_SUPERVISOR_SERVICE_NAME: &str = "com.bitloops.daemon";
const GLOBAL_DAEMON_DIR: &str = ".bitloops/daemon";
const SUPERVISOR_RUNTIME_STATE_FILE_NAME: &str = "supervisor-runtime.json";
const SUPERVISOR_SERVICE_STATE_FILE_NAME: &str = "supervisor-service.json";
const READY_TIMEOUT: Duration = Duration::from_secs(20);
const STOP_TIMEOUT: Duration = Duration::from_secs(10);

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
}

#[derive(Debug, Clone, Args, Default)]
pub struct InternalDaemonSupervisorArgs {}

impl InternalDaemonProcessArgs {
    pub fn from_server_config(
        daemon_config: &ResolvedDaemonConfig,
        mode: DaemonMode,
        service_name: Option<String>,
        config: &DashboardServerConfig,
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
struct SupervisorStartRequest {
    config_path: PathBuf,
    config: DashboardServerConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SupervisorStopRequest {}

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
struct SupervisorHealthResponse {
    status: String,
}

#[derive(Clone)]
struct SupervisorAppState {
    operation_lock: Arc<Mutex<()>>,
}

pub fn runtime_state_path(repo_root: &Path) -> PathBuf {
    let _ = repo_root;
    global_daemon_dir_fallback().join(RUNTIME_STATE_FILE_NAME)
}

pub fn service_metadata_path(repo_root: &Path) -> PathBuf {
    let _ = repo_root;
    global_daemon_dir_fallback().join(SERVICE_STATE_FILE_NAME)
}

fn global_daemon_dir() -> Result<PathBuf> {
    Ok(user_home_dir()?.join(GLOBAL_DAEMON_DIR))
}

fn global_daemon_dir_fallback() -> PathBuf {
    user_home_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(GLOBAL_DAEMON_DIR)
}

pub fn resolve_daemon_config(explicit_config_path: Option<&Path>) -> Result<ResolvedDaemonConfig> {
    let config_path = match explicit_config_path {
        Some(path) => expand_user_path(path)?,
        None => env::current_dir()
            .context("resolving current directory for Bitloops daemon config")?
            .join(BITLOOPS_CONFIG_RELATIVE_PATH),
    };
    if !config_path.is_file() {
        bail!(
            "Bitloops daemon config not found at {}. Pass `--config <path>` or run the command from a directory containing `./{}`.",
            config_path.display(),
            BITLOOPS_CONFIG_RELATIVE_PATH
        );
    }

    let config_path = config_path
        .canonicalize()
        .unwrap_or_else(|_| config_path.to_path_buf());
    let config_root = derive_config_root(&config_path)?;
    let backend_config = resolve_store_backend_config_for_repo(&config_root)
        .with_context(|| format!("resolving store backends from {}", config_path.display()))?;
    let relational_db_path = backend_config
        .relational
        .resolve_sqlite_db_path_for_repo(&config_root)
        .context("resolving SQLite path for Bitloops daemon")?;
    let events_db_path = backend_config
        .events
        .resolve_duckdb_db_path_for_repo(&config_root);
    let blob_store_path =
        resolve_blob_local_path_for_repo(&config_root, backend_config.blobs.local_path.as_deref())
            .context("resolving blob store path for Bitloops daemon")?;

    Ok(ResolvedDaemonConfig {
        config_path,
        config_root,
        relational_db_path,
        events_db_path,
        blob_store_path,
        repo_registry_path: global_daemon_dir()?.join("repo-path-registry.json"),
    })
}

fn derive_config_root(config_path: &Path) -> Result<PathBuf> {
    let config_dir = config_path
        .parent()
        .context("resolving Bitloops daemon config directory")?;
    if config_dir
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value == ".bitloops")
    {
        return config_dir
            .parent()
            .map(Path::to_path_buf)
            .context("resolving Bitloops daemon config root");
    }
    Ok(config_dir.to_path_buf())
}

fn expand_user_path(path: &Path) -> Result<PathBuf> {
    let raw = path.to_string_lossy();
    if raw == "~" {
        return user_home_dir();
    }
    if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        return Ok(user_home_dir()?.join(rest));
    }
    Ok(path.to_path_buf())
}

fn supervisor_runtime_state_path() -> Result<PathBuf> {
    Ok(global_daemon_dir()?.join(SUPERVISOR_RUNTIME_STATE_FILE_NAME))
}

fn supervisor_service_metadata_path() -> Result<PathBuf> {
    Ok(global_daemon_dir()?.join(SUPERVISOR_SERVICE_STATE_FILE_NAME))
}

pub async fn run_internal_process(args: InternalDaemonProcessArgs) -> Result<()> {
    let mode: DaemonMode = args.mode.into();
    let daemon_config = resolve_daemon_config(Some(args.config_path.as_path()))?;
    run_server(
        &daemon_config,
        args.server_config(),
        mode,
        args.service_name,
        false,
        "Bitloops daemon",
        false,
    )
    .await
}

pub async fn run_internal_supervisor(_args: InternalDaemonSupervisorArgs) -> Result<()> {
    let control_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("binding Bitloops daemon supervisor control listener")?;
    let control_addr = control_listener
        .local_addr()
        .context("reading Bitloops daemon supervisor listener address")?;
    let control_url = format!("http://127.0.0.1:{}", control_addr.port());
    let runtime_path = supervisor_runtime_state_path()?;
    let fingerprint = current_binary_fingerprint()?;

    write_json(
        &runtime_path,
        &SupervisorRuntimeState {
            version: 1,
            pid: std::process::id(),
            control_url: control_url.clone(),
            binary_fingerprint: fingerprint,
            updated_at_unix: unix_timestamp_now(),
        },
    )?;

    let app = Router::new()
        .route("/health", get(supervisor_health))
        .route("/daemon/start", post(handle_supervisor_start_repo))
        .route("/daemon/stop", post(handle_supervisor_stop_repo))
        .route("/daemon/restart", post(handle_supervisor_restart_repo))
        .with_state(SupervisorAppState {
            operation_lock: Arc::new(Mutex::new(())),
        });

    let result = axum::serve(control_listener, app)
        .with_graceful_shutdown(wait_for_shutdown_signal())
        .await;

    let _ = fs::remove_file(runtime_path);
    result.context("running Bitloops daemon supervisor")
}

pub async fn start_foreground(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
    open_browser: bool,
    ready_subject: &str,
) -> Result<()> {
    ensure_can_start(daemon_config.config_root.as_path(), false)?;
    run_server(
        daemon_config,
        config,
        DaemonMode::Foreground,
        None,
        open_browser,
        ready_subject,
        true,
    )
    .await
}

pub async fn start_detached(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
) -> Result<DaemonRuntimeState> {
    ensure_can_start(daemon_config.config_root.as_path(), false)?;
    let args = InternalDaemonProcessArgs::from_server_config(
        daemon_config,
        DaemonMode::Detached,
        None,
        &config,
    );
    let mut command = build_daemon_spawn_command(&args)?;
    command
        .current_dir(&daemon_config.config_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let child = command.spawn().with_context(|| {
        format!(
            "spawning detached Bitloops daemon for {}",
            daemon_config.config_path.display()
        )
    })?;
    log::debug!("spawned detached daemon pid={}", child.id());
    wait_until_ready(READY_TIMEOUT).await
}

pub async fn start_service(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
) -> Result<DaemonRuntimeState> {
    supervisor_start_repo(daemon_config, config).await
}

pub async fn restart(config_override: Option<&ResolvedDaemonConfig>) -> Result<DaemonRuntimeState> {
    let service = read_service_metadata(Path::new("."))?;
    let runtime = read_runtime_state(Path::new("."))?;

    if service.is_some() {
        let server_config = service
            .as_ref()
            .map(|metadata| metadata.config.clone())
            .context("service metadata missing for Bitloops daemon")?;
        let daemon_config = match config_override {
            Some(daemon_config) => daemon_config.clone(),
            None => {
                let config_path = service
                    .as_ref()
                    .map(|metadata| metadata.config_path.clone())
                    .context("daemon config path missing for Bitloops service metadata")?;
                resolve_daemon_config(Some(config_path.as_path()))?
            }
        };
        return supervisor_restart_repo(&daemon_config, server_config).await;
    }

    let runtime = runtime.context(
        "Bitloops daemon is not running. Start it with `bitloops daemon start`.",
    )?;
    let config = DashboardServerConfig {
        host: Some(runtime.host.clone()),
        port: runtime.port,
        no_open: true,
        force_http: runtime.url.starts_with("http://"),
        recheck_local_dashboard_net: false,
            bundle_dir: Some(runtime.bundle_dir.clone()),
        };
    stop().await?;
    let daemon_config = match config_override {
        Some(daemon_config) => daemon_config.clone(),
        None => resolve_daemon_config(Some(runtime.config_path.as_path()))?,
    };
    match runtime.mode {
        DaemonMode::Detached => start_detached(&daemon_config, config).await,
        DaemonMode::Foreground => {
            bail!(
                "cannot restart a foreground daemon from another process; run `bitloops daemon start` again"
            )
        }
        DaemonMode::Service => {
            bail!("service-backed daemon should have been handled before local runtime restart")
        }
    }
}

pub async fn stop() -> Result<()> {
    let service = read_service_metadata(Path::new("."))?;
    let runtime = read_runtime_state(Path::new("."))?;

    if let Some(metadata) = service {
        if supervisor_available().unwrap_or(false) {
            if let Err(err) = supervisor_stop_repo().await {
                if runtime.is_some() {
                    log::debug!("supervisor stop fallback to direct runtime stop: {err:#}");
                    stop_service_managed_repo_runtime()?;
                } else {
                    return Err(err);
                }
            }
        } else if runtime.is_some() {
            stop_service_managed_repo_runtime()?;
        }
        let runtime_path = runtime_state_path(Path::new("."));
        if runtime_path.exists() {
            wait_for_runtime_cleanup(&runtime_path, STOP_TIMEOUT)?;
        }
        let _ = metadata;
        return Ok(());
    }

    let runtime = runtime.context(
        "Bitloops daemon is not running. Start it with `bitloops daemon start`.",
    )?;
    terminate_process(runtime.pid)?;
    wait_for_runtime_cleanup(&runtime_state_path(Path::new(".")), STOP_TIMEOUT)?;
    Ok(())
}

pub async fn status() -> Result<DaemonStatusReport> {
    let runtime = read_runtime_state(Path::new("."))?;
    let service = read_service_metadata(Path::new("."))?;
    let service_running = service.is_some()
        && read_supervisor_service_metadata()?
            .as_ref()
            .map(|metadata| is_supervisor_service_running(metadata).unwrap_or(false))
            .unwrap_or(false);
    let health = match runtime.as_ref() {
        Some(state) => query_health(state).await.ok(),
        None => None,
    };

    Ok(DaemonStatusReport {
        runtime,
        service,
        service_running,
        health,
    })
}

pub async fn wait_until_ready(timeout: Duration) -> Result<DaemonRuntimeState> {
    let started = Instant::now();
    loop {
        if started.elapsed() > timeout {
            bail!(
                "Bitloops daemon did not become ready within {} seconds",
                timeout.as_secs()
            );
        }

        if let Some(state) = read_runtime_state(Path::new("."))?
            && daemon_http_ready(&state).await
        {
            return Ok(state);
        }

        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

pub async fn execute_graphql<T: DeserializeOwned>(
    repo_root: &Path,
    query: &str,
    variables: Value,
) -> Result<T> {
    execute_graphql_request(repo_root, "/devql/global", None, query, variables).await
}

pub(crate) async fn execute_slim_graphql<T: DeserializeOwned>(
    repo_root: &Path,
    scope: &SlimCliRepoScope,
    query: &str,
    variables: Value,
) -> Result<T> {
    execute_graphql_request(repo_root, "/devql", Some(scope), query, variables).await
}

async fn execute_graphql_request<T: DeserializeOwned>(
    repo_root: &Path,
    endpoint_path: &str,
    scope: Option<&SlimCliRepoScope>,
    query: &str,
    variables: Value,
) -> Result<T> {
    let timings_enabled = crate::devql_timing::timings_enabled_from_env();
    let trace = timings_enabled.then(crate::devql_timing::TimingTrace::new);

    let runtime_started = Instant::now();
    let runtime = read_runtime_state(repo_root)?.context(
        "Bitloops daemon is not running for this repository. Start it with `bitloops daemon start`.",
    )?;
    if let Some(trace) = trace.as_ref() {
        trace.record(
            "client.daemon.read_runtime_state",
            runtime_started.elapsed(),
            json!({
                "url": runtime.url,
            }),
        );
    }

    let client_started = Instant::now();
    let client = daemon_http_client()?;
    if let Some(trace) = trace.as_ref() {
        trace.record(
            "client.daemon.build_http_client",
            client_started.elapsed(),
            Value::Null,
        );
    }

    let endpoint = format!("{}{}", runtime.url.trim_end_matches('/'), endpoint_path);
    let send_started = Instant::now();
    let mut request = client.post(endpoint).json(&json!({
        "query": query,
        "variables": variables,
    }));
    if let Some(scope) = scope {
        request = attach_slim_cli_scope_headers(request, scope);
    }
    if timings_enabled {
        request = request.header(
            crate::devql_timing::DEVQL_TIMINGS_HEADER,
            crate::devql_timing::timing_header_value(),
        );
    }
    let response = request
        .send()
        .await
        .context("sending DevQL request to Bitloops daemon")?;
    if let Some(trace) = trace.as_ref() {
        trace.record(
            "client.daemon.http_post",
            send_started.elapsed(),
            json!({
                "status": response.status().as_u16(),
            }),
        );
    }

    if response.status() != ReqwestStatusCode::OK {
        emit_query_timing_debug(trace.as_ref(), None);
        bail!("Bitloops daemon returned HTTP {}", response.status());
    }

    let decode_started = Instant::now();
    let payload: GraphqlEnvelope = response
        .json()
        .await
        .context("decoding DevQL response from Bitloops daemon")?;
    if let Some(trace) = trace.as_ref() {
        trace.record(
            "client.daemon.decode_response_json",
            decode_started.elapsed(),
            Value::Null,
        );
    }

    let server_timings = payload
        .extensions
        .as_ref()
        .and_then(|extensions| extensions.get(crate::devql_timing::DEVQL_TIMINGS_EXTENSION))
        .cloned();

    if let Some(errors) = payload.errors
        && let Some(error) = errors.first()
    {
        emit_query_timing_debug(trace.as_ref(), server_timings.as_ref());
        bail!("{}", error.message);
    }

    let Some(data) = payload.data else {
        emit_query_timing_debug(trace.as_ref(), server_timings.as_ref());
        bail!("Bitloops daemon returned no GraphQL data payload");
    };

    let decode_graphql_started = Instant::now();
    let decoded = serde_json::from_value(data).context("decoding GraphQL data payload for CLI");
    if let Some(trace) = trace.as_ref() {
        trace.record(
            "client.daemon.decode_graphql_data",
            decode_graphql_started.elapsed(),
            Value::Null,
        );
    }
    emit_query_timing_debug(trace.as_ref(), server_timings.as_ref());
    decoded
}

pub fn choose_dashboard_launch_mode() -> Result<Option<DaemonMode>> {
    use std::io::{self, IsTerminal, Write};

    let stdin = io::stdin();
    if !stdin.is_terminal() {
        return Ok(None);
    }

    let mut stdout = io::stdout();
    writeln!(
        stdout,
        "Bitloops daemon is not running. Start it in foreground [f], detached [d], always-on [a], or cancel [c]?"
    )?;
    write!(stdout, "> ")?;
    stdout.flush()?;

    let mut input = String::new();
    stdin
        .read_line(&mut input)
        .context("reading dashboard daemon launch choice")?;
    let choice = match input.trim().to_ascii_lowercase().as_str() {
        "f" | "foreground" => Some(DaemonMode::Foreground),
        "d" | "detached" => Some(DaemonMode::Detached),
        "a" | "always-on" | "always_on" | "service" => Some(DaemonMode::Service),
        "c" | "cancel" | "" => None,
        other => bail!("unsupported dashboard launch choice `{other}`"),
    };
    Ok(choice)
}

pub fn daemon_url() -> Result<Option<String>> {
    Ok(read_runtime_state(Path::new("."))?.map(|state| state.url))
}

async fn supervisor_health() -> Json<SupervisorHealthResponse> {
    Json(SupervisorHealthResponse {
        status: "ok".to_string(),
    })
}

async fn handle_supervisor_start_repo(
    State(state): State<SupervisorAppState>,
    Json(request): Json<SupervisorStartRequest>,
) -> Result<Json<DaemonRuntimeState>, (axum::http::StatusCode, String)> {
    let _guard = state.operation_lock.lock().await;
    let daemon_config =
        resolve_daemon_config(Some(request.config_path.as_path())).map_err(supervisor_api_error)?;
    ensure_service_managed_repo_runtime(&daemon_config, request.config)
        .await
        .map(Json)
        .map_err(supervisor_api_error)
}

async fn handle_supervisor_stop_repo(
    State(state): State<SupervisorAppState>,
    Json(_request): Json<SupervisorStopRequest>,
) -> Result<Json<SupervisorHealthResponse>, (axum::http::StatusCode, String)> {
    let _guard = state.operation_lock.lock().await;
    stop_service_managed_repo_runtime().map_err(supervisor_api_error)?;
    Ok(Json(SupervisorHealthResponse {
        status: "ok".to_string(),
    }))
}

async fn handle_supervisor_restart_repo(
    State(state): State<SupervisorAppState>,
    Json(request): Json<SupervisorStartRequest>,
) -> Result<Json<DaemonRuntimeState>, (axum::http::StatusCode, String)> {
    let _guard = state.operation_lock.lock().await;
    let daemon_config =
        resolve_daemon_config(Some(request.config_path.as_path())).map_err(supervisor_api_error)?;
    restart_service_managed_repo_runtime(&daemon_config, request.config)
        .await
        .map(Json)
        .map_err(supervisor_api_error)
}

fn supervisor_api_error(err: anyhow::Error) -> (axum::http::StatusCode, String) {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        err.to_string(),
    )
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut terminate = signal(SignalKind::terminate()).ok();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = async {
                if let Some(signal) = terminate.as_mut() {
                    signal.recv().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn run_server(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
    mode: DaemonMode,
    service_name: Option<String>,
    open_browser: bool,
    ready_subject: &str,
    print_banner: bool,
) -> Result<()> {
    let config_root = daemon_config
        .config_root
        .canonicalize()
        .unwrap_or_else(|_| daemon_config.config_root.clone());
    let runtime_path = runtime_state_path(&config_root);
    let service_metadata_path = service_metadata_path(&config_root);
    let current_fingerprint = current_binary_fingerprint()?;
    let on_ready_config = daemon_config.clone();
    let on_ready_runtime_path = runtime_path.clone();
    let on_ready_service_metadata_path = service_metadata_path.clone();
    let on_ready_service_name = service_name.clone();
    let ready_hook: DashboardReadyHook = std::sync::Arc::new(move |ready| {
        write_runtime_state(
            &on_ready_runtime_path,
            &DaemonRuntimeState {
                version: 1,
                config_path: on_ready_config.config_path.clone(),
                config_root: on_ready_config.config_root.clone(),
                pid: std::process::id(),
                mode,
                service_name: on_ready_service_name.clone(),
                url: ready.url.clone(),
                host: ready.host.clone(),
                port: ready.port,
                bundle_dir: ready.bundle_dir.clone(),
                relational_db_path: on_ready_config.relational_db_path.clone(),
                events_db_path: on_ready_config.events_db_path.clone(),
                blob_store_path: on_ready_config.blob_store_path.clone(),
                repo_registry_path: on_ready_config.repo_registry_path.clone(),
                binary_fingerprint: current_fingerprint.clone(),
                updated_at_unix: unix_timestamp_now(),
            },
        )?;

        if matches!(mode, DaemonMode::Service)
            && let Ok(Some(mut metadata)) =
                read_service_metadata_for_path(&on_ready_service_metadata_path)
        {
            metadata.last_url = Some(ready.url.clone());
            metadata.last_pid = Some(std::process::id());
            write_json(&on_ready_service_metadata_path, &metadata)?;
        }

        Ok(())
    });
    let on_shutdown_runtime_path = runtime_path.clone();
    let on_shutdown = std::sync::Arc::new(move || {
        let _ = fs::remove_file(&on_shutdown_runtime_path);
    });

    api::run_with_options(
        config,
        DashboardRuntimeOptions {
            ready_subject: ready_subject.to_string(),
            print_ready_banner: print_banner,
            open_browser,
            shutdown_message: Some("Bitloops daemon stopped.".to_string()),
            on_ready: Some(ready_hook),
            on_shutdown: Some(on_shutdown),
            config_root: Some(config_root),
            repo_registry_path: Some(daemon_config.repo_registry_path.clone()),
        },
    )
    .await
}

async fn ensure_service_managed_repo_runtime(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
) -> Result<DaemonRuntimeState> {
    let config_root = daemon_config
        .config_root
        .canonicalize()
        .unwrap_or_else(|_| daemon_config.config_root.clone());

    if let Some(runtime) = read_runtime_state(&config_root)? {
        if runtime.mode == DaemonMode::Service
            && runtime.service_name.as_deref() == Some(GLOBAL_SUPERVISOR_SERVICE_NAME)
        {
            install_or_update_repo_service_binding(daemon_config, config)?;
            if daemon_http_ready(&runtime).await {
                return Ok(runtime);
            }
            return wait_until_ready(READY_TIMEOUT).await;
        }

        bail!(
            "Bitloops daemon is already running at {}. Use `bitloops daemon restart` if you need to replace it.",
            runtime.url
        );
    }

    let binding = install_or_update_repo_service_binding(daemon_config, config.clone())?;
    let args = InternalDaemonProcessArgs::from_server_config(
        daemon_config,
        DaemonMode::Service,
        Some(binding.service_name.clone()),
        &config,
    );
    let mut command = build_daemon_spawn_command(&args)?;
    command
        .current_dir(&config_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    command.spawn().with_context(|| {
        format!(
            "spawning service-managed Bitloops daemon for {}",
            daemon_config.config_path.display()
        )
    })?;

    wait_until_ready(READY_TIMEOUT).await
}

fn stop_service_managed_repo_runtime() -> Result<()> {
    let runtime = read_runtime_state(Path::new("."))?;
    let binding = read_service_metadata(Path::new("."))?;

    match runtime {
        Some(state)
            if state.mode == DaemonMode::Service
                && state.service_name.as_deref() == Some(GLOBAL_SUPERVISOR_SERVICE_NAME) =>
        {
            terminate_process(state.pid)?;
            wait_for_runtime_cleanup(&runtime_state_path(Path::new(".")), STOP_TIMEOUT)?;
            Ok(())
        }
        Some(state) => bail!(
            "Bitloops daemon is running in {} mode. Use `bitloops daemon stop` from that mode instead.",
            state.mode
        ),
        None if binding.is_some() => Ok(()),
        None => bail!("Bitloops daemon is not running. Start it with `bitloops daemon start`."),
    }
}

async fn restart_service_managed_repo_runtime(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
) -> Result<DaemonRuntimeState> {
    if read_runtime_state(daemon_config.config_root.as_path())?.is_some() {
        stop_service_managed_repo_runtime()?;
    }
    ensure_service_managed_repo_runtime(daemon_config, config).await
}

fn install_or_update_repo_service_binding(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
) -> Result<DaemonServiceMetadata> {
    let existing = read_service_metadata(daemon_config.config_root.as_path())?;
    if let Some(existing) = existing.as_ref()
        && existing.service_name != GLOBAL_SUPERVISOR_SERVICE_NAME
    {
        cleanup_legacy_repo_service(existing)?;
    }
    let metadata = DaemonServiceMetadata {
        version: 1,
        config_path: daemon_config.config_path.clone(),
        config_root: daemon_config.config_root.clone(),
        manager: current_service_manager(),
        service_name: GLOBAL_SUPERVISOR_SERVICE_NAME.to_string(),
        service_file: None,
        config,
        last_url: existing.as_ref().and_then(|value| value.last_url.clone()),
        last_pid: existing.as_ref().and_then(|value| value.last_pid),
    };
    write_service_metadata(
        &service_metadata_path(daemon_config.config_root.as_path()),
        &metadata,
    )?;
    Ok(metadata)
}

fn cleanup_legacy_repo_service(metadata: &DaemonServiceMetadata) -> Result<()> {
    match metadata.manager {
        ServiceManagerKind::Launchd => {
            let domain_target = launchd_domain_target()?;
            let mut command = Command::new("launchctl");
            command.arg("bootout").arg(&domain_target);
            if let Some(path) = metadata.service_file.as_ref() {
                command.arg(path);
            } else {
                command.arg(format!("{domain_target}/{}", metadata.service_name));
            }
            let _ = command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            if let Some(path) = metadata.service_file.as_ref() {
                let _ = fs::remove_file(path);
            }
        }
        ServiceManagerKind::SystemdUser => {
            let _ = Command::new("systemctl")
                .arg("--user")
                .arg("stop")
                .arg(&metadata.service_name)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = Command::new("systemctl")
                .arg("--user")
                .arg("disable")
                .arg(&metadata.service_name)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            if let Some(path) = metadata.service_file.as_ref() {
                let _ = fs::remove_file(path);
            }
            let _ = Command::new("systemctl")
                .arg("--user")
                .arg("daemon-reload")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        ServiceManagerKind::WindowsTask => {
            let _ = Command::new("schtasks")
                .arg("/Delete")
                .arg("/TN")
                .arg(&metadata.service_name)
                .arg("/F")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
    Ok(())
}

fn ensure_can_start(repo_root: &Path, allow_stopped_service: bool) -> Result<()> {
    if let Some(runtime) = read_runtime_state(repo_root)? {
        let current = current_binary_fingerprint().unwrap_or_default();
        if !runtime.binary_fingerprint.is_empty()
            && !current.is_empty()
            && runtime.binary_fingerprint != current
            && !matches!(runtime.mode, DaemonMode::Service)
        {
            terminate_process(runtime.pid)?;
            let _ = fs::remove_file(runtime_state_path(repo_root));
        } else {
            bail!(
                "Bitloops daemon is already running for this repository at {}. Use `bitloops daemon restart` if you need to replace it.",
                runtime.url
            );
        }
    }

    if !allow_stopped_service
        && let Some(metadata) = read_service_metadata(repo_root)?
        && metadata.service_name == GLOBAL_SUPERVISOR_SERVICE_NAME
        && supervisor_available().unwrap_or(false)
    {
        bail!(
            "Bitloops daemon is already running as an always-on service ({}) for this repository.",
            metadata.service_name
        );
    }

    Ok(())
}

fn read_runtime_state(repo_root: &Path) -> Result<Option<DaemonRuntimeState>> {
    let path = runtime_state_path(repo_root);
    let state = read_runtime_state_for_path(&path)?;
    if let Some(state) = state
        && process_is_running(state.pid)?
    {
        return Ok(Some(state));
    }
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    Ok(None)
}

fn read_runtime_state_for_path(path: &Path) -> Result<Option<DaemonRuntimeState>> {
    read_json(path)
}

fn read_service_metadata(repo_root: &Path) -> Result<Option<DaemonServiceMetadata>> {
    read_service_metadata_for_path(&service_metadata_path(repo_root))
}

fn read_service_metadata_for_path(path: &Path) -> Result<Option<DaemonServiceMetadata>> {
    read_json(path)
}

fn read_supervisor_service_metadata() -> Result<Option<SupervisorServiceMetadata>> {
    read_json(&supervisor_service_metadata_path()?)
}

fn read_supervisor_runtime_state() -> Result<Option<SupervisorRuntimeState>> {
    let path = supervisor_runtime_state_path()?;
    let state = read_json::<SupervisorRuntimeState>(&path)?;
    if let Some(state) = state
        && process_is_running(state.pid)?
    {
        return Ok(Some(state));
    }
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    Ok(None)
}

fn write_runtime_state(path: &Path, state: &DaemonRuntimeState) -> Result<()> {
    write_json(path, state)
}

fn write_service_metadata(path: &Path, state: &DaemonServiceMetadata) -> Result<()> {
    write_json(path, state)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let data = match fs::read(path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };
    let value =
        serde_json::from_slice(&data).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(value))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("resolving daemon state parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("creating daemon state directory {}", parent.display()))?;
    let mut bytes = serde_json::to_vec_pretty(value)
        .with_context(|| format!("serialising {}", path.display()))?;
    bytes.push(b'\n');
    fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

fn current_binary_fingerprint() -> Result<String> {
    let current_exe = env::current_exe().context("resolving Bitloops executable path")?;
    let bytes = fs::read(&current_exe)
        .with_context(|| format!("reading Bitloops executable {}", current_exe.display()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn build_daemon_spawn_command(args: &InternalDaemonProcessArgs) -> Result<Command> {
    #[cfg(unix)]
    use std::os::unix::process::CommandExt;

    let executable = env::current_exe().context("resolving Bitloops executable for daemon")?;
    let mut command = Command::new(executable);
    command.args(args.argv());
    #[cfg(unix)]
    {
        command.process_group(0);
    }
    Ok(command)
}

fn process_is_running(pid: u32) -> Result<bool> {
    #[cfg(windows)]
    {
        Ok(Command::new("cmd")
            .args([
                "/C",
                &format!("tasklist /FI \"PID eq {pid}\" | findstr {pid}"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false))
    }

    #[cfg(not(windows))]
    {
        Ok(Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false))
    }
}

fn terminate_process(pid: u32) -> Result<()> {
    #[cfg(windows)]
    {
        let status = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("running `taskkill` for Bitloops daemon")?;
        if !status.success() {
            bail!("failed to stop Bitloops daemon process {pid}");
        }
    }

    #[cfg(not(windows))]
    {
        let status = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("running `kill -TERM` for Bitloops daemon")?;
        if !status.success() {
            bail!("failed to stop Bitloops daemon process {pid}");
        }
    }

    Ok(())
}

fn wait_for_runtime_cleanup(runtime_path: &Path, timeout: Duration) -> Result<()> {
    let started = Instant::now();
    while runtime_path.exists() && started.elapsed() <= timeout {
        std::thread::sleep(Duration::from_millis(100));
    }
    if runtime_path.exists() {
        bail!(
            "Bitloops daemon did not shut down within {} seconds",
            timeout.as_secs()
        );
    }
    Ok(())
}

async fn daemon_http_ready(state: &DaemonRuntimeState) -> bool {
    let client = match daemon_http_client() {
        Ok(client) => client,
        Err(_) => return false,
    };
    let url = format!("{}/devql/sdl", state.url.trim_end_matches('/'));
    client
        .get(url)
        .send()
        .await
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}

fn daemon_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .context("building local Bitloops daemon HTTP client")
}

async fn query_health(state: &DaemonRuntimeState) -> Result<DaemonHealthSummary> {
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct HealthEnvelope {
        health: HealthPayload,
    }

    #[derive(Debug, Deserialize)]
    struct HealthPayload {
        relational: Option<HealthBackend>,
        events: Option<HealthBackend>,
        blob: Option<HealthBackend>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct HealthBackend {
        backend: Option<String>,
        connected: Option<bool>,
    }

    let payload: HealthEnvelope = execute_graphql(
        &state.config_root,
        r#"{ health { relational { backend connected } events { backend connected } blob { backend connected } } }"#,
        json!({}),
    )
    .await?;

    Ok(DaemonHealthSummary {
        relational_backend: payload
            .health
            .relational
            .as_ref()
            .and_then(|value| value.backend.clone()),
        relational_connected: payload.health.relational.and_then(|value| value.connected),
        events_backend: payload
            .health
            .events
            .as_ref()
            .and_then(|value| value.backend.clone()),
        events_connected: payload.health.events.and_then(|value| value.connected),
        blob_backend: payload
            .health
            .blob
            .as_ref()
            .and_then(|value| value.backend.clone()),
        blob_connected: payload.health.blob.and_then(|value| value.connected),
    })
}

async fn ensure_supervisor_available() -> Result<SupervisorRuntimeState> {
    let metadata = install_or_update_supervisor_service()?;
    let current = current_binary_fingerprint().unwrap_or_default();
    if let Some(runtime) = read_supervisor_runtime_state()?
        && supervisor_http_ready(&runtime).await
        && (runtime.binary_fingerprint == current || current.is_empty())
    {
        return Ok(runtime);
    }

    start_configured_supervisor_service(&metadata)?;
    wait_until_supervisor_ready(READY_TIMEOUT).await
}

fn supervisor_available() -> Result<bool> {
    Ok(read_supervisor_runtime_state()?.is_some())
}

async fn wait_until_supervisor_ready(timeout: Duration) -> Result<SupervisorRuntimeState> {
    let started = Instant::now();
    loop {
        if started.elapsed() > timeout {
            bail!(
                "Bitloops daemon supervisor did not become ready within {} seconds",
                timeout.as_secs()
            );
        }

        if let Some(runtime) = read_supervisor_runtime_state()?
            && supervisor_http_ready(&runtime).await
        {
            return Ok(runtime);
        }

        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

async fn supervisor_http_ready(runtime: &SupervisorRuntimeState) -> bool {
    reqwest::Client::new()
        .get(format!(
            "{}/health",
            runtime.control_url.trim_end_matches('/')
        ))
        .send()
        .await
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}

async fn supervisor_start_repo(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
) -> Result<DaemonRuntimeState> {
    let runtime = ensure_supervisor_available().await?;
    let response = reqwest::Client::new()
        .post(format!(
            "{}/daemon/start",
            runtime.control_url.trim_end_matches('/')
        ))
        .json(&SupervisorStartRequest {
            config_path: daemon_config.config_path.clone(),
            config,
        })
        .send()
        .await
        .context("sending start request to Bitloops daemon supervisor")?;
    decode_supervisor_response(response).await
}

async fn supervisor_stop_repo() -> Result<()> {
    let runtime = read_supervisor_runtime_state()?
        .context("Bitloops daemon supervisor is not running")?;
    let response = reqwest::Client::new()
        .post(format!(
            "{}/daemon/stop",
            runtime.control_url.trim_end_matches('/')
        ))
        .json(&SupervisorStopRequest {})
        .send()
        .await
        .context("sending stop request to Bitloops daemon supervisor")?;
    decode_supervisor_response::<SupervisorHealthResponse>(response)
        .await
        .map(|_| ())
}

async fn supervisor_restart_repo(
    daemon_config: &ResolvedDaemonConfig,
    config: DashboardServerConfig,
) -> Result<DaemonRuntimeState> {
    let runtime = ensure_supervisor_available().await?;
    let response = reqwest::Client::new()
        .post(format!(
            "{}/daemon/restart",
            runtime.control_url.trim_end_matches('/')
        ))
        .json(&SupervisorStartRequest {
            config_path: daemon_config.config_path.clone(),
            config,
        })
        .send()
        .await
        .context("sending restart request to Bitloops daemon supervisor")?;
    decode_supervisor_response(response).await
}

async fn decode_supervisor_response<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!(
            "Bitloops daemon supervisor returned HTTP {}{}",
            status,
            if body.trim().is_empty() {
                "".to_string()
            } else {
                format!(": {}", body.trim())
            }
        );
    }

    response
        .json::<T>()
        .await
        .context("decoding Bitloops daemon supervisor response")
}

fn install_or_update_supervisor_service() -> Result<SupervisorServiceMetadata> {
    let manager = current_service_manager();
    let service_name = GLOBAL_SUPERVISOR_SERVICE_NAME.to_string();
    let executable =
        env::current_exe().context("resolving Bitloops executable for supervisor service")?;
    let service_metadata_path = supervisor_service_metadata_path()?;
    let argv = vec![OsString::from(INTERNAL_SUPERVISOR_COMMAND_NAME)];
    let working_directory = user_home_dir()?;

    let metadata = match manager {
        ServiceManagerKind::Launchd => {
            let path = launch_agent_plist_path(&service_name)?;
            let plist = render_launchd_plist(&service_name, &working_directory, &executable, &argv);
            write_text_file(&path, &plist)?;
            SupervisorServiceMetadata {
                version: 1,
                manager,
                service_name,
                service_file: Some(path),
            }
        }
        ServiceManagerKind::SystemdUser => {
            let path = systemd_user_unit_path(&service_name)?;
            let unit = render_systemd_unit(&service_name, &working_directory, &executable, &argv);
            write_text_file(&path, &unit)?;
            let mut command = Command::new("systemctl");
            command.arg("--user").arg("daemon-reload");
            run_status_command(
                command,
                "reloading systemd user units for Bitloops daemon supervisor",
            )?;
            SupervisorServiceMetadata {
                version: 1,
                manager,
                service_name,
                service_file: Some(path),
            }
        }
        ServiceManagerKind::WindowsTask => SupervisorServiceMetadata {
            version: 1,
            manager,
            service_name,
            service_file: None,
        },
    };

    write_json(&service_metadata_path, &metadata)?;
    Ok(metadata)
}

fn start_configured_supervisor_service(metadata: &SupervisorServiceMetadata) -> Result<()> {
    match metadata.manager {
        ServiceManagerKind::Launchd => {
            let path = metadata
                .service_file
                .as_ref()
                .context("missing launchd plist path for Bitloops daemon supervisor")?;
            let domain_target = launchd_domain_target()?;
            let _ = Command::new("launchctl")
                .arg("bootout")
                .arg(&domain_target)
                .arg(path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let mut bootstrap = Command::new("launchctl");
            bootstrap.arg("bootstrap").arg(&domain_target).arg(path);
            run_status_command(
                bootstrap,
                "bootstrapping Bitloops daemon supervisor launch agent",
            )?;
            let mut kickstart = Command::new("launchctl");
            kickstart
                .arg("kickstart")
                .arg("-k")
                .arg(format!("{domain_target}/{}", metadata.service_name));
            run_status_command(
                kickstart,
                "starting Bitloops daemon supervisor launch agent",
            )?;
        }
        ServiceManagerKind::SystemdUser => {
            let mut enable = Command::new("systemctl");
            enable
                .arg("--user")
                .arg("enable")
                .arg(&metadata.service_name);
            run_status_command(enable, "enabling Bitloops daemon supervisor user service")?;
            let mut restart = Command::new("systemctl");
            restart
                .arg("--user")
                .arg("restart")
                .arg(&metadata.service_name);
            run_status_command(restart, "starting Bitloops daemon supervisor user service")?;
        }
        ServiceManagerKind::WindowsTask => {
            let executable = env::current_exe()
                .context("resolving Bitloops executable for Windows supervisor task")?;
            let task_command = render_windows_task_command(
                &executable,
                &[OsString::from(INTERNAL_SUPERVISOR_COMMAND_NAME)],
            );
            let mut create = Command::new("schtasks");
            create
                .arg("/Create")
                .arg("/TN")
                .arg(&metadata.service_name)
                .arg("/TR")
                .arg(task_command)
                .arg("/SC")
                .arg("ONLOGON")
                .arg("/F");
            run_status_command(
                create,
                "creating Windows scheduled task for Bitloops daemon supervisor",
            )?;
            let mut run = Command::new("schtasks");
            run.arg("/Run").arg("/TN").arg(&metadata.service_name);
            run_status_command(
                run,
                "starting Windows scheduled task for Bitloops daemon supervisor",
            )?;
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn stop_configured_supervisor_service(metadata: &SupervisorServiceMetadata) -> Result<()> {
    match metadata.manager {
        ServiceManagerKind::Launchd => {
            let domain_target = launchd_domain_target()?;
            let mut command = Command::new("launchctl");
            command.arg("bootout").arg(&domain_target);
            if let Some(path) = metadata.service_file.as_ref() {
                command.arg(path);
            } else {
                command.arg(format!("{domain_target}/{}", metadata.service_name));
            }
            run_status_command(command, "stopping Bitloops daemon supervisor launch agent")?;
        }
        ServiceManagerKind::SystemdUser => {
            let mut command = Command::new("systemctl");
            command
                .arg("--user")
                .arg("stop")
                .arg(&metadata.service_name);
            run_status_command(command, "stopping Bitloops daemon supervisor user service")?;
        }
        ServiceManagerKind::WindowsTask => {
            let mut command = Command::new("schtasks");
            command.arg("/End").arg("/TN").arg(&metadata.service_name);
            run_status_command(
                command,
                "stopping Windows scheduled task for Bitloops daemon supervisor",
            )?;
        }
    }
    Ok(())
}

fn is_supervisor_service_running(metadata: &SupervisorServiceMetadata) -> Result<bool> {
    match metadata.manager {
        ServiceManagerKind::Launchd => {
            let domain_target = launchd_domain_target()?;
            let status = Command::new("launchctl")
                .arg("print")
                .arg(format!("{domain_target}/{}", metadata.service_name))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .context("querying launchd Bitloops daemon supervisor status")?;
            Ok(status.success())
        }
        ServiceManagerKind::SystemdUser => {
            let status = Command::new("systemctl")
                .arg("--user")
                .arg("is-active")
                .arg("--quiet")
                .arg(&metadata.service_name)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .context("querying systemd user Bitloops daemon supervisor status")?;
            Ok(status.success())
        }
        ServiceManagerKind::WindowsTask => {
            let output = Command::new("schtasks")
                .arg("/Query")
                .arg("/TN")
                .arg(&metadata.service_name)
                .output()
                .context("querying Windows scheduled task Bitloops daemon supervisor status")?;
            Ok(output.status.success())
        }
    }
}

fn current_service_manager() -> ServiceManagerKind {
    #[cfg(target_os = "macos")]
    {
        return ServiceManagerKind::Launchd;
    }
    #[cfg(target_os = "linux")]
    {
        return ServiceManagerKind::SystemdUser;
    }
    #[cfg(target_os = "windows")]
    {
        return ServiceManagerKind::WindowsTask;
    }
    #[allow(unreachable_code)]
    ServiceManagerKind::Launchd
}

fn launchd_domain_target() -> Result<String> {
    let uid = current_uid().context("resolving current uid for launchd user domain")?;
    Ok(format!("gui/{uid}"))
}

fn current_uid() -> Result<String> {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .context("running `id -u` for Bitloops daemon")?;
    if !output.status.success() {
        bail!("failed to resolve current uid for Bitloops daemon");
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn user_home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .context("resolving user home directory for Bitloops daemon service files")
}

fn launch_agent_plist_path(service_name: &str) -> Result<PathBuf> {
    Ok(user_home_dir()?
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{service_name}.plist")))
}

fn systemd_user_unit_path(service_name: &str) -> Result<PathBuf> {
    Ok(user_home_dir()?
        .join(".config")
        .join("systemd")
        .join("user")
        .join(format!("{service_name}.service")))
}

fn render_launchd_plist(
    service_name: &str,
    repo_root: &Path,
    executable: &Path,
    argv: &[OsString],
) -> String {
    let mut rendered = String::new();
    rendered.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    rendered.push_str("<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n");
    rendered.push_str("<plist version=\"1.0\">\n<dict>\n");
    rendered.push_str("  <key>Label</key>\n");
    rendered.push_str(&format!(
        "  <string>{}</string>\n",
        xml_escape(service_name)
    ));
    rendered.push_str("  <key>ProgramArguments</key>\n  <array>\n");
    rendered.push_str(&format!(
        "    <string>{}</string>\n",
        xml_escape(&executable.to_string_lossy())
    ));
    for arg in argv {
        rendered.push_str(&format!(
            "    <string>{}</string>\n",
            xml_escape(&arg.to_string_lossy())
        ));
    }
    rendered.push_str("  </array>\n");
    rendered.push_str("  <key>WorkingDirectory</key>\n");
    rendered.push_str(&format!(
        "  <string>{}</string>\n",
        xml_escape(&repo_root.to_string_lossy())
    ));
    rendered.push_str("  <key>RunAtLoad</key>\n  <true/>\n");
    rendered.push_str("  <key>KeepAlive</key>\n  <true/>\n");
    rendered.push_str("</dict>\n</plist>\n");
    rendered
}

fn render_systemd_unit(
    service_name: &str,
    repo_root: &Path,
    executable: &Path,
    argv: &[OsString],
) -> String {
    let exec_start = std::iter::once(executable.as_os_str().to_os_string())
        .chain(argv.iter().cloned())
        .map(|arg| systemd_escape_arg(&arg.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "[Unit]\nDescription=Bitloops daemon ({service_name})\n\n[Service]\nType=simple\nWorkingDirectory={}\nExecStart={exec_start}\nRestart=always\nRestartSec=2\n\n[Install]\nWantedBy=default.target\n",
        repo_root.display()
    )
}

fn render_windows_task_command(executable: &Path, argv: &[OsString]) -> String {
    std::iter::once(executable.as_os_str().to_os_string())
        .chain(argv.iter().cloned())
        .map(|arg| windows_escape_arg(&arg.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn systemd_escape_arg(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn windows_escape_arg(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\\\""))
}

fn write_text_file(path: &Path, content: &str) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("resolving service file parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("creating service file directory {}", parent.display()))?;
    fs::write(path, content).with_context(|| format!("writing {}", path.display()))
}

fn run_status_command(mut command: Command, action: &str) -> Result<()> {
    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| action.to_string())?;
    if !status.success() {
        bail!("{action} failed");
    }
    Ok(())
}

fn unix_timestamp_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Deserialize)]
struct GraphqlEnvelope {
    data: Option<Value>,
    extensions: Option<serde_json::Map<String, Value>>,
    errors: Option<Vec<GraphqlError>>,
}

#[derive(Debug, Deserialize)]
struct GraphqlError {
    message: String,
}

fn emit_query_timing_debug(
    trace: Option<&crate::devql_timing::TimingTrace>,
    server_timings: Option<&Value>,
) {
    if let Some(server_timings) = server_timings {
        crate::devql_timing::print_summary("server", server_timings);
    }
    if let Some(trace) = trace {
        crate::devql_timing::print_summary("client", &trace.summary_value());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::process_state::enter_process_state;
    use serde_json::json;
    use tempfile::TempDir;

    fn write_daemon_test_config(config_root: &Path) -> PathBuf {
        let config_path = config_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
        let parent = config_path.parent().expect("config parent");
        fs::create_dir_all(parent).expect("create config parent");
        fs::write(
            &config_path,
            serde_json::to_vec_pretty(&json!({
                "version": "1.0",
                "scope": "project",
                "settings": {
                    "stores": {
                        "relational": {
                            "sqlite_path": ".bitloops/stores/daemon.sqlite"
                        },
                        "events": {
                            "duckdb_path": ".bitloops/stores/daemon.duckdb"
                        },
                        "blob": {
                            "local_path": ".bitloops/blob-store"
                        }
                    }
                }
            }))
            .expect("serialise test config"),
        )
        .expect("write test config");
        config_path
    }

    #[test]
    fn supervisor_service_name_is_global_and_stable() {
        assert_eq!(GLOBAL_SUPERVISOR_SERVICE_NAME, "com.bitloops.daemon");
    }

    #[test]
    fn launchd_plist_includes_hidden_supervisor_command() {
        let rendered = render_launchd_plist(
            GLOBAL_SUPERVISOR_SERVICE_NAME,
            Path::new("/Users/test"),
            Path::new("/usr/local/bin/bitloops"),
            &[OsString::from(INTERNAL_SUPERVISOR_COMMAND_NAME)],
        );
        assert!(rendered.contains(INTERNAL_SUPERVISOR_COMMAND_NAME));
        assert!(rendered.contains(GLOBAL_SUPERVISOR_SERVICE_NAME));
    }

    #[test]
    fn systemd_unit_includes_hidden_supervisor_command() {
        let rendered = render_systemd_unit(
            GLOBAL_SUPERVISOR_SERVICE_NAME,
            Path::new("/Users/test"),
            Path::new("/usr/local/bin/bitloops"),
            &[OsString::from(INTERNAL_SUPERVISOR_COMMAND_NAME)],
        );
        assert!(rendered.contains(INTERNAL_SUPERVISOR_COMMAND_NAME));
        assert!(rendered.contains("WorkingDirectory=/Users/test"));
    }

    #[test]
    fn read_runtime_state_drops_stale_file() {
        let dir = TempDir::new().expect("temp dir");
        let repo_root = dir.path();
        let runtime_path = runtime_state_path(repo_root);
        write_runtime_state(
            &runtime_path,
            &DaemonRuntimeState {
                version: 1,
                config_path: repo_root.join(".bitloops").join("config.json"),
                config_root: repo_root.to_path_buf(),
                pid: 999_999,
                mode: DaemonMode::Detached,
                service_name: None,
                url: "http://127.0.0.1:5667".to_string(),
                host: "127.0.0.1".to_string(),
                port: 5667,
                bundle_dir: repo_root.join("bundle"),
                relational_db_path: repo_root.join("relational.db"),
                events_db_path: repo_root.join("events.duckdb"),
                blob_store_path: repo_root.join("blob"),
                repo_registry_path: repo_root.join("repo-path-registry.json"),
                binary_fingerprint: "test".to_string(),
                updated_at_unix: 0,
            },
        )
        .expect("write runtime state");

        let state = read_runtime_state(repo_root).expect("read runtime state");
        assert!(state.is_none());
        assert!(
            !runtime_path.exists(),
            "stale runtime state file should be cleaned up"
        );
    }

    #[test]
    fn resolve_daemon_config_uses_explicit_config_path_independent_of_cwd() {
        let config_root = TempDir::new().expect("temp dir");
        let other_cwd = TempDir::new().expect("temp dir");
        let config_path = write_daemon_test_config(config_root.path());
        let _guard = enter_process_state(Some(other_cwd.path()), &[]);

        let resolved =
            resolve_daemon_config(Some(config_path.as_path())).expect("resolve daemon config");
        let canonical_root = config_root
            .path()
            .canonicalize()
            .unwrap_or_else(|_| config_root.path().to_path_buf());

        assert_eq!(resolved.config_root, canonical_root);
        assert_eq!(
            resolved.relational_db_path,
            canonical_root.join(".bitloops/stores/daemon.sqlite")
        );
        assert_eq!(
            resolved.events_db_path,
            canonical_root.join(".bitloops/stores/daemon.duckdb")
        );
        assert_eq!(
            resolved.blob_store_path,
            canonical_root.join(".bitloops/blob-store")
        );
        assert_eq!(
            resolved.repo_registry_path,
            global_daemon_dir_fallback().join("repo-path-registry.json")
        );
    }

    #[test]
    fn resolve_daemon_config_uses_local_dot_bitloops_config_by_default() {
        let config_root = TempDir::new().expect("temp dir");
        let config_path = write_daemon_test_config(config_root.path());
        let _guard = enter_process_state(Some(config_root.path()), &[]);

        let resolved = resolve_daemon_config(None).expect("resolve daemon config");
        let canonical_root = config_root
            .path()
            .canonicalize()
            .unwrap_or_else(|_| config_root.path().to_path_buf());

        assert_eq!(
            resolved.config_path,
            config_path
                .canonicalize()
                .unwrap_or_else(|_| config_path.clone())
        );
        assert_eq!(resolved.config_root, canonical_root);
        assert_eq!(
            resolved.relational_db_path,
            canonical_root.join(".bitloops/stores/daemon.sqlite")
        );
    }
}
